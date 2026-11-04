//! Linux backend: getdents64 via `RawDir`, dirfd-relative statx, statfs + sysfs storage detection.

use crate::pool::pool;
use crate::{EntryKind, FsCaps, FsKind, Listing, StorageKind, Strategy, VolumeInfo};
use rayon::prelude::*;
use rustix::fs::{
    AtFlags, CWD, FileType, Mode, OFlags, RawDir, StatxFlags, major, minor, openat, statfs, statx,
};
use std::ffi::CStr;
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Stat calls handed to one rayon task; amortises scheduling over ~tens of microseconds of syscalls.
const STAT_CHUNK: usize = 256;
/// Below this many entries the pool hand-off costs more than it saves.
const MIN_PARALLEL_ENTRIES: usize = 512;

type Stat = (u64, Option<SystemTime>);

pub(crate) fn enumerate(
    dir: &Path,
    strategy: &Strategy,
    metadata: bool,
    out: &mut Listing,
) -> io::Result<()> {
    let fd = openat(
        CWD,
        dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let first = out.entries.len();
    // NUL-terminated copies of the names, so statx can use them without allocating per entry.
    let mut cnames: Vec<u8> = Vec::new();
    let mut name_ends: Vec<usize> = Vec::new();

    let mut buf = vec![MaybeUninit::<u8>::uninit(); strategy.buf_size.max(4096)];
    let mut raw = RawDir::new(&fd, &mut buf);
    while let Some(entry) = raw.next() {
        let entry = entry?;
        let name = entry.file_name();
        let bytes = name.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let kind = match entry.file_type() {
            FileType::Unknown => kind_via_statx(fd.as_fd(), name),
            t => kind_from_file_type(t),
        };
        out.push_encoded(bytes, kind, bytes[0] == b'.', entry.ino());
        cnames.extend_from_slice(name.to_bytes_with_nul());
        name_ends.push(cnames.len());
    }

    if metadata {
        let stats = stat_all(
            fd.as_fd(),
            &cnames,
            &name_ends,
            &out.entries[first..],
            strategy,
        );
        for (e, s) in out.entries[first..].iter_mut().zip(stats) {
            if let Some((size, modified)) = s {
                e.size = Some(size);
                e.modified = modified;
            }
        }
    }
    Ok(())
}

fn cname<'a>(cnames: &'a [u8], ends: &[usize], i: usize) -> &'a CStr {
    let start = if i == 0 { 0 } else { ends[i - 1] };
    CStr::from_bytes_with_nul(&cnames[start..ends[i]]).expect("names are NUL-terminated copies")
}

fn kind_from_file_type(t: FileType) -> EntryKind {
    match t {
        FileType::RegularFile => EntryKind::File,
        FileType::Directory => EntryKind::Dir,
        FileType::Symlink => EntryKind::Symlink,
        _ => EntryKind::Other,
    }
}

fn kind_via_statx(dirfd: BorrowedFd<'_>, name: &CStr) -> EntryKind {
    statx(dirfd, name, AtFlags::SYMLINK_NOFOLLOW, StatxFlags::TYPE).map_or(EntryKind::Other, |s| {
        kind_from_file_type(FileType::from_raw_mode(s.stx_mode.into()))
    })
}

fn stat_one(dirfd: BorrowedFd<'_>, name: &CStr) -> Option<Stat> {
    let flags = StatxFlags::TYPE | StatxFlags::SIZE | StatxFlags::MTIME;
    let s = statx(dirfd, name, AtFlags::SYMLINK_NOFOLLOW, flags).ok()?;
    let mtime = &s.stx_mtime;
    let nanos = Duration::from_nanos(u64::from(mtime.tv_nsec));
    let secs = Duration::from_secs(mtime.tv_sec.unsigned_abs());
    // Pre-1970 stamps: seconds count backwards, nanoseconds forwards (as std does).
    let modified = if mtime.tv_sec >= 0 {
        UNIX_EPOCH.checked_add(secs)
    } else {
        UNIX_EPOCH.checked_sub(secs)
    }
    .and_then(|t| t.checked_add(nanos));
    Some((s.stx_size, modified))
}

fn stat_all(
    dirfd: BorrowedFd<'_>,
    cnames: &[u8],
    ends: &[usize],
    entries: &[crate::Entry],
    strategy: &Strategy,
) -> Vec<Option<Stat>> {
    let n = entries.len();
    let mut order: Vec<usize> = (0..n).collect();
    if strategy.inode_order {
        order.sort_unstable_by_key(|&i| entries[i].inode);
    }
    let one = |i: usize| stat_one(dirfd, cname(cnames, ends, i));

    let mut results = vec![None; n];
    if strategy.stat_threads <= 1 || n < MIN_PARALLEL_ENTRIES {
        for &i in &order {
            results[i] = one(i);
        }
        return results;
    }
    let done: Vec<Vec<(usize, Option<Stat>)>> = pool(strategy.stat_threads).install(|| {
        order
            .par_chunks(STAT_CHUNK)
            .map(|c| c.iter().map(|&i| (i, one(i))).collect())
            .collect()
    });
    for (i, s) in done.into_iter().flatten() {
        results[i] = s;
    }
    results
}

pub(crate) fn detect_storage(path: &Path) -> StorageKind {
    let Ok(fs) = statfs(path) else {
        return StorageKind::Unknown;
    };
    // f_type is signed or 32-bit depending on the arch; compare the low 32 bits.
    if let Some(kind) = kind_from_fs_magic(fs.f_type as u64 & 0xFFFF_FFFF) {
        return kind;
    }
    let Ok(st) = rustix::fs::stat(path) else {
        return StorageKind::Unknown;
    };
    rotational_kind(major(st.st_dev), minor(st.st_dev))
}

/// Network and FUSE-like filesystems by `statfs` magic; `None` means "ask the block device".
fn kind_from_fs_magic(magic: u64) -> Option<StorageKind> {
    const NFS: u64 = 0x6969;
    const CIFS: u64 = 0xFF53_4D42;
    const SMB2: u64 = 0xFE53_4D42;
    const SMB: u64 = 0x517B;
    const V9FS: u64 = 0x0102_1997;
    const FUSE: u64 = 0x6573_5546;
    const CEPH: u64 = 0x00C3_6400;
    const AFS: u64 = 0x5346_414F;
    matches!(magic, NFS | CIFS | SMB2 | SMB | V9FS | FUSE | CEPH | AFS)
        .then_some(StorageKind::Network)
}

/// Reads `rel` of the block device, falling back to its parent disk: a partition has no
/// `queue/`, `device/` or `removable` of its own.
fn block_attr(major: u32, minor: u32, rel: &str) -> Option<String> {
    let base = format!("/sys/dev/block/{major}:{minor}");
    [format!("{base}/{rel}"), format!("{base}/../{rel}")]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
}

fn rotational_kind(major: u32, minor: u32) -> StorageKind {
    let rotational = block_attr(major, minor, "queue/rotational");
    let model = block_attr(major, minor, "device/model").unwrap_or_default();
    kind_from_block_info(rotational.as_deref(), &model)
}

/// One `stat` and one `statfs`; block-device storage adds a few small sysfs reads.
pub(crate) fn volume_info(path: &Path) -> VolumeInfo {
    let Ok(st) = rustix::fs::stat(path) else {
        return VolumeInfo::unknown(0);
    };
    let id = st.st_dev;
    let Ok(fs) = statfs(path) else {
        return VolumeInfo::unknown(id);
    };
    let magic = fs.f_type as u64 & 0xFFFF_FFFF;
    let kind = fs_kind_from_magic(magic);
    let storage = kind_from_fs_magic(magic).unwrap_or_else(|| {
        let (major, minor) = (major(id), minor(id));
        let device = std::fs::canonicalize(format!("/sys/dev/block/{major}:{minor}")).ok();
        let flag = block_attr(major, minor, "removable");
        if is_removable(flag.as_deref(), device.as_deref().and_then(Path::to_str)) {
            StorageKind::Removable
        } else {
            rotational_kind(major, minor)
        }
    });
    VolumeInfo {
        id,
        fs: kind,
        storage,
        caps: FsCaps::for_fs(kind),
    }
}

/// `removable` is the sysfs flag; `device` the resolved `/sys/dev/block/M:m` path,
/// which runs through `.../usbN/...` for USB disks that do not set the flag (most USB SSDs).
fn is_removable(removable: Option<&str>, device: Option<&str>) -> bool {
    removable.map(str::trim) == Some("1")
        || device.is_some_and(|d| d.split('/').any(|part| part.starts_with("usb")))
}

fn fs_kind_from_magic(magic: u64) -> FsKind {
    match magic {
        0x9123_683E => FsKind::Btrfs,
        0x5846_5342 => FsKind::Xfs,
        0xEF53 => FsKind::Ext4,
        0xCA45_1A4E => FsKind::Bcachefs,
        0x2FC1_2FC1 => FsKind::Zfs,
        0xF2F5_2010 => FsKind::F2fs,
        0x0102_1994 => FsKind::Tmpfs,
        0x4D44 => FsKind::Fat,
        0x2011_BAB0 => FsKind::Exfat,
        0x7366_746E | 0x5346_544E => FsKind::Ntfs,
        0x6969 => FsKind::Nfs,
        0xFF53_4D42 | 0xFE53_4D42 | 0x517B => FsKind::Smb,
        0x6573_5546 => FsKind::Fuse,
        _ => FsKind::Other,
    }
}

/// Hypervisor disks (Hyper-V/WSL2, QEMU, VirtualBox, VMware) report `rotational=1`
/// by default without knowing the backing media, so their flag is not trusted.
// ponytail: model-string heuristic; measure seek latency if VMs on real HDDs matter.
fn kind_from_block_info(rotational: Option<&str>, model: &str) -> StorageKind {
    let model = model.to_ascii_lowercase();
    let virtual_disk = ["virtual", "qemu", "vbox"]
        .iter()
        .any(|m| model.contains(m));
    match rotational.map(str::trim) {
        Some("1") if virtual_disk => StorageKind::Unknown,
        Some("1") => StorageKind::Hdd,
        Some("0") => StorageKind::Ssd,
        _ => StorageKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_disks_do_not_count_as_hdd() {
        assert_eq!(
            kind_from_block_info(Some("1\n"), "Virtual Disk    \n"),
            StorageKind::Unknown
        );
        assert_eq!(
            kind_from_block_info(Some("1"), "QEMU HARDDISK"),
            StorageKind::Unknown
        );
        assert_eq!(
            kind_from_block_info(Some("1"), "ST4000DM004"),
            StorageKind::Hdd
        );
        assert_eq!(
            kind_from_block_info(Some("0"), "Samsung SSD 980"),
            StorageKind::Ssd
        );
        assert_eq!(kind_from_block_info(None, ""), StorageKind::Unknown);
    }

    #[test]
    fn fs_magics_map_to_kinds() {
        for (magic, kind) in [
            (0x9123_683E, FsKind::Btrfs),
            (0x5846_5342, FsKind::Xfs),
            (0xEF53, FsKind::Ext4),
            (0xCA45_1A4E, FsKind::Bcachefs),
            (0x2FC1_2FC1, FsKind::Zfs),
            (0xF2F5_2010, FsKind::F2fs),
            (0x0102_1994, FsKind::Tmpfs),
            (0x4D44, FsKind::Fat),
            (0x2011_BAB0, FsKind::Exfat),
            (0x7366_746E, FsKind::Ntfs),
            (0x6969, FsKind::Nfs),
            (0xFF53_4D42, FsKind::Smb),
            (0x6573_5546, FsKind::Fuse),
            (0x0102_1997, FsKind::Other),
        ] {
            assert_eq!(fs_kind_from_magic(magic), kind, "{magic:#x}");
        }
    }

    #[test]
    fn removable_by_flag_or_usb_path() {
        assert!(is_removable(Some("1\n"), None));
        assert!(is_removable(
            Some("0\n"),
            Some(
                "/sys/devices/pci0000:00/0000:00:14.0/usb2/2-1/2-1:1.0/host6/target6:0:0/6:0:0:0/block/sdb"
            )
        ));
        assert!(!is_removable(
            Some("0"),
            Some("/sys/devices/pci0000:00/0000:00:1d.0/0000:3d:00.0/nvme/nvme0/nvme0n1")
        ));
        assert!(!is_removable(None, None));
    }

    #[test]
    fn volume_info_of_a_temp_dir_is_sane() {
        let dir = tempfile::tempdir().unwrap();
        let info = volume_info(dir.path());
        assert_eq!(info.id, rustix::fs::stat(dir.path()).unwrap().st_dev);
        assert_eq!(info.caps, FsCaps::for_fs(info.fs));
    }

    #[test]
    fn network_magics_are_network_and_others_are_not() {
        for m in [
            0x6969,
            0xFF53_4D42,
            0xFE53_4D42,
            0x517B,
            0x0102_1997,
            0x6573_5546,
            0x00C3_6400,
            0x5346_414F,
        ] {
            assert_eq!(kind_from_fs_magic(m), Some(StorageKind::Network), "{m:#x}");
        }
        assert_eq!(kind_from_fs_magic(0xEF53), None); // ext4
        assert_eq!(kind_from_fs_magic(0x9123_683E), None); // btrfs
    }
}
