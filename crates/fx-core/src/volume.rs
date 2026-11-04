//! Domain: what a volume is made of (filesystem, storage, capabilities), as the copy planner sees it.

use crate::storage::StorageKind;
use std::fmt;

/// `st_dev` on Unix, the volume serial number on Windows. Equal ids mean "same volume", so a rename can work.
pub type VolumeId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FsKind {
    Btrfs,
    Xfs,
    Bcachefs,
    Zfs,
    Ext4,
    F2fs,
    Tmpfs,
    Apfs,
    Hfs,
    Ntfs,
    Refs,
    Fat,
    Exfat,
    Nfs,
    Smb,
    Fuse,
    Other,
}

impl FsKind {
    /// Maps a filesystem type name (macOS `f_fstypename`, Windows `GetVolumeInformation`) to a kind.
    /// Case-insensitive, because Windows reports `NTFS`/`ReFS` and macOS `apfs`/`msdos`.
    pub fn from_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "apfs" => Self::Apfs,
            "hfs" => Self::Hfs,
            "ntfs" | "ntfs3" => Self::Ntfs,
            "refs" => Self::Refs,
            "msdos" | "fat" | "fat12" | "fat16" | "fat32" | "vfat" => Self::Fat,
            "exfat" => Self::Exfat,
            "nfs" => Self::Nfs,
            "smbfs" | "cifs" | "smb2" => Self::Smb,
            "macfuse" | "osxfuse" | "fuse" => Self::Fuse,
            "btrfs" => Self::Btrfs,
            "xfs" => Self::Xfs,
            "ext4" | "ext3" | "ext2" => Self::Ext4,
            "zfs" => Self::Zfs,
            "tmpfs" => Self::Tmpfs,
            _ => Self::Other,
        }
    }
}

impl fmt::Display for FsKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Btrfs => "btrfs",
            Self::Xfs => "xfs",
            Self::Bcachefs => "bcachefs",
            Self::Zfs => "zfs",
            Self::Ext4 => "ext4",
            Self::F2fs => "f2fs",
            Self::Tmpfs => "tmpfs",
            Self::Apfs => "apfs",
            Self::Hfs => "hfs",
            Self::Ntfs => "ntfs",
            Self::Refs => "refs",
            Self::Fat => "fat",
            Self::Exfat => "exfat",
            Self::Nfs => "nfs",
            Self::Smb => "smb",
            Self::Fuse => "fuse",
            Self::Other => "other",
        })
    }
}

/// FAT32 cannot hold a file of 4 GiB or more.
pub const FAT32_MAX_FILE_SIZE: u64 = 4 * 1024 * 1024 * 1024 - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FsCaps {
    /// Extents can be shared (reflink, APFS clone, ReFS block cloning).
    /// A hint only: the copy engine still probes, because e.g. XFS may be formatted without reflink.
    pub clone: bool,
    pub posix_perms: bool,
    pub xattrs: bool,
    pub case_insensitive: bool,
    pub max_file_size: u64,
    /// Coarsest mtime step the filesystem stores; times closer than this count as equal.
    pub mtime_granularity_ns: u64,
}

impl FsCaps {
    /// Pure defaults per filesystem; OS probes may refine `clone` (Windows block-refcounting flag).
    pub fn for_fs(fs: FsKind) -> Self {
        let base = Self {
            clone: false,
            posix_perms: true,
            xattrs: true,
            case_insensitive: false,
            max_file_size: u64::MAX,
            mtime_granularity_ns: 1,
        };
        match fs {
            FsKind::Btrfs | FsKind::Xfs | FsKind::Bcachefs | FsKind::Zfs => Self {
                clone: true,
                ..base
            },
            FsKind::Apfs => Self {
                clone: true,
                case_insensitive: true,
                ..base
            },
            FsKind::Hfs => Self {
                case_insensitive: true,
                mtime_granularity_ns: 1_000_000_000,
                ..base
            },
            FsKind::Ntfs => Self {
                posix_perms: false,
                xattrs: false,
                case_insensitive: true,
                mtime_granularity_ns: 100,
                ..base
            },
            FsKind::Refs => Self {
                clone: true,
                posix_perms: false,
                xattrs: false,
                case_insensitive: true,
                mtime_granularity_ns: 100,
                ..base
            },
            FsKind::Fat => Self {
                posix_perms: false,
                xattrs: false,
                case_insensitive: true,
                max_file_size: FAT32_MAX_FILE_SIZE,
                mtime_granularity_ns: 2_000_000_000,
                ..base
            },
            FsKind::Exfat => Self {
                posix_perms: false,
                xattrs: false,
                case_insensitive: true,
                mtime_granularity_ns: 10_000_000,
                ..base
            },
            FsKind::Smb => Self {
                posix_perms: false,
                xattrs: false,
                case_insensitive: true,
                mtime_granularity_ns: 100,
                ..base
            },
            FsKind::Ext4
            | FsKind::F2fs
            | FsKind::Tmpfs
            | FsKind::Nfs
            | FsKind::Fuse
            | FsKind::Other => base,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeInfo {
    pub id: VolumeId,
    pub fs: FsKind,
    pub storage: StorageKind,
    pub caps: FsCaps,
}

impl VolumeInfo {
    /// What a volume that could not be probed is assumed to be: nothing special, every method falls back.
    pub fn unknown(id: VolumeId) -> Self {
        Self {
            id,
            fs: FsKind::Other,
            storage: StorageKind::Unknown,
            caps: FsCaps::for_fs(FsKind::Other),
        }
    }
}

/// Probes the volume holding `path` (one statfs/volume query). Never fails: unknown parts stay `Other`/`Unknown`.
pub fn volume_info(path: &std::path::Path) -> VolumeInfo {
    crate::sys::native::volume_info(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_map_case_insensitively() {
        for (name, fs) in [
            ("apfs", FsKind::Apfs),
            ("hfs", FsKind::Hfs),
            ("NTFS", FsKind::Ntfs),
            ("ReFS", FsKind::Refs),
            ("FAT32", FsKind::Fat),
            ("msdos", FsKind::Fat),
            ("exFAT", FsKind::Exfat),
            ("smbfs", FsKind::Smb),
            ("nfs", FsKind::Nfs),
            ("macfuse", FsKind::Fuse),
            ("udf", FsKind::Other),
        ] {
            assert_eq!(FsKind::from_name(name), fs, "{name}");
        }
    }

    #[test]
    fn caps_follow_the_filesystem() {
        assert!(FsCaps::for_fs(FsKind::Btrfs).clone);
        assert!(FsCaps::for_fs(FsKind::Apfs).clone);
        assert!(FsCaps::for_fs(FsKind::Refs).clone);
        assert!(!FsCaps::for_fs(FsKind::Ext4).clone);
        let fat = FsCaps::for_fs(FsKind::Fat);
        assert_eq!(fat.max_file_size, FAT32_MAX_FILE_SIZE);
        assert_eq!(fat.mtime_granularity_ns, 2_000_000_000);
        assert!(!fat.posix_perms && !fat.xattrs && fat.case_insensitive);
        assert_eq!(
            FsCaps::for_fs(FsKind::Exfat).mtime_granularity_ns,
            10_000_000
        );
        assert!(FsCaps::for_fs(FsKind::Ext4).posix_perms);
    }
}
