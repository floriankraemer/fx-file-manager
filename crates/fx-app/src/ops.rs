//! Disk-touching file operations. A `Job` runs on a worker thread and yields an `Outcome`; `Explorer` schedules jobs and applies outcomes.

use fx_core::{EntryKind, rename_no_replace};
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
            Self::Empty => crate::t("error.name_empty"),
            Self::Separator => crate::t("error.name_separator"),
            Self::Dot => crate::t("error.name_dot"),
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
                    let key = if permanent {
                        "notice.delete_failed"
                    } else {
                        "notice.trash_failed"
                    };
                    crate::tr(key, &[("name", &display_name(path)), ("error", &message)])
                });
            }
        }
    }
    let total = paths.len();
    let notice = match first_failure {
        None => Notice::info(crate::tn(
            if permanent {
                "notice.deleted"
            } else {
                "notice.trashed"
            },
            total as u64,
            &[],
        )),
        Some(first) if deleted == 0 && total == 1 => Notice::error(first),
        Some(first) if deleted == 0 => Notice::error(crate::tr(
            "notice.delete_all_failed",
            &[
                ("first", &first),
                ("failed", &failed.to_string()),
                ("total", &total.to_string()),
            ],
        )),
        Some(first) => Notice::error(crate::tr(
            if permanent {
                "notice.delete_partial"
            } else {
                "notice.trash_partial"
            },
            &[
                ("done", &deleted.to_string()),
                ("total", &total.to_string()),
                ("first", &first),
            ],
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
        Ok(()) => Notice::info(crate::tr(
            "notice.renamed",
            &[("from", &from_name), ("to", &to_name)],
        )),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            Notice::error(crate::tr("notice.exists", &[("name", &to_name)]))
        }
        Err(err) => Notice::error(crate::tr(
            "notice.rename_failed",
            &[("name", &from_name), ("error", &plain(&err))],
        )),
    };
    Outcome {
        notice,
        touched,
        created: None,
    }
}

fn create_folder(path: PathBuf) -> Outcome {
    let name = display_name(&path);
    let touched = parents([path.as_path()]);
    match fs::create_dir(&path) {
        Ok(()) => Outcome {
            notice: Notice::info(crate::tr("notice.folder_created", &[("name", &name)])),
            touched,
            created: Some(path),
        },
        Err(err) => Outcome {
            notice: Notice::error(if err.kind() == io::ErrorKind::AlreadyExists {
                crate::tr("notice.exists", &[("name", &name)])
            } else {
                crate::tr(
                    "notice.folder_failed",
                    &[("name", &name), ("error", &plain(&err))],
                )
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

/// Common error kinds in the UI language; other kinds fall back to the OS text without its `(os error N)` suffix.
fn plain(err: &io::Error) -> String {
    use io::ErrorKind::*;
    let key = match err.kind() {
        NotFound => "os_error.not_found",
        PermissionDenied => "os_error.permission_denied",
        AlreadyExists => "os_error.already_exists",
        DirectoryNotEmpty => "os_error.directory_not_empty",
        StorageFull => "os_error.no_space",
        _ => "",
    };
    if !key.is_empty() {
        return crate::t(key).to_owned();
    }
    let text = err.to_string();
    match text.rsplit_once(" (os error ") {
        Some((head, _)) => head.to_owned(),
        None => text,
    }
}

pub(crate) fn display_name(path: &Path) -> String {
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
    fn mapped_error_kinds_use_the_catalog() {
        use crate::i18n::{Language, find_in};
        for (kind, key) in [
            (io::ErrorKind::NotFound, "os_error.not_found"),
            (
                io::ErrorKind::PermissionDenied,
                "os_error.permission_denied",
            ),
            (io::ErrorKind::AlreadyExists, "os_error.already_exists"),
            (
                io::ErrorKind::DirectoryNotEmpty,
                "os_error.directory_not_empty",
            ),
            (io::ErrorKind::StorageFull, "os_error.no_space"),
        ] {
            assert_eq!(
                plain(&io::Error::from(kind)),
                find_in(Language::En, key).unwrap()
            );
            for lang in Language::CATALOGS {
                assert!(
                    crate::i18n::catalog(lang).contains_key(key),
                    "{lang:?} {key}"
                );
            }
        }
    }

    #[test]
    fn plain_strips_os_error_suffix() {
        // EINVAL / ERROR_INVALID_PARAMETER: a kind with no catalog text, so the OS wording is kept.
        // Windows words its errors in the system language.
        let err = io::Error::from_raw_os_error(if cfg!(windows) { 87 } else { 22 });
        let text = plain(&err);
        assert!(!text.contains("os error") && !text.is_empty(), "{text}");
        assert!(err.to_string().starts_with(&text));
        if cfg!(unix) {
            assert_eq!(text, "Invalid argument");
        }
    }

    fn run(job: Job) -> Outcome {
        job.run()
    }

    #[test]
    fn rename_create_delete_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        fs::write(&a, b"").unwrap();
        run(Job::Rename {
            from: a.clone(),
            to: b.clone(),
        });
        assert!(b.exists() && !a.exists());
        let f = dir.path().join("New");
        let out = run(Job::CreateFolder { path: f.clone() });
        assert!(f.is_dir());
        assert_eq!(out.created, Some(f.clone()));
        run(Job::CreateFolder { path: f.clone() });
        let out = run(Job::Delete {
            paths: vec![b.clone(), f.clone()],
            permanent: true,
        });
        assert!(!b.exists() && !f.exists());
        assert!(!out.notice.text.is_empty());
        let out = run(Job::Delete {
            paths: vec![b],
            permanent: true,
        });
        assert!(
            out.notice.text.starts_with("Could not delete"),
            "{}",
            out.notice.text
        );
    }
}
