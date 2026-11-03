//! `std::fs` backend: baseline, test oracle, and fallback on other Unixes.

use crate::{EntryKind, Listing, Strategy};
use std::fs;
use std::io;
use std::path::Path;

pub(crate) fn enumerate(
    dir: &Path,
    _strategy: &Strategy,
    metadata: bool,
    out: &mut Listing,
) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let kind = entry.file_type().map_or(EntryKind::Other, |t| {
            if t.is_symlink() {
                EntryKind::Symlink
            } else if t.is_dir() {
                EntryKind::Dir
            } else if t.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            }
        });
        // DirEntry::metadata does not follow symlinks; free on Windows, one lstat on Unix.
        let md = if metadata || cfg!(windows) {
            entry.metadata().ok()
        } else {
            None
        };
        let (hidden, inode) = hidden_and_inode(&entry, &name, md.as_ref());
        out.push(&name, kind, hidden, inode);
        if metadata && let Some(md) = md {
            let e = out.entries.last_mut().expect("just pushed");
            e.size = Some(md.len());
            e.modified = md.modified().ok();
        }
    }
    Ok(())
}

#[cfg(unix)]
fn hidden_and_inode(
    entry: &fs::DirEntry,
    name: &std::ffi::OsStr,
    _md: Option<&fs::Metadata>,
) -> (bool, u64) {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::DirEntryExt;
    let dotfile = name.as_bytes().first() == Some(&b'.');
    // macOS also hides files carrying UF_HIDDEN (chflags hidden); needs an lstat even without metadata.
    #[cfg(target_os = "macos")]
    let dotfile = dotfile || {
        use std::os::macos::fs::MetadataExt;
        _md.cloned()
            .or_else(|| entry.metadata().ok())
            .is_some_and(|m| m.st_flags() & libc::UF_HIDDEN != 0)
    };
    (dotfile, entry.ino())
}

#[cfg(windows)]
fn hidden_and_inode(
    _entry: &fs::DirEntry,
    _name: &std::ffi::OsStr,
    md: Option<&fs::Metadata>,
) -> (bool, u64) {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    (
        md.is_some_and(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0),
        0,
    )
}

#[cfg(not(any(unix, windows)))]
fn hidden_and_inode(
    _: &fs::DirEntry,
    name: &std::ffi::OsStr,
    _: Option<&fs::Metadata>,
) -> (bool, u64) {
    (name.to_string_lossy().starts_with('.'), 0)
}
