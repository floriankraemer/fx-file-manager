//! Use case: parallel recursive walk (names and kinds only), sized by the storage strategy.

use crate::list::{Backend, ListError};
use crate::listing::{EntryKind, Listing};
use crate::pool::pool;
use crate::storage::{StorageKind, Strategy};
use crate::sys;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::thread;

#[derive(Clone, Copy, Debug)]
pub struct WalkOptions {
    pub backend: Backend,
    /// Overrides storage detection.
    pub storage: Option<StorageKind>,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            backend: Backend::Native,
            storage: None,
        }
    }
}

/// Counts of entries below the root (the root itself is not counted).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WalkStats {
    pub files: u64,
    pub dirs: u64,
    pub symlinks: u64,
    pub others: u64,
    /// Subdirectories that could not be read.
    pub errors: u64,
}

#[derive(Debug)]
pub struct Walked {
    pub stats: WalkStats,
    pub storage: StorageKind,
    pub strategy: Strategy,
}

type Enumerate = fn(&Path, &Strategy, bool, &mut Listing) -> io::Result<()>;

#[derive(Default)]
struct Counters {
    files: AtomicU64,
    dirs: AtomicU64,
    symlinks: AtomicU64,
    others: AtomicU64,
    errors: AtomicU64,
}

struct Walk<'a, F> {
    enumerate: Enumerate,
    strategy: Strategy,
    on_dir: &'a F,
    counters: Counters,
}

/// Walks `root` without following symlinks. `on_dir` runs once per readable directory
/// (root included), from many threads, in no particular order. Unsorted.
/// A root that cannot be read is an error; unreadable subdirectories only bump `errors`.
pub fn walk<F>(root: &Path, opts: &WalkOptions, on_dir: F) -> Result<Walked, ListError>
where
    F: Fn(&Path, &Listing) + Sync,
{
    // ponytail: storage detected once at root; per-mount re-detection when walks cross into network mounts.
    let storage = opts
        .storage
        .unwrap_or_else(|| sys::native::detect_storage(root));
    let cores = thread::available_parallelism().map_or(1, |n| n.get());
    let strategy = Strategy::for_kind(storage, cores);
    let enumerate: Enumerate = match opts.backend {
        Backend::Native => sys::native::enumerate,
        Backend::Std => sys::std_fs::enumerate,
    };

    let mut listing = Listing::default();
    enumerate(root, &strategy, false, &mut listing).map_err(|e| ListError::new(root, e))?;

    let walk = Walk {
        enumerate,
        strategy,
        on_dir: &on_dir,
        counters: Counters::default(),
    };
    let walk = &walk;
    pool(strategy.walk_threads).install(|| {
        rayon::scope(|s| walk.visit(s, root, &listing));
    });

    let c = &walk.counters;
    Ok(Walked {
        stats: WalkStats {
            files: c.files.load(Relaxed),
            dirs: c.dirs.load(Relaxed),
            symlinks: c.symlinks.load(Relaxed),
            others: c.others.load(Relaxed),
            errors: c.errors.load(Relaxed),
        },
        storage,
        strategy,
    })
}

impl<'a, F: Fn(&Path, &Listing) + Sync> Walk<'a, F> {
    fn visit<'s>(&'s self, scope: &rayon::Scope<'s>, dir: &Path, listing: &Listing) {
        (self.on_dir)(dir, listing);
        let c = &self.counters;
        for e in &listing.entries {
            match e.kind {
                EntryKind::File => c.files.fetch_add(1, Relaxed),
                EntryKind::Symlink => c.symlinks.fetch_add(1, Relaxed),
                EntryKind::Other => c.others.fetch_add(1, Relaxed),
                EntryKind::Dir => {
                    c.dirs.fetch_add(1, Relaxed);
                    let path: PathBuf = dir.join(listing.name(e));
                    scope.spawn(move |s| self.visit_dir(s, &path));
                    continue;
                }
            };
        }
    }

    fn visit_dir<'s>(&'s self, scope: &rayon::Scope<'s>, dir: &Path) {
        // ponytail: path-based; openat(parent_fd, name) on Linux is the upgrade path
        // (skips re-resolving the whole path per directory) if profiling shows path lookup dominates.
        let mut listing = Listing::default();
        match (self.enumerate)(dir, &self.strategy, false, &mut listing) {
            Ok(()) => self.visit(scope, dir, &listing),
            Err(_) => {
                self.counters.errors.fetch_add(1, Relaxed);
            }
        }
    }
}
