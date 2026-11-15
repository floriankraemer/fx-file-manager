//! The TwoPhase schedule: walk and create the whole tree first, then copy files one at a time in disk
//! order, batching the reads of small files on a shared spindle.

use super::dirs::{Entry, Fds, OpenParents};
use super::entry::Prefetched;
use super::preflight::Top;
use super::{Job, os};
use crate::EntryKind;
use crate::copy::Meta;
use crate::copy::plan::Order;
use crate::copy::progress::{ErrorOp, Progress};
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// TwoPhase same-spindle batching: read up to this much (or this many files) before writing any of it.
const BATCH_BYTES: u64 = 64 * 1024 * 1024;
const BATCH_FILES: usize = 256;
/// Larger files are streamed, not batched.
const BATCH_FILE_MAX: u64 = 8 * 1024 * 1024;
/// Open directory pairs kept by the TwoPhase copy loop.
const DIR_CACHE: usize = 64;

/// A file found by the walk, in the folder pair `dirs[dir]`.
struct File {
    dir: usize,
    name: OsString,
    dst_name: OsString,
    kind: EntryKind,
    size: u64,
    ino: u64,
    physical: Option<u64>,
    item: usize,
    keep_both: bool,
}

impl File {
    fn top(dir: usize, top: &Top) -> Self {
        Self::new(
            dir,
            top.name.clone(),
            top.dst_name.clone(),
            top.kind,
            top.item,
            top.keep_both,
        )
    }

    fn child(dir: usize, name: OsString, kind: EntryKind, item: usize) -> Self {
        Self::new(dir, name.clone(), name, kind, item, false)
    }

    fn new(
        dir: usize,
        name: OsString,
        dst_name: OsString,
        kind: EntryKind,
        item: usize,
        keep_both: bool,
    ) -> Self {
        Self {
            dir,
            name,
            dst_name,
            kind,
            size: 0,
            ino: 0,
            physical: None,
            item,
            keep_both,
        }
    }

    fn entry<'e>(&'e self, fds: &'e Fds) -> Entry<'e> {
        Entry {
            src_dir: &fds.src,
            name: &self.name,
            kind: self.kind,
            dst_dir: &fds.dst,
            dst_name: &self.dst_name,
            keep_both: self.keep_both,
        }
    }

    fn batchable(&self) -> bool {
        self.kind == EntryKind::File && !self.keep_both && self.size <= BATCH_FILE_MAX
    }
}

/// Everything the walk found: folder pairs, folders that get their metadata last, and files.
struct Walk {
    order: Order,
    /// Folders found but not created and listed yet.
    pending: Vec<PendingDir>,
    dirs: Vec<(PathBuf, PathBuf)>,
    finals: Vec<(PathBuf, Meta)>,
    files: Vec<File>,
}

impl Walk {
    fn new(order: Order) -> Self {
        Self {
            order,
            pending: Vec::new(),
            dirs: Vec::new(),
            finals: Vec::new(),
            files: Vec::new(),
        }
    }

    fn add_dir(&mut self, fds: &Fds) -> usize {
        self.dirs
            .push((fds.src.path().to_path_buf(), fds.dst.path().to_path_buf()));
        self.dirs.len() - 1
    }

    /// Stats the file (and finds its disk position for extent order) and counts it in the totals.
    fn add_file(&mut self, progress: &Progress, fds: &Fds, mut file: File) {
        let (meta, physical) = probe(&fds.src, &file.name, file.kind, self.order);
        (file.size, file.ino) = meta.map_or((0, 0), |m| (m.size, m.ino));
        file.physical = physical;
        Progress::add(&progress.files_total, 1);
        Progress::add(&progress.bytes_total, file.size);
        self.files.push(file);
    }

    /// Queues a subfolder, or adds a file of folder pair `dir`.
    fn add_child(
        &mut self,
        progress: &Progress,
        fds: &Arc<Fds>,
        dir: usize,
        child: OsString,
        kind: EntryKind,
        item: usize,
    ) {
        if kind != EntryKind::Dir {
            self.add_file(progress, fds, File::child(dir, child, kind, item));
            return;
        }
        self.pending.push(PendingDir {
            parent: Arc::clone(fds),
            name: child.clone(),
            dst_name: child,
            keep_both: false,
        });
    }

    /// Disk order where the filesystem reports extents, else inode order (as `stat_all` does).
    fn sort_by_disk_order(&mut self) {
        if self.files.iter().any(|f| f.physical.is_some()) {
            self.files.sort_by_key(|f| f.physical.unwrap_or(0));
        } else {
            self.files.sort_by_key(|f| f.ino);
        }
    }
}

fn probe(
    dir: &os::Dir,
    name: &OsStr,
    kind: EntryKind,
    order: Order,
) -> (Option<Meta>, Option<u64>) {
    if kind != EntryKind::File || order != Order::Extent {
        return (dir.stat_at(name).ok(), None);
    }
    match os::open_file(dir, name) {
        Ok(src) => (Some(*src.meta()), os::extent_key(&src)),
        Err(_) => (None, None),
    }
}

/// A folder still to be created and listed by the walk.
struct PendingDir {
    parent: Arc<Fds>,
    name: OsString,
    dst_name: OsString,
    keep_both: bool,
}

/// Small files read ahead on a shared spindle, written out together.
#[derive(Default)]
struct Batch {
    files: Vec<(Arc<Fds>, usize, Prefetched)>,
    bytes: u64,
}

/// Recently used folder pairs of the copy loop, bounded so a big tree cannot exhaust descriptors.
struct DirCache<'w> {
    dirs: &'w [(PathBuf, PathBuf)],
    open: HashMap<usize, Arc<Fds>>,
}

impl Job<'_> {
    /// Walk and create the whole tree, then copy files one at a time in disk order.
    pub(super) fn two_phase(&self, tops: &[Top], order: Order, same_volume: bool) {
        let Some(mut walk) = self.walk(tops, order) else {
            return;
        };
        if self.last_group {
            self.sh
                .progress
                .walk_complete
                .store(true, Ordering::Release);
        }
        if let Some(top) = tops.first() {
            self.check_space(&top.dst_parent);
        }
        walk.sort_by_disk_order();
        self.copy_in_order(&walk, same_volume);
        for (path, meta) in walk.finals.iter().rev() {
            self.finish_dir(path, meta);
        }
    }

    /// `None` when cancelled during the walk.
    fn walk(&self, tops: &[Top], order: Order) -> Option<Walk> {
        let mut walk = Walk::new(order);
        let mut parents = OpenParents::new();
        for top in tops {
            if self.sh.is_cancelled() {
                return None;
            }
            let Some(fds) = self.open_top(top, &mut parents) else {
                continue;
            };
            let dir = walk.add_dir(&fds);
            if top.kind == EntryKind::Dir {
                self.walk_tree(&mut walk, fds, top);
            } else {
                walk.add_file(&self.sh.progress, &fds, File::top(dir, top));
            }
        }
        Some(walk)
    }

    /// Creates every folder of a top-level folder item, depth first, and collects its files.
    fn walk_tree(&self, walk: &mut Walk, fds: Arc<Fds>, top: &Top) {
        let item = &self.items[top.item];
        walk.pending.push(PendingDir {
            parent: fds,
            name: top.name.clone(),
            dst_name: top.dst_name.clone(),
            keep_both: top.keep_both,
        });
        while let Some(next) = walk.pending.pop() {
            let opened = self.open_dir_pair(
                &next.parent,
                &next.name,
                &next.dst_name,
                next.keep_both,
                item,
            );
            let Some((fds, meta)) = opened else {
                continue;
            };
            if let Some(meta) = meta {
                walk.finals.push((fds.dst.path().to_path_buf(), meta));
            }
            let dir = walk.add_dir(&fds);
            let entries = self.list(&fds, item);
            let fds = Arc::new(fds);
            for (child, kind) in entries {
                walk.add_child(&self.sh.progress, &fds, dir, child, kind, top.item);
            }
        }
    }

    fn copy_in_order(&self, walk: &Walk, same_volume: bool) {
        let mut dirs = DirCache {
            dirs: &walk.dirs,
            open: HashMap::new(),
        };
        let mut batch = Batch::default();
        for (fi, f) in walk.files.iter().enumerate() {
            if self.sh.is_cancelled() {
                break;
            }
            let Some(fds) = self.cached_dir(&mut dirs, f) else {
                continue;
            };
            if same_volume
                && f.batchable()
                && let Some(prefetched) = prefetch(&fds.src, &f.name)
            {
                self.add_to_batch(&mut batch, fds, fi, prefetched, &walk.files);
                continue;
            }
            self.flush(&mut batch, &walk.files);
            self.copy_entry(f.entry(&fds), &self.items[f.item], None, true);
        }
        self.flush(&mut batch, &walk.files);
    }

    fn cached_dir(&self, cache: &mut DirCache<'_>, f: &File) -> Option<Arc<Fds>> {
        if let Some(fds) = cache.open.get(&f.dir) {
            return Some(Arc::clone(fds));
        }
        if cache.open.len() >= DIR_CACHE {
            cache.open.clear();
        }
        let (src, dst) = &cache.dirs[f.dir];
        let opened = Fds::open(src, dst);
        let fds = Arc::new(self.ok_or_fail(&self.items[f.item], src, ErrorOp::List, opened)?);
        cache.open.insert(f.dir, Arc::clone(&fds));
        Some(fds)
    }

    fn add_to_batch(
        &self,
        batch: &mut Batch,
        fds: Arc<Fds>,
        fi: usize,
        prefetched: Prefetched,
        files: &[File],
    ) {
        batch.bytes += prefetched.1.len() as u64;
        batch.files.push((fds, fi, prefetched));
        if batch.bytes >= BATCH_BYTES || batch.files.len() >= BATCH_FILES {
            self.flush(batch, files);
        }
    }

    fn flush(&self, batch: &mut Batch, files: &[File]) {
        batch.bytes = 0;
        for (fds, fi, prefetched) in batch.files.drain(..) {
            let f = &files[fi];
            self.copy_entry(f.entry(&fds), &self.items[f.item], Some(prefetched), true);
        }
    }
}

/// Reads a small file whole, ahead of its copy.
fn prefetch(dir: &os::Dir, name: &OsStr) -> Option<Prefetched> {
    let src = os::open_file(dir, name).ok()?;
    let mut data = Vec::new();
    os::read_all(&src, &mut data).ok()?;
    Some((src, data))
}
