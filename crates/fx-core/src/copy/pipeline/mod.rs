//! Scheduling: pre-flight, same-volume renames, the Streaming and TwoPhase schedules, conflicts,
//! hardlinks, retries, post-order directory metadata and the move barrier.
//!
//! `run` drives one request through its phases; a [`Job`] executes one group of items that share a
//! plan. The submodules each own one concern of a job.

mod backend;
mod conflicts;
mod dirs;
mod entry;
mod errors;
mod links;
mod moves;
mod preflight;
mod streaming;
mod two_phase;

pub(crate) use backend::{Copied, FileCtx, Meter};
pub(crate) use errors::is_unsupported;

use crate::copy::conflict::ConflictPolicy;
use crate::copy::plan::{CopyPlan, Durability, MethodCache, Op, Os, Schedule};
use crate::copy::progress::{ErrorOp, Progress, Shared, is_cancelled};
use crate::copy::{Meta, Request, Summary};
use crate::sys::copy as os;
use crate::volume::{FsCaps, VolumeInfo};
use links::LinkGroups;
use preflight::{Parents, Top};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

pub(crate) fn run(req: &Request, sh: &Shared) -> Summary {
    os::raise_fd_limit();
    let cores = thread::available_parallelism().map_or(1, |n| n.get());
    let items: Vec<Arc<ItemState>> = req.items.iter().map(|_| Arc::default()).collect();
    let (tops, prepared) = prepare_all(req, sh, &items, cores);
    let copies = rename_same_volume(req, sh, &items, tops, cores);
    let groups = group_by_plan(copies);
    let kept_sources = copy_groups(req, sh, &items, &groups);
    sh.progress.walk_complete.store(true, Ordering::Release);
    summarize(req, sh, &items, &prepared, &groups, kept_sources)
}

/// Pre-flight checks and planning for every item. The flags mark the items that passed.
fn prepare_all(
    req: &Request,
    sh: &Shared,
    items: &[Arc<ItemState>],
    cores: usize,
) -> (Vec<Top>, Vec<bool>) {
    let mut prepared = vec![false; req.items.len()];
    let mut parents = Parents::default();
    let mut tops = Vec::new();
    for (i, item) in req.items.iter().enumerate() {
        if sh.is_cancelled() {
            break;
        }
        match preflight::prepare(req, i, &item.src, &item.dst, cores, &mut parents) {
            // `None`: moving an item onto itself, nothing to do.
            Ok(top) => {
                prepared[i] = true;
                tops.extend(top);
            }
            Err(e) => {
                sh.record(&item.src, ErrorOp::Preflight, &e, false);
                items[i].errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    (tops, prepared)
}

/// Same-volume moves: one rename per item, whatever the tree size. Returns the items left to copy.
fn rename_same_volume(
    req: &Request,
    sh: &Shared,
    items: &[Arc<ItemState>],
    tops: Vec<Top>,
    cores: usize,
) -> Vec<Top> {
    let mut copies = Vec::new();
    for top in tops {
        if !top.plan.renames() {
            copies.push(top);
            continue;
        }
        let job = Job::new(sh, req, items, top.plan, &top.dst_vol, false);
        match job.rename_top(&top) {
            Ok(()) => {}
            // Bind mounts and btrfs subvolumes share st_dev semantics only partly: copy instead.
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                copies.push(top.into_copy(req, cores));
            }
            Err(e) => job.fail(top.item, &top.src_path(), ErrorOp::Rename, &e),
        }
    }
    copies
}

/// Items that share a (source volume, destination volume, plan) run as one job.
struct Group {
    plan: CopyPlan,
    src_vol: VolumeInfo,
    dst_vol: VolumeInfo,
    tops: Vec<Top>,
}

impl Group {
    fn accepts(&self, top: &Top) -> bool {
        self.plan == top.plan
            && self.src_vol.id == top.src_vol.id
            && self.dst_vol.id == top.dst_vol.id
    }
}

/// One group per distinct (pair, plan): usually exactly one.
fn group_by_plan(copies: Vec<Top>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for top in copies {
        match groups.iter_mut().find(|g| g.accepts(&top)) {
            Some(group) => group.tops.push(top),
            None => groups.push(Group {
                plan: top.plan,
                src_vol: top.src_vol,
                dst_vol: top.dst_vol,
                tops: vec![top],
            }),
        }
    }
    groups
}

/// Runs each group's schedule; returns the move sources that were kept.
fn copy_groups(
    req: &Request,
    sh: &Shared,
    items: &[Arc<ItemState>],
    groups: &[Group],
) -> Vec<PathBuf> {
    let last = groups.len().saturating_sub(1);
    let mut kept_sources = Vec::new();
    for (gi, group) in groups.iter().enumerate() {
        let mut job = Job::new(sh, req, items, group.plan, &group.dst_vol, gi == last);
        job.roots = group.tops.iter().map(Top::src_path).collect();
        job.run_schedule(group);
        if group.plan.durability == Durability::BeforeSourceDelete {
            kept_sources.extend(job.delete_moved_sources(&group.tops, &req.items));
        }
    }
    kept_sources
}

fn summarize(
    req: &Request,
    sh: &Shared,
    items: &[Arc<ItemState>],
    prepared: &[bool],
    groups: &[Group],
    kept_sources: Vec<PathBuf>,
) -> Summary {
    let cancelled = sh.is_cancelled();
    let snapshot = sh.snapshot();
    let items_done = if cancelled {
        0
    } else {
        items
            .iter()
            .zip(prepared)
            .filter(|(state, ok)| **ok && state.clean())
            .count()
    };
    Summary {
        op: req.op,
        items: req.items.len(),
        items_done,
        files: snapshot.files_done,
        bytes: snapshot.bytes_done,
        cloned: snapshot.cloned,
        skipped: snapshot.skipped,
        errors: sh.report(),
        cancelled,
        kept_sources,
        plans: groups.iter().map(|g| g.plan).collect(),
        elapsed: snapshot.elapsed,
    }
}

/// Per top-level item: a move deletes its source only if both stay zero.
#[derive(Default)]
struct ItemState {
    errors: AtomicU64,
    skipped: AtomicU64,
}

impl ItemState {
    fn clean(&self) -> bool {
        self.errors.load(Ordering::Relaxed) == 0 && self.skipped.load(Ordering::Relaxed) == 0
    }
}

/// One group of items copied with one plan: the state its schedule and workers share.
struct Job<'a> {
    sh: &'a Shared,
    plan: CopyPlan,
    cache: MethodCache,
    caps: FsCaps,
    policy: ConflictPolicy,
    /// First destination of each multiply-linked source inode, so later names become hardlinks.
    links: LinkGroups,
    /// Directories scheduled but not listed yet; the walk is complete when it drops to zero.
    dirs_pending: AtomicUsize,
    last_group: bool,
    items: &'a [Arc<ItemState>],
    op: Op,
    /// Move sources whose copy got another name (keep both), so verification checks the right file.
    renamed: Mutex<HashMap<PathBuf, PathBuf>>,
    /// One destination folder per device written to, each flushed by the move barrier.
    dst_devices: Mutex<HashMap<u64, PathBuf>>,
    /// Source path of each top-level item of the job, for links whose target lies inside the copied tree.
    roots: Vec<PathBuf>,
}

impl<'a> Job<'a> {
    fn new(
        sh: &'a Shared,
        req: &Request,
        items: &'a [Arc<ItemState>],
        plan: CopyPlan,
        dst: &VolumeInfo,
        last_group: bool,
    ) -> Self {
        Self {
            sh,
            plan,
            cache: MethodCache::default(),
            caps: dst.caps,
            policy: req.policy,
            links: LinkGroups::default(),
            dirs_pending: AtomicUsize::new(0),
            last_group,
            items,
            op: req.op,
            renamed: Mutex::default(),
            dst_devices: Mutex::default(),
            roots: Vec::new(),
        }
    }

    fn run_schedule(&self, group: &Group) {
        match group.plan.schedule {
            Schedule::Streaming => self.streaming(&group.tops),
            Schedule::TwoPhase { order } => {
                self.two_phase(
                    &group.tops,
                    order,
                    group.src_vol.same_volume(&group.dst_vol),
                );
            }
        }
    }

    fn ctx(&self) -> FileCtx<'_> {
        FileCtx {
            plan: &self.plan,
            cache: &self.cache,
            shared: self.sh,
            caps: self.caps,
            // Linux flushes the whole destination with one syncfs at the barrier instead.
            durable: self.plan.durability == Durability::BeforeSourceDelete
                && Os::CURRENT != Os::Linux,
        }
    }

    fn fail_state(&self, item: &ItemState, path: &Path, op: ErrorOp, e: &io::Error) {
        if is_cancelled(e) {
            return;
        }
        item.errors.fetch_add(1, Ordering::Relaxed);
        self.sh.record(path, op, e, false);
    }

    fn fail(&self, item: usize, path: &Path, op: ErrorOp, e: &io::Error) {
        self.fail_state(&self.items[item], path, op, e);
    }

    /// The value, or `None` after the error was recorded against `item`.
    fn ok_or_fail<T>(
        &self,
        item: &ItemState,
        path: &Path,
        op: ErrorOp,
        result: io::Result<T>,
    ) -> Option<T> {
        result
            .inspect_err(|e| self.fail_state(item, path, op, e))
            .ok()
    }

    fn skip(&self, item: &ItemState) {
        item.skipped.fetch_add(1, Ordering::Relaxed);
        Progress::add(&self.sh.progress.skipped, 1);
    }

    fn warn(&self, path: &Path, op: ErrorOp, warnings: Vec<io::Error>) {
        for w in warnings {
            self.sh.record(path, op, &w, true);
        }
    }

    fn finish_dir(&self, path: &Path, meta: &Meta) {
        if let Err(e) = os::finish_dir(path, meta, &self.caps) {
            self.sh.record(path, ErrorOp::Metadata, &e, true);
        }
    }

    fn dir_listed(&self) {
        if self.dirs_pending.fetch_sub(1, Ordering::AcqRel) == 1 && self.last_group {
            self.sh
                .progress
                .walk_complete
                .store(true, Ordering::Release);
        }
    }
}

/// The same file or folder reached by two paths (bind mounts, hardlinks, aliases). Windows reports no
/// inode here, so it relies on the canonical-path comparison in `prepare`.
fn same_inode(a: &Meta, b: &Meta) -> bool {
    a.ino != 0 && a.dev == b.dev && a.ino == b.ino
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}
