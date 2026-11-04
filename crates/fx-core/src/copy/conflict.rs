//! Name clashes at the destination: the policies, the pure decision, and `name (2)` candidates.
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

/// `stem (n).ext` for `n >= 2`, keeping non-UTF-8 bytes intact. `t.tar.gz` becomes `t.tar (2).gz`.
pub fn keep_both_name(name: &OsStr, n: u32) -> OsString {
    let path = Path::new(name);
    let mut out = path.file_stem().unwrap_or(name).to_os_string();
    out.push(format!(" ({n})"));
    if let Some(ext) = path.extension() {
        out.push(".");
        out.push(ext);
    }
    out
}

/// Where an overwrite stages new content before renaming it over `name`, in the same directory.
/// Unique per process and call, so concurrent overwrites never touch each other's file.
pub(crate) fn temp_name(name: &OsStr) -> OsString {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut out = OsString::from(".");
    out.push(name);
    out.push(format!(
        ".{}-{}.fx-part",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
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
    fn keep_both_names_and_temp_names() {
        assert_eq!(keep_both_name("t.tar.gz".as_ref(), 2), "t.tar (2).gz");
        assert_eq!(keep_both_name("sub".as_ref(), 3), "sub (3)");
        assert_eq!(keep_both_name(".bashrc".as_ref(), 2), ".bashrc (2)");
        let (a, b) = (temp_name("a.txt".as_ref()), temp_name("a.txt".as_ref()));
        assert_ne!(a, b);
        let a = a.to_string_lossy();
        assert!(a.starts_with(".a.txt.") && a.ends_with(".fx-part"), "{a}");
    }
}
