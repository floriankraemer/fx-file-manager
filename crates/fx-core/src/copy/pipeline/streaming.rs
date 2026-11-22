//! The Streaming schedule: the walk and the copy run together on the pool, folder by folder, and
//! each folder gets its final mode and times once all its children are done.

use super::dirs::{Entry, Fds, OpenParents, Pair};
use super::preflight::Top;
use super::{ItemState, Job};
use crate::EntryKind;
use crate::copy::Meta;
use crate::copy::progress::{ErrorOp, Progress};
use crate::pool::copy_pool;
use std::cell::Cell;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Files handed to one task; amortises scheduling for tiny files.
const FILES_PER_TASK: usize = 64;
/// A task that has copied this much hands its remaining files to a new task, so large files spread out.
const TASK_BYTES: u64 = 8 * 1024 * 1024;

type Listing = Vec<(OsString, EntryKind)>;

/// Where a directory task finds its parent: the shared top-level folders stay open for the whole job.
enum Parent {
    Open(Arc<Fds>),
    Closed(Arc<Pair>),
}

impl Parent {
    fn open(self) -> io::Result<Arc<Fds>> {
        match self {
            Parent::Open(fds) => Ok(fds),
            Parent::Closed(pair) => pair.open().map(Arc::new),
        }
    }
}

/// A destination directory waiting for its children before its final mode and times are set.
struct Node {
    /// `None` for a folder that existed already: merged into, its metadata is left alone.
    meta: Option<Meta>,
    src_path: PathBuf,
    dst_path: PathBuf,
    /// The listing itself plus every child task still running.
    pending: AtomicUsize,
    parent: Option<Arc<Node>>,
    item: Arc<ItemState>,
}

/// One folder to visit: `name` under `parent` becomes `dst_name`.
struct Visit<'n> {
    parent: Parent,
    parent_node: Option<Arc<Node>>,
    name: &'n OsStr,
    dst_name: &'n OsStr,
    item: Arc<ItemState>,
    keep_both: bool,
}

impl Job<'_> {
    pub(super) fn streaming(&self, tops: &[Top]) {
        self.early_space_check(tops);
        copy_pool(self.plan.workers).install(|| rayon::scope(|s| self.spawn_tops(s, tops)));
    }

    fn spawn_tops<'s>(&'s self, s: &rayon::Scope<'s>, tops: &'s [Top]) {
        // Held while top-level items are scheduled, so the walk cannot look complete too early.
        self.dirs_pending.fetch_add(1, Ordering::AcqRel);
        let mut parents = OpenParents::new();
        for top in tops {
            let Some(fds) = self.open_top(top, &mut parents) else {
                continue;
            };
            self.spawn_top(s, top, fds);
        }
        self.dir_listed();
    }

    fn spawn_top<'s>(&'s self, s: &rayon::Scope<'s>, top: &'s Top, fds: Arc<Fds>) {
        let item = Arc::clone(&self.items[top.item]);
        if top.kind == EntryKind::Dir {
            self.dirs_pending.fetch_add(1, Ordering::AcqRel);
            s.spawn(move |s| {
                let visit = Visit {
                    parent: Parent::Open(fds),
                    parent_node: None,
                    name: &top.name,
                    dst_name: &top.dst_name,
                    item,
                    keep_both: top.keep_both,
                };
                self.visit_dir(s, visit);
            });
            return;
        }
        Progress::add(&self.sh.progress.files_total, 1);
        s.spawn(move |_| {
            let e = Entry {
                src_dir: &fds.src,
                name: &top.name,
                kind: top.kind,
                dst_dir: &fds.dst,
                dst_name: &top.dst_name,
                keep_both: top.keep_both,
                answer: Cell::default(),
            };
            self.copy_entry(e, &item, None, false);
        });
    }

    /// Creates one folder, spawns a visit per subfolder and copy tasks for its files.
    fn visit_dir<'s>(&'s self, s: &rayon::Scope<'s>, visit: Visit<'_>) {
        let Visit {
            parent,
            parent_node,
            name,
            dst_name,
            item,
            keep_both,
        } = visit;
        let opened = self.open_visit(
            parent,
            parent_node.as_deref(),
            name,
            dst_name,
            keep_both,
            &item,
        );
        let Some((fds, meta)) = opened else {
            self.dir_listed();
            if let Some(parent) = parent_node {
                self.node_done(parent);
            }
            return;
        };
        let node = Arc::new(Node {
            meta,
            src_path: fds.src.path().to_path_buf(),
            dst_path: fds.dst.path().to_path_buf(),
            pending: AtomicUsize::new(1),
            parent: parent_node,
            item,
        });
        let entries = self.list(&fds, &node.item);
        let pair = Pair::of(&fds);
        drop(fds);
        let files = self.spawn_subdirs(s, &pair, &node, entries);
        Progress::add(&self.sh.progress.files_total, files.len() as u64);
        self.spawn_batches(s, &pair, &node, files);
        self.dir_listed();
        self.node_done(node);
    }

    /// Reopens the parent if it was closed, then opens and creates the folder itself.
    fn open_visit(
        &self,
        parent: Parent,
        parent_node: Option<&Node>,
        name: &OsStr,
        dst_name: &OsStr,
        keep_both: bool,
        item: &ItemState,
    ) -> Option<(Fds, Option<Meta>)> {
        let parent = match parent.open() {
            Ok(fds) => fds,
            Err(e) => {
                let src_parent = parent_node.map(|n| n.src_path.clone()).unwrap_or_default();
                self.fail_state(item, &src_parent.join(name), ErrorOp::List, &e);
                return None;
            }
        };
        self.open_dir_pair(&parent, name, dst_name, keep_both, item)
    }

    /// Spawns a visit for each subfolder; returns the other entries.
    fn spawn_subdirs<'s>(
        &'s self,
        s: &rayon::Scope<'s>,
        pair: &Arc<Pair>,
        node: &Arc<Node>,
        entries: Listing,
    ) -> Listing {
        let mut files = Vec::new();
        for (child, kind) in entries {
            if kind != EntryKind::Dir {
                files.push((child, kind));
                continue;
            }
            self.dirs_pending.fetch_add(1, Ordering::AcqRel);
            node.pending.fetch_add(1, Ordering::AcqRel);
            let (pair, node) = (Arc::clone(pair), Arc::clone(node));
            s.spawn(move |s| {
                let visit = Visit {
                    parent: Parent::Closed(pair),
                    item: Arc::clone(&node.item),
                    parent_node: Some(node),
                    name: &child,
                    dst_name: &child,
                    keep_both: false,
                };
                self.visit_dir(s, visit);
            });
        }
        files
    }

    fn spawn_batches<'s>(
        &'s self,
        s: &rayon::Scope<'s>,
        pair: &Arc<Pair>,
        node: &Arc<Node>,
        mut files: Listing,
    ) {
        while !files.is_empty() {
            let rest = files.split_off(files.len().saturating_sub(FILES_PER_TASK));
            self.spawn_batch(s, pair, node, rest);
        }
    }

    fn spawn_batch<'s>(
        &'s self,
        s: &rayon::Scope<'s>,
        pair: &Arc<Pair>,
        node: &Arc<Node>,
        files: Listing,
    ) {
        node.pending.fetch_add(1, Ordering::AcqRel);
        let (pair, node) = (Arc::clone(pair), Arc::clone(node));
        s.spawn(move |s| self.copy_batch(s, pair, node, files));
    }

    fn copy_batch<'s>(
        &'s self,
        s: &rayon::Scope<'s>,
        pair: Arc<Pair>,
        node: Arc<Node>,
        files: Listing,
    ) {
        match pair.open() {
            Ok(fds) => self.copy_files(s, &fds, &pair, &node, files),
            Err(e) => self.fail_state(&node.item, &pair.src, ErrorOp::List, &e),
        }
        self.node_done(node);
    }

    fn copy_files<'s>(
        &'s self,
        s: &rayon::Scope<'s>,
        fds: &Fds,
        pair: &Arc<Pair>,
        node: &Arc<Node>,
        files: Listing,
    ) {
        let mut bytes = 0;
        let mut rest = files.into_iter();
        while let Some((name, kind)) = rest.next() {
            bytes += self.copy_entry(Entry::child(fds, &name, kind), &node.item, None, false);
            if bytes >= TASK_BYTES && rest.len() > 0 {
                self.spawn_batch(s, pair, node, rest.collect());
                return;
            }
        }
    }

    /// Marks one unit of `node` done; the last one applies the final mode and times, bottom-up.
    fn node_done(&self, node: Arc<Node>) {
        let mut current = Some(node);
        while let Some(node) = current {
            if node.pending.fetch_sub(1, Ordering::AcqRel) != 1 {
                return;
            }
            if let Some(meta) = &node.meta {
                self.finish_dir(&node.dst_path, meta);
            }
            current = node.parent.clone();
        }
    }
}
