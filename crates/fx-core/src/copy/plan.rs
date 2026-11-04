//! Strategy selection as data: `CopyPlan::for_pair` is pure, like `Strategy::for_kind`.
//!
//! Every `Method` exists on every OS so the selection matrix can be table-tested for all OSes on any host;
//! the OS backend reports a foreign method as unsupported and the chain moves on.

use crate::storage::StorageKind;
use crate::volume::{FsKind, VolumeInfo};
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;
const MIB_U64: u64 = 1024 * 1024;
const GIB_U64: u64 = 1024 * MIB_U64;

/// Requests that hide network latency; Phase 4 benchmarks tune 8/16/32.
const NETWORK_WORKERS: usize = 16;
/// USB sticks thrash (and throttle) with more parallel writers.
const REMOVABLE_WORKERS: usize = 2;
/// Clones are metadata-only, so a few workers do not seek-thrash an HDD.
const SLOW_CLONE_WORKERS: usize = 4;
/// Files at least this large get preallocation and sequential read-ahead.
const LARGE_FILE: u64 = 64 * MIB_U64;
/// `COPY_FILE_NO_BUFFERING` threshold, as robocopy `/J` recommends.
const WINDOWS_LARGE_FILE: u64 = 256 * MIB_U64;
/// Above this the copy stops polluting the page cache.
const UNCACHED_FROM: u64 = GIB_U64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Copy,
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Inode,
    /// Physical location of the first extent (FIEMAP / retrieval pointers), inode order where unknown.
    Extent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Schedule {
    /// Walk and copy fused in one parallel pass (SSD, network, removable).
    Streaming,
    /// Walk everything, create the tree, then copy files sorted by disk position with one data worker (HDD).
    TwoPhase { order: Order },
}

/// A data-transfer algorithm. Tried in plan order per file; unsupported ones are disabled for the volume pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Method {
    /// Same-volume move: one no-replace rename per top-level item.
    Rename,
    /// Linux reflink (`ioctl(FICLONE)`).
    Ficlone,
    /// Linux in-kernel copy; may reflink, NFS 4.2/CIFS copy server-side.
    CopyFileRange,
    /// Linux in-kernel copy across any filesystems.
    Sendfile,
    /// macOS APFS clone (`clonefileat`), data and metadata in one call.
    CloneFileAt,
    /// Windows `CopyFile2`: ADS, attributes, SMB copychunk/ODX, block clone on 24H2+.
    CopyFile2,
    /// Windows ReFS `FSCTL_DUPLICATE_EXTENTS_TO_FILE`.
    DuplicateExtents,
    /// Portable user-space buffer; always last.
    ReadWrite,
}

impl Method {
    pub const ALL: [Method; 8] = [
        Method::Rename,
        Method::Ficlone,
        Method::CopyFileRange,
        Method::Sendfile,
        Method::CloneFileAt,
        Method::CopyFile2,
        Method::DuplicateExtents,
        Method::ReadWrite,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Method::Rename => "rename",
            Method::Ficlone => "ficlone",
            Method::CopyFileRange => "copy-file-range",
            Method::Sendfile => "sendfile",
            Method::CloneFileAt => "clonefileat",
            Method::CopyFile2 => "copyfile2",
            Method::DuplicateExtents => "duplicate-extents",
            Method::ReadWrite => "read-write",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The chain that tries only this method (benchmark override).
    pub fn only(self) -> &'static [Method] {
        match self {
            Method::Rename => &[Method::Rename],
            Method::Ficlone => &[Method::Ficlone],
            Method::CopyFileRange => &[Method::CopyFileRange],
            Method::Sendfile => &[Method::Sendfile],
            Method::CloneFileAt => &[Method::CloneFileAt],
            Method::CopyFile2 => &[Method::CopyFile2],
            Method::DuplicateExtents => &[Method::DuplicateExtents],
            Method::ReadWrite => &[Method::ReadWrite],
        }
    }

    /// Shares extents instead of copying data; counted as "cloned" in the report.
    pub fn clones(self) -> bool {
        matches!(
            self,
            Method::Ficlone | Method::CloneFileAt | Method::DuplicateExtents
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Durability {
    /// Plain copy: no fsync, like `cp`, robocopy and Finder.
    None,
    /// Cross-volume move: the destination is flushed before any source is deleted.
    BeforeSourceDelete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    Macos,
    Windows,
    Other,
}

impl Os {
    pub const CURRENT: Os = if cfg!(target_os = "linux") {
        Os::Linux
    } else if cfg!(target_os = "macos") {
        Os::Macos
    } else if cfg!(windows) {
        Os::Windows
    } else {
        Os::Other
    };
}

const RENAME: &[Method] = &[Method::Rename];
const LINUX_CLONE: &[Method] = &[Method::Ficlone, Method::CopyFileRange, Method::ReadWrite];
const LINUX_SAME_FS: &[Method] = &[Method::CopyFileRange, Method::ReadWrite];
const LINUX_CROSS_FS: &[Method] = &[Method::CopyFileRange, Method::Sendfile, Method::ReadWrite];
const MACOS_CLONE: &[Method] = &[Method::CloneFileAt, Method::ReadWrite];
const WINDOWS: &[Method] = &[Method::CopyFile2, Method::ReadWrite];
const WINDOWS_CLONE: &[Method] = &[
    Method::CopyFile2,
    Method::DuplicateExtents,
    Method::ReadWrite,
];
const PORTABLE: &[Method] = &[Method::ReadWrite];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyPlan {
    pub workers: usize,
    pub schedule: Schedule,
    /// Tried in order per file.
    pub methods: &'static [Method],
    /// User-space fallback buffer.
    pub buf_size: usize,
    /// Files at least this large take the large-file path (prealloc, fadvise, `NO_BUFFERING`).
    pub large_file: u64,
    /// Bypass the page cache above this size.
    pub uncached_from: Option<u64>,
    pub durability: Durability,
}

/// Benchmark and settings overrides; `None` keeps the planned value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Overrides {
    /// Replaces detected storage on both sides before planning.
    pub storage: Option<StorageKind>,
    pub workers: Option<usize>,
    /// Forces a single-method chain; on a move this also skips the rename.
    pub method: Option<Method>,
    pub schedule: Option<Schedule>,
}

impl CopyPlan {
    /// Pure: `cores` is passed in so the result is deterministic.
    pub fn for_pair(src: &VolumeInfo, dst: &VolumeInfo, op: Op, cores: usize) -> Self {
        Self::for_pair_on(Os::CURRENT, src, dst, op, cores)
    }

    /// [`for_pair`](Self::for_pair) for any OS, so every row of the selection matrix is testable anywhere.
    pub fn for_pair_on(os: Os, src: &VolumeInfo, dst: &VolumeInfo, op: Op, cores: usize) -> Self {
        if op == Op::Move && src.id == dst.id {
            return Self {
                workers: 1,
                schedule: Schedule::Streaming,
                methods: RENAME,
                buf_size: MIB,
                large_file: LARGE_FILE,
                uncached_from: None,
                durability: Durability::None,
            };
        }
        let mut plan = copy_plan(os, src, dst, cores);
        if op == Op::Move {
            plan.durability = Durability::BeforeSourceDelete;
        }
        plan
    }

    /// The data plan for a move whose rename failed with `EXDEV` (bind mounts, btrfs subvolumes).
    pub fn for_moved_data_on(os: Os, src: &VolumeInfo, dst: &VolumeInfo, cores: usize) -> Self {
        Self {
            durability: Durability::BeforeSourceDelete,
            ..copy_plan(os, src, dst, cores)
        }
    }

    pub fn with(mut self, o: &Overrides) -> Self {
        if let Some(workers) = o.workers {
            self.workers = workers.max(1);
        }
        if let Some(method) = o.method {
            self.methods = method.only();
        }
        if let Some(schedule) = o.schedule {
            self.schedule = schedule;
        }
        self
    }

    pub fn renames(&self) -> bool {
        self.methods == RENAME
    }
}

/// Rules in order: network, clone-capable pair, HDD, removable, SSD (`Unknown` behaves like `Ssd`).
fn copy_plan(os: Os, src: &VolumeInfo, dst: &VolumeInfo, cores: usize) -> CopyPlan {
    let either = |kind| src.storage == kind || dst.storage == kind;
    let same_volume = src.id == dst.id;
    let cores = cores.max(1);
    let ssd_workers = match os {
        Os::Windows => (2 * cores).clamp(8, 32),
        _ => (2 * cores).clamp(4, 32),
    };
    let (large_file, uncached_from) = match os {
        Os::Windows => (WINDOWS_LARGE_FILE, Some(WINDOWS_LARGE_FILE)),
        _ => (LARGE_FILE, Some(UNCACHED_FROM)),
    };
    let base = CopyPlan {
        workers: ssd_workers,
        schedule: Schedule::Streaming,
        methods: PORTABLE,
        buf_size: MIB,
        large_file,
        uncached_from,
        durability: Durability::None,
    };
    let slow_buf = if either(StorageKind::Hdd) {
        8 * MIB
    } else if either(StorageKind::Removable) {
        4 * MIB
    } else {
        MIB
    };

    if either(StorageKind::Network) {
        return CopyPlan {
            workers: NETWORK_WORKERS,
            methods: match os {
                Os::Linux => LINUX_SAME_FS,
                Os::Windows => WINDOWS,
                Os::Macos | Os::Other => PORTABLE,
            },
            // The redirector does its own buffering; NO_BUFFERING is for local disks only.
            uncached_from: if os == Os::Windows {
                None
            } else {
                uncached_from
            },
            ..base
        };
    }
    let clone = dst.caps.clone
        && match os {
            // btrfs subvolumes share extents across different st_dev; EXDEV elsewhere disables it once.
            Os::Linux => src.fs == dst.fs,
            Os::Macos => same_volume && dst.fs == FsKind::Apfs,
            Os::Windows => same_volume,
            Os::Other => false,
        };
    if clone {
        return CopyPlan {
            workers: if either(StorageKind::Hdd) || either(StorageKind::Removable) {
                SLOW_CLONE_WORKERS
            } else {
                ssd_workers
            },
            methods: match os {
                Os::Linux => LINUX_CLONE,
                Os::Macos => MACOS_CLONE,
                Os::Windows => WINDOWS_CLONE,
                Os::Other => PORTABLE,
            },
            buf_size: slow_buf,
            ..base
        };
    }
    let local = match os {
        Os::Linux => LINUX_CROSS_FS,
        Os::Windows => WINDOWS,
        Os::Macos | Os::Other => PORTABLE,
    };
    if either(StorageKind::Hdd) {
        return CopyPlan {
            workers: 1,
            schedule: Schedule::TwoPhase {
                order: Order::Extent,
            },
            methods: local,
            buf_size: slow_buf,
            ..base
        };
    }
    if either(StorageKind::Removable) {
        return CopyPlan {
            workers: REMOVABLE_WORKERS,
            methods: local,
            buf_size: slow_buf,
            ..base
        };
    }
    CopyPlan {
        methods: match os {
            Os::Linux if same_volume => LINUX_SAME_FS,
            _ => local,
        },
        ..base
    }
}

/// Methods that failed as unsupported for one volume pair, as a lock-free bitset.
#[derive(Debug, Default)]
pub(crate) struct MethodCache(AtomicU8);

impl MethodCache {
    pub(crate) fn enabled(&self, m: Method) -> bool {
        self.0.load(Relaxed) & (1 << m as u8) == 0
    }

    pub(crate) fn disable(&self, m: Method) {
        self.0.fetch_or(1 << m as u8, Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::volume::FsCaps;
    use Method::*;

    fn vol(id: u64, fs: FsKind, storage: StorageKind) -> VolumeInfo {
        VolumeInfo {
            id,
            fs,
            storage,
            caps: FsCaps::for_fs(fs),
        }
    }

    const TWO_PHASE: Schedule = Schedule::TwoPhase {
        order: Order::Extent,
    };
    use FsKind as F;
    use Schedule::Streaming;
    use StorageKind as S;

    type Row = (
        &'static str,
        Os,
        VolumeInfo,
        VolumeInfo,
        Op,
        &'static [Method],
        usize,
        Schedule,
    );

    /// One case per row of the selection matrix in issue #15 (cores = 8: SSD workers 16).
    #[test]
    fn selection_matrix() {
        #[rustfmt::skip]
        let rows: &[Row] = &[
            ("same volume move", Os::Linux, vol(1, F::Ext4, S::Ssd), vol(1, F::Ext4, S::Ssd), Op::Move, &[Rename], 1, Streaming),
            ("same volume move macOS", Os::Macos, vol(1, F::Apfs, S::Ssd), vol(1, F::Apfs, S::Ssd), Op::Move, &[Rename], 1, Streaming),
            ("same volume move Windows HDD", Os::Windows, vol(1, F::Ntfs, S::Hdd), vol(1, F::Ntfs, S::Hdd), Op::Move, &[Rename], 1, Streaming),
            ("linux btrfs clone SSD", Os::Linux, vol(1, F::Btrfs, S::Ssd), vol(1, F::Btrfs, S::Ssd), Op::Copy, &[Ficlone, CopyFileRange, ReadWrite], 16, Streaming),
            ("linux xfs clone HDD", Os::Linux, vol(1, F::Xfs, S::Hdd), vol(1, F::Xfs, S::Hdd), Op::Copy, &[Ficlone, CopyFileRange, ReadWrite], 4, Streaming),
            ("linux btrfs subvolumes", Os::Linux, vol(1, F::Btrfs, S::Ssd), vol(2, F::Btrfs, S::Ssd), Op::Copy, &[Ficlone, CopyFileRange, ReadWrite], 16, Streaming),
            ("linux ext4 same fs", Os::Linux, vol(1, F::Ext4, S::Ssd), vol(1, F::Ext4, S::Ssd), Op::Copy, &[CopyFileRange, ReadWrite], 16, Streaming),
            ("linux tmpfs unknown storage", Os::Linux, vol(1, F::Tmpfs, S::Unknown), vol(1, F::Tmpfs, S::Unknown), Op::Copy, &[CopyFileRange, ReadWrite], 16, Streaming),
            ("linux cross fs SSD", Os::Linux, vol(1, F::Ext4, S::Ssd), vol(2, F::Xfs, S::Ssd), Op::Copy, &[CopyFileRange, Sendfile, ReadWrite], 16, Streaming),
            ("linux to HDD", Os::Linux, vol(1, F::Ext4, S::Ssd), vol(2, F::Ext4, S::Hdd), Op::Copy, &[CopyFileRange, Sendfile, ReadWrite], 1, TWO_PHASE),
            ("linux HDD same disk", Os::Linux, vol(1, F::Ext4, S::Hdd), vol(1, F::Ext4, S::Hdd), Op::Copy, &[CopyFileRange, Sendfile, ReadWrite], 1, TWO_PHASE),
            ("linux nfs same server", Os::Linux, vol(1, F::Nfs, S::Network), vol(1, F::Nfs, S::Network), Op::Copy, &[CopyFileRange, ReadWrite], 16, Streaming),
            ("linux to network", Os::Linux, vol(1, F::Ext4, S::Ssd), vol(2, F::Fuse, S::Network), Op::Copy, &[CopyFileRange, ReadWrite], 16, Streaming),
            ("linux to USB", Os::Linux, vol(1, F::Ext4, S::Ssd), vol(2, F::Exfat, S::Removable), Op::Copy, &[CopyFileRange, Sendfile, ReadWrite], 2, Streaming),
            ("macOS same APFS volume", Os::Macos, vol(1, F::Apfs, S::Ssd), vol(1, F::Apfs, S::Ssd), Op::Copy, &[CloneFileAt, ReadWrite], 16, Streaming),
            ("macOS cross volume", Os::Macos, vol(1, F::Apfs, S::Ssd), vol(2, F::Apfs, S::Ssd), Op::Copy, &[ReadWrite], 16, Streaming),
            ("macOS to exFAT stick", Os::Macos, vol(1, F::Apfs, S::Ssd), vol(2, F::Exfat, S::Removable), Op::Copy, &[ReadWrite], 2, Streaming),
            ("macOS to HDD", Os::Macos, vol(1, F::Apfs, S::Ssd), vol(2, F::Hfs, S::Hdd), Op::Copy, &[ReadWrite], 1, TWO_PHASE),
            ("macOS network", Os::Macos, vol(1, F::Apfs, S::Ssd), vol(2, F::Smb, S::Network), Op::Copy, &[ReadWrite], 16, Streaming),
            ("windows ReFS same volume", Os::Windows, vol(1, F::Refs, S::Ssd), vol(1, F::Refs, S::Ssd), Op::Copy, &[CopyFile2, DuplicateExtents, ReadWrite], 16, Streaming),
            ("windows NTFS local", Os::Windows, vol(1, F::Ntfs, S::Ssd), vol(2, F::Ntfs, S::Ssd), Op::Copy, &[CopyFile2, ReadWrite], 16, Streaming),
            ("windows ReFS cross volume", Os::Windows, vol(1, F::Refs, S::Ssd), vol(2, F::Refs, S::Ssd), Op::Copy, &[CopyFile2, ReadWrite], 16, Streaming),
            ("windows NTFS HDD", Os::Windows, vol(1, F::Ntfs, S::Ssd), vol(2, F::Ntfs, S::Hdd), Op::Copy, &[CopyFile2, ReadWrite], 1, TWO_PHASE),
            ("windows SMB", Os::Windows, vol(1, F::Ntfs, S::Ssd), vol(2, F::Smb, S::Network), Op::Copy, &[CopyFile2, ReadWrite], 16, Streaming),
            ("windows USB", Os::Windows, vol(1, F::Ntfs, S::Ssd), vol(2, F::Fat, S::Removable), Op::Copy, &[CopyFile2, ReadWrite], 2, Streaming),
        ];
        for (name, os, src, dst, op, methods, workers, schedule) in rows {
            let plan = CopyPlan::for_pair_on(*os, src, dst, *op, 8);
            assert_eq!(plan.methods, *methods, "{name}: methods");
            assert_eq!(plan.workers, *workers, "{name}: workers");
            assert_eq!(plan.schedule, *schedule, "{name}: schedule");
        }
    }

    #[test]
    fn worker_bounds_and_buffers() {
        let ssd = vol(1, F::Ext4, S::Ssd);
        let other = vol(2, F::Ext4, S::Ssd);
        assert_eq!(
            CopyPlan::for_pair_on(Os::Linux, &ssd, &other, Op::Copy, 1).workers,
            4
        );
        assert_eq!(
            CopyPlan::for_pair_on(Os::Linux, &ssd, &other, Op::Copy, 64).workers,
            32
        );
        assert_eq!(
            CopyPlan::for_pair_on(Os::Windows, &ssd, &other, Op::Copy, 1).workers,
            8
        );
        assert_eq!(
            CopyPlan::for_pair_on(Os::Linux, &ssd, &other, Op::Copy, 0).workers,
            4
        );
        let hdd = vol(3, F::Ext4, S::Hdd);
        assert_eq!(
            CopyPlan::for_pair_on(Os::Linux, &ssd, &hdd, Op::Copy, 8).buf_size,
            8 * MIB
        );
        let usb = vol(4, F::Fat, S::Removable);
        assert_eq!(
            CopyPlan::for_pair_on(Os::Linux, &ssd, &usb, Op::Copy, 8).buf_size,
            4 * MIB
        );
        assert_eq!(
            CopyPlan::for_pair_on(Os::Linux, &ssd, &other, Op::Copy, 8).buf_size,
            MIB
        );
        let net = vol(5, F::Smb, S::Network);
        assert_eq!(
            CopyPlan::for_pair_on(Os::Windows, &ssd, &net, Op::Copy, 8).uncached_from,
            None
        );
    }

    #[test]
    fn cross_volume_move_is_durable_and_copy_is_not() {
        let (a, b) = (vol(1, F::Ext4, S::Ssd), vol(2, F::Ext4, S::Ssd));
        let mv = CopyPlan::for_pair_on(Os::Linux, &a, &b, Op::Move, 8);
        assert_eq!(mv.durability, Durability::BeforeSourceDelete);
        assert_eq!(mv.methods, LINUX_CROSS_FS);
        let cp = CopyPlan::for_pair_on(Os::Linux, &a, &b, Op::Copy, 8);
        assert_eq!(cp.durability, Durability::None);
        let exdev = CopyPlan::for_moved_data_on(Os::Linux, &a, &a, 8);
        assert_eq!(exdev.methods, LINUX_SAME_FS);
        assert_eq!(exdev.durability, Durability::BeforeSourceDelete);
    }

    #[test]
    fn overrides_replace_planned_values() {
        let a = vol(1, F::Ext4, S::Ssd);
        let plan = CopyPlan::for_pair_on(Os::Linux, &a, &a, Op::Copy, 8).with(&Overrides {
            workers: Some(0),
            method: Some(ReadWrite),
            schedule: Some(TWO_PHASE),
            storage: None,
        });
        assert_eq!(
            (plan.workers, plan.methods, plan.schedule),
            (1, &[ReadWrite][..], TWO_PHASE)
        );
    }

    #[test]
    fn method_names_round_trip_and_cache_disables() {
        for m in Method::ALL {
            assert_eq!(Method::from_name(m.name()), Some(m));
            assert_eq!(m.only(), &[m]);
        }
        let cache = MethodCache::default();
        assert!(cache.enabled(Ficlone));
        cache.disable(Ficlone);
        assert!(!cache.enabled(Ficlone) && cache.enabled(ReadWrite));
    }
}
