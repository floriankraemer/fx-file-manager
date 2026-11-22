//! Name clashes at the destination: the policies, the pure decision, and candidate names beside a clash.
//!
//! Existence is detected by the atomic exclusive create itself (`O_EXCL`, `CREATE_NEW`,
//! `COPY_FILE_FAIL_IF_EXISTS`), never by a check beforehand, so a clash can be neither missed nor invented.

use super::Meta;
use crate::EntryKind;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What to do when an entry already exists. Folder onto folder always merges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConflictPolicy {
    /// Queue the conflict for the user; only the conflicting item waits.
    #[default]
    Ask,
    Skip,
    /// Atomically: the new content is written next to the target and renamed over it.
    Overwrite,
    /// Copy as `name (2)`, `name (3)`, ...
    KeepBoth,
    /// Overwrite only when the source is newer than the destination.
    OverwriteIfNewer,
    /// Skip when size and mtime match (within the destination's granularity), overwrite otherwise.
    /// This is the resume policy: an interrupted file keeps a fresh mtime, so it is rewritten.
    SkipIdentical,
}

/// The answer for one conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Skip,
    Overwrite,
    KeepBoth,
}

/// A clash waiting for the user, as shown in a conflict dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conflict {
    pub id: u64,
    pub src: PathBuf,
    pub dst: PathBuf,
    pub src_is_dir: bool,
    pub dst_is_dir: bool,
    pub src_size: u64,
    pub dst_size: u64,
    pub src_modified: Option<SystemTime>,
    pub dst_modified: Option<SystemTime>,
}

impl Conflict {
    pub(crate) fn new(src: &Path, src_meta: &Meta, dst: &Path, dst_meta: &Meta) -> Self {
        Self {
            id: 0,
            src: src.to_path_buf(),
            dst: dst.to_path_buf(),
            src_is_dir: src_meta.kind == EntryKind::Dir,
            dst_is_dir: dst_meta.kind == EntryKind::Dir,
            src_size: src_meta.size,
            dst_size: dst_meta.size,
            src_modified: src_meta.mtime.system_time(),
            dst_modified: dst_meta.mtime.system_time(),
        }
    }
}

/// Pure decision for `policy`; `None` means "ask the user". `granularity_ns` is the destination's mtime step.
pub(crate) fn resolve(
    policy: ConflictPolicy,
    src: &Meta,
    dst: &Meta,
    granularity_ns: u64,
) -> Option<Resolution> {
    let delta = src.mtime.nanos_since(dst.mtime);
    let same_time = delta.unsigned_abs() < u128::from(granularity_ns.max(1));
    match policy {
        ConflictPolicy::Ask => None,
        ConflictPolicy::Skip => Some(Resolution::Skip),
        ConflictPolicy::Overwrite => Some(Resolution::Overwrite),
        ConflictPolicy::KeepBoth => Some(Resolution::KeepBoth),
        ConflictPolicy::OverwriteIfNewer if delta > 0 && !same_time => Some(Resolution::Overwrite),
        ConflictPolicy::OverwriteIfNewer => Some(Resolution::Skip),
        ConflictPolicy::SkipIdentical
            if src.kind == dst.kind && src.size == dst.size && same_time =>
        {
            Some(Resolution::Skip)
        }
        ConflictPolicy::SkipIdentical => Some(Resolution::Overwrite),
    }
}

/// Where a copy lands beside an existing item of the same name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Beside {
    /// After a "keep both" answer: `name (2)`, `name (3)`, ...
    Numbered,
    /// A file copied onto itself: `name copy (1).ext`, `name copy (2).ext`, ...
    Copy,
    /// A folder copied onto itself: `name copy (1)`, `name copy (2)`, ...
    CopyFolder,
}

impl Beside {
    /// The first index `name` takes: `name (2)` and `name copy (1)` are the first candidates.
    pub(crate) fn first(self) -> u32 {
        match self {
            Beside::Numbered => 2,
            Beside::Copy | Beside::CopyFolder => 1,
        }
    }

    pub(crate) fn name(self, name: &OsStr, n: u32) -> OsString {
        match self {
            Beside::Numbered => keep_both_name(name, n),
            Beside::Copy => copy_name(name, n),
            Beside::CopyFolder => copy_folder_name(name, n),
        }
    }
}

/// `stem (n).ext` for `n >= 2`, keeping non-UTF-8 bytes intact. `t.tar.gz` becomes `t.tar (2).gz`.
pub fn keep_both_name(name: &OsStr, n: u32) -> OsString {
    let path = Path::new(name);
    let mut out = path.file_stem().unwrap_or(name).to_os_string();
    out.push(format!(" ({n})"));
    push_extension(&mut out, path);
    out
}

/// `stem copy (n).ext`; `t.tar.gz` becomes `t.tar copy (1).gz`. Copying a copy numbers the original again.
fn copy_name(name: &OsStr, n: u32) -> OsString {
    let path = Path::new(name);
    let stem = path.file_stem().unwrap_or(name);
    let mut out = copy_suffix_stripped(stem);
    out.push(format!(" copy ({n})"));
    push_extension(&mut out, path);
    out
}

/// `name copy (n)`; a folder has no extension to keep apart.
fn copy_folder_name(name: &OsStr, n: u32) -> OsString {
    let mut out = copy_suffix_stripped(name);
    out.push(format!(" copy ({n})"));
    out
}

/// `name` without a trailing ` copy` or ` copy (N)`, so copies of copies never stack the word.
fn copy_suffix_stripped(name: &OsStr) -> OsString {
    let Some(text) = name.to_str() else {
        return name.to_os_string();
    };
    let base = match text
        .strip_suffix(')')
        .and_then(|t| t.rsplit_once(" copy ("))
    {
        Some((base, digits)) if digits.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => text.strip_suffix(" copy").unwrap_or(text),
    };
    OsString::from(if base.is_empty() { text } else { base })
}

fn push_extension(out: &mut OsString, path: &Path) {
    if let Some(ext) = path.extension() {
        out.push(".");
        out.push(ext);
    }
}

/// Longest file name the supported filesystems store, in bytes.
const NAME_MAX: usize = 255;

/// Where an overwrite stages new content before renaming it over `name`, in the same directory.
/// Unique per process and call, so concurrent overwrites never touch each other's file.
/// A name too long to carry the decoration is left out, so the temp name still fits.
pub(crate) fn temp_name(name: &OsStr) -> OsString {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let unique = format!(
        ".{}-{}.fx-part",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let mut out = OsString::from(".");
    if name.len() + 1 + unique.len() <= NAME_MAX {
        out.push(name);
    }
    out.push(unique);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::copy::Time;

    fn meta(size: u64, secs: i64, nanos: u32) -> Meta {
        Meta {
            kind: EntryKind::File,
            size,
            mtime: Time { secs, nanos },
            ..Meta::default()
        }
    }

    #[test]
    fn policies_decide_without_asking_except_ask() {
        use ConflictPolicy::*;
        use Resolution as R;
        let old = meta(10, 100, 0);
        let new = meta(10, 200, 0);
        let rows = [
            (Ask, &new, &old, None),
            (Skip, &new, &old, Some(R::Skip)),
            (Overwrite, &old, &new, Some(R::Overwrite)),
            (KeepBoth, &new, &old, Some(R::KeepBoth)),
            (OverwriteIfNewer, &new, &old, Some(R::Overwrite)),
            (OverwriteIfNewer, &old, &new, Some(R::Skip)),
            (OverwriteIfNewer, &old, &old, Some(R::Skip)),
            (SkipIdentical, &old, &old, Some(R::Skip)),
            (SkipIdentical, &new, &old, Some(R::Overwrite)),
        ];
        for (policy, src, dst, expected) in rows {
            assert_eq!(resolve(policy, src, dst, 1), expected, "{policy:?}");
        }
        assert_eq!(
            resolve(SkipIdentical, &meta(11, 100, 0), &old, 1),
            Some(R::Overwrite),
            "a different size is never identical"
        );
    }

    #[test]
    fn granularity_makes_close_times_equal() {
        // FAT stores mtimes in 2 s steps: 100.0 s and 101.5 s are the same time there.
        let src = meta(1, 101, 500_000_000);
        let dst = meta(1, 100, 0);
        assert_eq!(
            resolve(ConflictPolicy::SkipIdentical, &src, &dst, 2_000_000_000),
            Some(Resolution::Skip)
        );
        assert_eq!(
            resolve(ConflictPolicy::OverwriteIfNewer, &src, &dst, 2_000_000_000),
            Some(Resolution::Skip)
        );
        assert_eq!(
            resolve(ConflictPolicy::SkipIdentical, &src, &dst, 1),
            Some(Resolution::Overwrite)
        );
    }

    #[test]
    fn copy_beside_names() {
        let file = |n| Beside::Copy.name("t.tar.gz".as_ref(), n);
        assert_eq!(file(1), "t.tar copy (1).gz");
        assert_eq!(file(2), "t.tar copy (2).gz");
        assert_eq!(Beside::Copy.name("README".as_ref(), 1), "README copy (1)");
        assert_eq!(Beside::Copy.name(".bashrc".as_ref(), 1), ".bashrc copy (1)");
        assert_eq!(
            Beside::Copy.name("f copy (1).txt".as_ref(), 2),
            "f copy (2).txt"
        );
        assert_eq!(
            Beside::Copy.name("f copy.txt".as_ref(), 1),
            "f copy (1).txt"
        );
        assert_eq!(
            Beside::Copy.name("copy.txt".as_ref(), 1),
            "copy copy (1).txt"
        );
        let folder = |n| Beside::CopyFolder.name("my.dir".as_ref(), n);
        assert_eq!(folder(1), "my.dir copy (1)");
        assert_eq!(folder(2), "my.dir copy (2)");
        assert_eq!(
            Beside::CopyFolder.name("d copy (1)".as_ref(), 2),
            "d copy (2)"
        );
        assert_eq!(Beside::Copy.first(), 1);
        assert_eq!(Beside::Numbered.first(), 2);
    }

    #[test]
    fn keep_both_names_and_temp_names() {
        assert_eq!(keep_both_name("t.tar.gz".as_ref(), 2), "t.tar (2).gz");
        assert_eq!(keep_both_name("sub".as_ref(), 3), "sub (3)");
        assert_eq!(keep_both_name(".bashrc".as_ref(), 2), ".bashrc (2)");
        let (a, b) = (temp_name("a.txt".as_ref()), temp_name("a.txt".as_ref()));
        assert_ne!(a, b);
        let a = a.to_string_lossy();
        assert!(a.starts_with(".a.txt.") && a.ends_with(".fx-part"), "{a}");
        let long = temp_name("x".repeat(255).as_ref());
        assert!(long.len() <= 255 && long.to_string_lossy().ends_with(".fx-part"));
    }
}
