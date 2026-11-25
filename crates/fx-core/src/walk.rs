//! Use case: parallel recursive walk (names and kinds only), sized by the storage strategy.

use crate::list::{Backend, ListError};
use crate::listing::{EntryKind, Listing};
use crate::pool::run_in_pool;
use crate::storage::{StorageKind, Strategy};
use crate::sys;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
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
///
/// After a cancelled walk `dirs` counts folders *seen* in the listings that were read, which is
/// more than the folders *walked*; use [`WalkControl::dirs_read`] for the latter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WalkStats {
    pub files: u64,
    pub dirs: u64,
    pub symlinks: u64,
    pub others: u64,
    /// Subdirectories that could not be read.
    pub errors: u64,
}

impl WalkStats {
    fn count(&mut self, kind: EntryKind) {
        match kind {
            EntryKind::File => self.files += 1,
            EntryKind::Dir => self.dirs += 1,
            EntryKind::Symlink => self.symlinks += 1,
            EntryKind::Other => self.others += 1,
        }
    }
}

#[derive(Debug)]
pub struct Walked {
    pub stats: WalkStats,
    pub storage: StorageKind,
    pub strategy: Strategy,
}

type Enumerate = fn(&Path, &Strategy, bool, &mut Listing) -> io::Result<()>;

/// Cancels a walk and reports its progress, from any thread while the walk runs.
#[derive(Debug, Default)]
pub struct WalkControl {
    cancel: AtomicBool,
    dirs_read: AtomicU64,
    unreadable: AtomicU64,
    storage: OnceLock<StorageKind>,
}

impl WalkControl {
    /// Stops the walk from reading further directories.
    pub fn cancel(&self) {
        self.cancel.store(true, Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Relaxed)
    }

    /// Directories read so far, the root included. Exact for cancelled walks, unlike [`WalkStats::dirs`].
    pub fn dirs_read(&self) -> u64 {
        self.dirs_read.load(Relaxed)
    }

    /// The storage kind the walk detected (or was told) for its root; `None` until it has.
    pub fn storage(&self) -> Option<StorageKind> {
        self.storage.get().copied()
    }

    /// Subdirectories that could not be read so far.
    pub fn unreadable(&self) -> u64 {
        self.unreadable.load(Relaxed)
    }
}

/// Receives what a walk finds, from many threads, in no particular order.
pub trait WalkVisitor: Sync {
    /// Called once per readable directory, the root included.
    fn directory(&self, dir: &Path, listing: &Listing);
    /// Called once per subdirectory that could not be read.
    fn unreadable(&self, dir: &Path, err: &io::Error);
}

struct OnDirectory<F>(F);

impl<F: Fn(&Path, &Listing) + Sync> WalkVisitor for OnDirectory<F> {
    fn directory(&self, dir: &Path, listing: &Listing) {
        (self.0)(dir, listing);
    }

    fn unreadable(&self, _dir: &Path, _err: &io::Error) {}
}

#[derive(Default)]
struct Counters {
    files: AtomicU64,
    dirs: AtomicU64,
    symlinks: AtomicU64,
    others: AtomicU64,
    errors: AtomicU64,
}

impl Counters {
    fn add(&self, s: &WalkStats) {
        self.files.fetch_add(s.files, Relaxed);
        self.dirs.fetch_add(s.dirs, Relaxed);
        self.symlinks.fetch_add(s.symlinks, Relaxed);
        self.others.fetch_add(s.others, Relaxed);
    }
}

struct Walk<'a, V> {
    enumerate: Enumerate,
    strategy: Strategy,
    visitor: &'a V,
    control: &'a WalkControl,
    counters: Counters,
}

/// Walks `root` without following symlinks. `on_dir` runs once per readable directory
/// (root included), from many threads, in no particular order. Unsorted.
/// A root that cannot be read is an error; unreadable subdirectories only bump `errors`.
pub fn walk<F>(root: &Path, opts: &WalkOptions, on_dir: F) -> Result<Walked, ListError>
where
    F: Fn(&Path, &Listing) + Sync,
{
    walk_cancellable(root, opts, &WalkControl::default(), &OnDirectory(on_dir))
}

/// Like [`walk`], but reports unreadable subdirectories to `visitor`, stops reading directories
/// once `control` is cancelled and publishes live counts to it.
/// Returns normally; the stats cover what was visited before the stop.
pub fn walk_cancellable<V: WalkVisitor>(
    root: &Path,
    opts: &WalkOptions,
    control: &WalkControl,
    visitor: &V,
) -> Result<Walked, ListError> {
    // ponytail: storage detected once at root; per-mount re-detection when walks cross into network mounts.
    let storage = opts
        .storage
        .unwrap_or_else(|| sys::native::detect_storage(root));
    let _ = control.storage.set(storage);
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
        visitor,
        control,
        counters: Counters::default(),
    };
    let walk = &walk;
    run_in_pool(strategy.walk_threads, || {
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

impl<'a, V: WalkVisitor> Walk<'a, V> {
    fn visit<'s>(&'s self, scope: &rayon::Scope<'s>, dir: &Path, listing: &Listing) {
        if self.cancelled() {
            return;
        }
        self.control.dirs_read.fetch_add(1, Relaxed);
        self.visitor.directory(dir, listing);
        let mut tally = WalkStats::default();
        for e in &listing.entries {
            tally.count(e.kind);
            if e.kind != EntryKind::Dir {
                continue;
            }
            if self.cancelled() {
                break;
            }
            let path: PathBuf = dir.join(listing.name(e));
            scope.spawn(move |s| self.visit_dir(s, &path));
        }
        self.counters.add(&tally);
    }

    fn cancelled(&self) -> bool {
        self.control.is_cancelled()
    }

    fn visit_dir<'s>(&'s self, scope: &rayon::Scope<'s>, dir: &Path) {
        // ponytail: path-based; openat(parent_fd, name) on Linux is the upgrade path
        // (skips re-resolving the whole path per directory) if profiling shows path lookup dominates.
        if self.cancelled() {
            return;
        }
        let mut listing = Listing::default();
        match (self.enumerate)(dir, &self.strategy, false, &mut listing) {
            Ok(()) => self.visit(scope, dir, &listing),
            Err(e) => {
                log_unreadable(dir, &e);
                self.counters.errors.fetch_add(1, Relaxed);
                self.control.unreadable.fetch_add(1, Relaxed);
                self.visitor.unreadable(dir, &e);
            }
        }
    }
}

/// Missing or protected folders are routine during a walk; anything else is worth a warning.
fn log_unreadable(dir: &Path, err: &io::Error) {
    let level = match err.kind() {
        io::ErrorKind::PermissionDenied | io::ErrorKind::NotFound => log::Level::Debug,
        _ => log::Level::Warn,
    };
    log::log!(level, "cannot read {}: {err}", dir.display());
}
