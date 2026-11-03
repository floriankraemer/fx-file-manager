//! Disk-touching file operations. A `Job` runs on a worker thread and yields an `Outcome`; `Explorer` schedules jobs and applies outcomes.

use fx_core::EntryKind;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Error,
}

/// A one-line message for the UI, e.g. "Moved 3 items to trash" or "Could not delete foo: Permission denied".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
}

impl Notice {
    pub(crate) fn info(text: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Info,
            text: text.into(),
        }
    }

    pub(crate) fn error(text: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Error,
            text: text.into(),
        }
    }
}

/// Why a new entry name is rejected. Existence is checked by the job, not here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    Separator,
    Dot,
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "The name must not be empty",
            Self::Separator => "The name must not contain a path separator",
            Self::Dot => "\".\" and \"..\" are not valid names",
        })
    }
}

/// Validates a single path component supplied by the user.
pub fn validate_name(name: &str) -> Result<(), NameError> {
    if name.trim().is_empty() {
        Err(NameError::Empty)
    } else if name == "." || name == ".." {
        Err(NameError::Dot)
    } else if name
        .chars()
        .any(|c| c == '/' || (cfg!(windows) && c == '\\'))
    {
        Err(NameError::Separator)
    } else {
        Ok(())
    }
}

/// `base`, or `base (2)`, `base (3)`, ... — the first name for which `is_free` holds.
pub(crate) fn unique_name(base: &str, is_free: impl Fn(&str) -> bool) -> String {
    if is_free(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base} ({n})"))
        .find(|name| is_free(name))
        .expect("unbounded range")
}

/// Work for a worker thread. Paths are captured when the request is made, never row indices.
pub(crate) enum Job {
    Delete {
        paths: Vec<PathBuf>,
        permanent: bool,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    CreateFolder {
        path: PathBuf,
    },
}

/// Result of a finished job, applied by `Explorer::poll`.
pub(crate) struct Outcome {
    pub notice: Notice,
    /// Directories whose listings may have changed.
    pub touched: Vec<PathBuf>,
    /// Entry to select once its folder reloads.
    pub created: Option<PathBuf>,
}

impl Job {
    pub(crate) fn run(self) -> Outcome {
        match self {
            Self::Delete { paths, permanent } => delete(&paths, permanent),
            Self::Rename { from, to } => rename(&from, &to),
            Self::CreateFolder { path } => create_folder(path),
        }
    }
}

/// Continues past per-item failures and reports the count plus the first error.
fn delete(paths: &[PathBuf], permanent: bool) -> Outcome {
    let mut deleted = 0;
    let mut first_failure: Option<String> = None;
    let mut failed = 0;
    for path in paths {
        match remove(path, permanent) {
            Ok(()) => deleted += 1,
            Err(message) => {
                failed += 1;
                first_failure.get_or_insert_with(|| {
                    let action = if permanent { "delete" } else { "move to trash" };
                    format!("Could not {action} {}: {message}", display_name(path))
                });
            }
        }
    }
    let total = paths.len();
    let verb = if permanent { "Deleted" } else { "Moved" };
    let destination = if permanent { "" } else { " to trash" };
    let notice = match first_failure {
        None if permanent => Notice::info(format!("Deleted {}", items(total))),
        None => Notice::info(format!("Moved {} to trash", items(total))),
        Some(first) if deleted == 0 && total == 1 => Notice::error(first),
        Some(first) if deleted == 0 => {
            Notice::error(format!("{first} ({failed} of {total} items failed)"))
        }
        Some(first) => Notice::error(format!(
            "{verb} {deleted} of {total} items{destination}. {first}"
        )),
    };
    Outcome {
        notice,
        touched: parents(paths.iter().map(PathBuf::as_path)),
        created: None,
    }
}

/// Permanent delete never follows a symlink: the link itself is removed and its target survives.
fn remove(path: &Path, permanent: bool) -> Result<(), String> {
    if !permanent {
        return trash::delete(path).map_err(|err| err.to_string());
    }
    let meta = fs::symlink_metadata(path).map_err(|err| plain(&err))?;
    let result = if meta.is_dir() {
        fs::remove_dir_all(path)
    } else if meta.file_type().is_symlink() {
        unlink_symlink(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|err| plain(&err))
}

#[cfg(not(windows))]
fn unlink_symlink(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

/// Windows needs `remove_dir` for links to directories. `metadata` follows the link, so this only tells us the kind and never touches the target.
#[cfg(windows)]
fn unlink_symlink(path: &Path) -> io::Result<()> {
    if fs::metadata(path).is_ok_and(|meta| meta.is_dir()) {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

fn rename(from: &Path, to: &Path) -> Outcome {
    let (from_name, to_name) = (display_name(from), display_name(to));
    let touched = parents([from, to]);
    let notice = match rename_no_replace(from, to) {
        Ok(()) => Notice::info(format!("Renamed {from_name} to {to_name}")),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            Notice::error(format!("An item named {to_name} already exists"))
        }
        Err(err) => Notice::error(format!("Could not rename {from_name}: {}", plain(&err))),
    };
    Outcome {
        notice,
        touched,
        created: None,
    }
}

/// Renames without ever replacing an existing entry. Linux uses `renameat2(RENAME_NOREPLACE)`, which is atomic.
/// Elsewhere, and on filesystems without that flag, a check precedes the rename and a file created in the gap can be overwritten.
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{CWD, RenameFlags, renameat_with};
        use rustix::io::Errno;
        match renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE) {
            Ok(()) => return Ok(()),
            Err(Errno::INVAL | Errno::NOSYS) => {}
            Err(err) => return Err(err.into()),
        }
    }
    if fs::symlink_metadata(to).is_ok() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    fs::rename(from, to)
}

fn create_folder(path: PathBuf) -> Outcome {
    let name = display_name(&path);
    let touched = parents([path.as_path()]);
    match fs::create_dir(&path) {
        Ok(()) => Outcome {
            notice: Notice::info(format!("Created folder {name}")),
            touched,
            created: Some(path),
        },
        Err(err) => Outcome {
            notice: Notice::error(if err.kind() == io::ErrorKind::AlreadyExists {
                format!("An item named {name} already exists")
            } else {
                format!("Could not create folder {name}: {}", plain(&err))
            }),
            touched,
            created: None,
        },
    }
}

/// Metadata for a Properties dialog. Read without following symlinks; directories get no size (no walk).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryDetails {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    /// Unix: `rwxr-xr-x`. Windows: `read-only` or `read-write`.
    pub permissions: String,
}

pub(crate) fn details(path: &Path) -> Option<EntryDetails> {
    let meta = fs::symlink_metadata(path).ok()?;
    let file_type = meta.file_type();
    let kind = if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Dir
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    Some(EntryDetails {
        name: display_name(path),
        path: path.to_path_buf(),
        kind,
        size: (kind == EntryKind::File).then_some(meta.len()),
        modified: meta.modified().ok(),
        permissions: permissions_text(&meta),
    })
}

#[cfg(unix)]
fn permissions_text(meta: &fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    mode_text(meta.permissions().mode())
}

/// The nine permission bits as `rwxrwxrwx`, with `-` for each unset bit.
#[cfg(unix)]
fn mode_text(mode: u32) -> String {
    const BITS: [(u32, char); 9] = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    BITS.iter()
        .map(|&(bit, c)| if mode & bit != 0 { c } else { '-' })
        .collect()
}

#[cfg(not(unix))]
fn permissions_text(meta: &fs::Metadata) -> String {
    if meta.permissions().readonly() {
        "read-only".to_owned()
    } else {
        "read-write".to_owned()
    }
}

/// Strips the `(os error N)` suffix so messages read as "Permission denied".
fn plain(err: &io::Error) -> String {
    let text = err.to_string();
    match text.rsplit_once(" (os error ") {
        Some((head, _)) => head.to_owned(),
        None => text,
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// Distinct parent directories, in first-seen order.
fn parents<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter_map(Path::parent)
        .filter(|parent| seen.insert(parent.to_path_buf()))
        .map(Path::to_path_buf)
        .collect()
}

fn items(count: usize) -> String {
    if count == 1 {
        "1 item".to_owned()
    } else {
        format!("{count} items")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_names() {
        assert_eq!(validate_name(""), Err(NameError::Empty));
        assert_eq!(validate_name("   "), Err(NameError::Empty));
        assert_eq!(validate_name("."), Err(NameError::Dot));
        assert_eq!(validate_name(".."), Err(NameError::Dot));
        assert_eq!(validate_name("a/b"), Err(NameError::Separator));
        assert_eq!(validate_name("ok name.txt"), Ok(()));
        assert_eq!(validate_name("..hidden"), Ok(()));
    }

    #[test]
    fn rename_never_replaces_an_existing_entry() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        let err = rename_no_replace(&a, &b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&b).unwrap(), b"b");
        rename_no_replace(&a, &dir.path().join("c")).unwrap();
        assert!(!a.exists());
    }

    #[test]
    fn unique_names_skip_taken_ones() {
        let taken = ["New folder", "New folder (2)"];
        let free = |n: &str| !taken.contains(&n);
        assert_eq!(unique_name("New folder", free), "New folder (3)");
        assert_eq!(unique_name("Other", free), "Other");
    }

    #[cfg(unix)]
    #[test]
    fn mode_bits_render_like_ls() {
        assert_eq!(mode_text(0o755), "rwxr-xr-x");
        assert_eq!(mode_text(0o640), "rw-r-----");
        assert_eq!(mode_text(0o000), "---------");
    }

    #[test]
    fn plain_strips_os_error_suffix() {
        let err = io::Error::from_raw_os_error(13);
        assert_eq!(plain(&err), "Permission denied");
    }
}
