//! Storage classification and the concurrency strategy derived from it.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StorageKind {
    Ssd,
    Hdd,
    Network,
    /// USB sticks and other hot-pluggable media.
    Removable,
    Unknown,
}

impl std::fmt::Display for StorageKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Ssd => "ssd",
            Self::Hdd => "hdd",
            Self::Network => "network",
            Self::Removable => "removable",
            Self::Unknown => "unknown",
        })
    }
}

const KIB: usize = 1024;
/// In-flight requests that hide network latency without overloading the server.
const NETWORK_IN_FLIGHT: usize = 8;
const HDD_WALK_THREADS: usize = 2;
/// USB flash has little internal parallelism; more requests only queue up.
const REMOVABLE_THREADS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strategy {
    pub stat_threads: usize,
    pub walk_threads: usize,
    /// Sort stat calls by inode to cut seeks.
    pub inode_order: bool,
    pub buf_size: usize,
}

impl Strategy {
    /// Pure: `cores` is passed in so the result is deterministic. `Unknown` behaves like `Ssd`.
    pub fn for_kind(kind: StorageKind, cores: usize) -> Self {
        let cores = cores.max(1);
        match kind {
            StorageKind::Ssd | StorageKind::Unknown => Self {
                stat_threads: cores,
                walk_threads: cores,
                inode_order: false,
                buf_size: 64 * KIB,
            },
            StorageKind::Hdd => Self {
                stat_threads: 1,
                walk_threads: HDD_WALK_THREADS,
                inode_order: true,
                buf_size: 64 * KIB,
            },
            StorageKind::Removable => Self {
                stat_threads: REMOVABLE_THREADS,
                walk_threads: REMOVABLE_THREADS,
                inode_order: false,
                buf_size: 64 * KIB,
            },
            StorageKind::Network => Self {
                stat_threads: NETWORK_IN_FLIGHT,
                walk_threads: NETWORK_IN_FLIGHT,
                inode_order: false,
                buf_size: 256 * KIB,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategy_per_kind() {
        let ssd = Strategy::for_kind(StorageKind::Ssd, 16);
        assert_eq!(
            (ssd.stat_threads, ssd.walk_threads, ssd.inode_order),
            (16, 16, false)
        );
        assert_eq!(Strategy::for_kind(StorageKind::Unknown, 16), ssd);

        let hdd = Strategy::for_kind(StorageKind::Hdd, 16);
        assert_eq!(
            (hdd.stat_threads, hdd.walk_threads, hdd.inode_order),
            (1, 2, true)
        );

        let net = Strategy::for_kind(StorageKind::Network, 16);
        assert_eq!((net.stat_threads, net.buf_size), (8, 256 * 1024));

        let usb = Strategy::for_kind(StorageKind::Removable, 16);
        assert_eq!((usb.stat_threads, usb.walk_threads), (2, 2));
    }

    #[test]
    fn zero_cores_still_has_a_thread() {
        assert_eq!(Strategy::for_kind(StorageKind::Ssd, 0).stat_threads, 1);
    }
}
