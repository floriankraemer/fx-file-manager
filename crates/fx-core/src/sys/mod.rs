//! OS dispatch. Every backend module exposes the same two functions:
//!
//! ```ignore
//! /// Appends the entries of `dir` (no `.`/`..`, symlinks not followed) to `out`.
//! /// When `metadata` is set, also fills `size`/`modified` (per-entry failure => None).
//! /// The backend owns enrichment so it can reuse state from enumeration (Linux: the dirfd).
//! pub(crate) fn enumerate(dir: &Path, strategy: &Strategy, metadata: bool, out: &mut Listing) -> io::Result<()>;
//! pub(crate) fn detect_storage(path: &Path) -> StorageKind;
//! pub(crate) fn volume_info(path: &Path) -> VolumeInfo;
//! ```
//!
//! `hidden` and `inode` are always filled (dotfile / FILE_ATTRIBUTE_HIDDEN / UF_HIDDEN).

pub(crate) mod std_fs;

#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux as native;

#[cfg(windows)]
pub(crate) mod windows;
#[cfg(windows)]
pub(crate) use windows as native;

#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos as native;

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
pub(crate) use fallback as native;

/// Native stand-in until an OS backend exists: std enumeration, unknown storage.
#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
pub(crate) mod fallback {
    use crate::StorageKind;
    use std::path::Path;

    pub(crate) use super::std_fs::enumerate;

    pub(crate) fn detect_storage(_path: &Path) -> StorageKind {
        StorageKind::Unknown
    }

    pub(crate) fn volume_info(path: &Path) -> crate::VolumeInfo {
        #[cfg(unix)]
        let id =
            std::fs::symlink_metadata(path).map_or(0, |m| std::os::unix::fs::MetadataExt::dev(&m));
        #[cfg(not(unix))]
        let id = {
            let _ = path;
            0
        };
        crate::VolumeInfo::unknown(id)
    }
}
