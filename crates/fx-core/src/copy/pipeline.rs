//! Scheduling: pre-flight, same-volume renames, the Streaming and TwoPhase schedules, conflicts,
//! hardlinks, retries, post-order directory metadata and the move barrier.

use super::conflict::{self, Conflict, ConflictPolicy, Resolution, keep_both_name, temp_name};
use super::plan::{CopyPlan, Durability, MethodCache, Op, Order, Os, Schedule};
use super::progress::{DiskFull, ErrorOp, Progress, Shared, is_cancelled};
use super::{Meta, Request, Summary};
use crate::EntryKind;
use crate::pool::copy_pool;
use crate::sys::copy as os;
use crate::volume::{FsCaps, VolumeInfo, volume_info};
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Files handed to one task; amortises scheduling for tiny files.
const FILES_PER_TASK: usize = 64;
/// A task that has copied this much hands its remaining files to a new task, so large files spread out.
const TASK_BYTES: u64 = 8 * 1024 * 1024;
/// Transient network errors restart the file after these pauses.
const RETRY_BACKOFF: [Duration; 3] = [
    Duration::from_millis(250),
    Duration::from_secs(1),
    Duration::from_secs(4),
];
/// TwoPhase same-spindle batching: read up to this much (or this many files) before writing any of it.
const BATCH_BYTES: u64 = 64 * 1024 * 1024;
const BATCH_FILES: usize = 256;
/// Larger files are streamed, not batched.
const BATCH_FILE_MAX: u64 = 8 * 1024 * 1024;
/// Open directory pairs kept by the TwoPhase copy loop.
const DIR_CACHE: usize = 64;

/// What the OS backend needs to copy one file.
pub(crate) struct FileCtx<'a> {
    pub(crate) plan: &'a CopyPlan,
    pub(crate) cache: &'a MethodCache,
    pub(crate) shared: &'a Shared,
    /// Destination capabilities: steps the filesystem cannot store are skipped up front.
    pub(crate) caps: FsCaps,
    /// fsync each file (macOS, Windows, portable); Linux flushes with one `syncfs` at the barrier instead.
    pub(crate) durable: bool,
}

impl FileCtx<'_> {
    /// Counts bytes as they are copied and takes them back if the attempt fails.
    pub(crate) fn meter(&self) -> Meter<'_> {
        Meter {
            progress: &self.shared.progress,
            added: 0,
        }
    }
}

pub(crate) struct Meter<'a> {
    progress: &'a Progress,
    added: u64,
}

impl Meter<'_> {
    pub(crate) fn add(&mut self, n: u64) {
        self.added += n;
        Progress::add(&self.progress.bytes_done, n);
    }

    /// Forgets the bytes of an attempt that will be redone or abandoned.
    pub(crate) fn rollback(&mut self) {
        Progress::sub(&self.progress.bytes_done, self.added);
        self.added = 0;
    }
}

/// A finished file copy. Warnings are metadata steps that failed after the content was copied.
#[derive(Debug, Default)]
pub(crate) struct Copied {
    pub(crate) cloned: bool,
    pub(crate) warnings: Vec<io::Error>,
}

/// `ENOTSUP`/`EXDEV`/`EINVAL`-style failures mean "try the next method" and disable this one for the pair.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn is_unsupported(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::CrossesDevices | io::ErrorKind::InvalidInput
    ) || {
        #[cfg(unix)]
        {
            let unsupported = [
                rustix::io::Errno::NOSYS,
                rustix::io::Errno::OPNOTSUPP,
                rustix::io::Errno::NOTSUP,
                rustix::io::Errno::OVERFLOW,
                rustix::io::Errno::NOTTY,
                rustix::io::Errno::XDEV,
                rustix::io::Errno::INVAL,
            ];
            e.raw_os_error().is_some_and(|code| {
                unsupported.contains(&rustix::io::Errno::from_raw_os_error(code))
            })
        }
        #[cfg(not(unix))]
        false
    }
}

fn is_transient(e: &io::Error) -> bool {
    use io::ErrorKind::*;
    // ERROR_UNEXP_NET_ERR, ERROR_NETNAME_DELETED: an SMB session dropped mid-file.
    let windows_net = cfg!(windows) && matches!(e.raw_os_error(), Some(59 | 64));
    // EMFILE/ENFILE: other workers close their descriptors within one file.
    let out_of_fds = cfg!(unix) && matches!(e.raw_os_error(), Some(23 | 24));
    windows_net
        || out_of_fds
        || matches!(
            e.kind(),
            TimedOut
                | WouldBlock
                | ConnectionReset
                | ConnectionAborted
                | NetworkDown
                | NetworkUnreachable
                | HostUnreachable
                | Interrupted
        ) && !is_cancelled(e)
}

fn is_disk_full(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded
    )
}

/// An error and the step it belongs to.
struct Failure {
    op: ErrorOp,
    err: io::Error,
}

trait At<T> {
    fn at(self, op: ErrorOp) -> Result<T, Failure>;
}

impl<T> At<T> for io::Result<T> {
    fn at(self, op: ErrorOp) -> Result<T, Failure> {
        self.map_err(|err| Failure { op, err })
    }
}

enum Done {
    Copied { cloned: bool },
    Skipped,
}

/// One non-directory entry to copy: `name` in `src_dir` becomes `dst_name` in `dst_dir`.
struct Entry<'e> {
    src_dir: &'e os::Dir,
    name: &'e OsStr,
    kind: EntryKind,
    dst_dir: &'e os::Dir,
    dst_name: &'e OsStr,
    /// Source and destination are the same entry: go straight to `name (2)`.
    keep_both: bool,
}

enum Placed<T> {
    Done(T, OsString),
    Skipped,
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

/// An open source directory and its destination counterpart.
struct Fds {
    src: os::Dir,
    dst: os::Dir,
}

/// The paths of an `Fds`, for queued tasks. Descriptors are opened when a task starts and closed when it
/// ends, so a walk that runs ahead of the copy cannot pile up open directories (and hit `EMFILE`).
/// The per-file work inside a task stays relative to the open descriptors.
struct Pair {
    src: PathBuf,
    dst: PathBuf,
}

impl Pair {
    fn of(fds: &Fds) -> Arc<Self> {
        Arc::new(Self {
            src: fds.src.path().to_path_buf(),
            dst: fds.dst.path().to_path_buf(),
        })
    }

    fn open(&self) -> io::Result<Fds> {
        Ok(Fds {
            src: os::Dir::open_nofollow(&self.src)?,
            dst: os::Dir::open_nofollow(&self.dst)?,
        })
    }
}

/// Where a directory task finds its parent: the shared top-level folders stay open for the whole job.
enum Parent {
    Open(Arc<Fds>),
    Closed(Arc<Pair>),
}

/// A destination directory waiting for its children before its final mode and times are set.
struct Node {
    meta: Meta,
    src_path: PathBuf,
    dst_path: PathBuf,
    /// The listing itself plus every child task still running.
    pending: AtomicUsize,
    parent: Option<Arc<Node>>,
    item: Arc<ItemState>,
}

/// A prepared top-level item.
struct Top {
    item: usize,
    src_parent: PathBuf,
    name: OsString,
    dst_parent: PathBuf,
    dst_name: OsString,
    kind: EntryKind,
    /// Source and destination are the same entry: copy as `name (2)`.
    keep_both: bool,
    src_vol: VolumeInfo,
    dst_vol: VolumeInfo,
    plan: CopyPlan,
}

struct Job<'a> {
    sh: &'a Shared,
    plan: CopyPlan,
    cache: MethodCache,
    caps: FsCaps,
    policy: ConflictPolicy,
    /// First destination of each multiply-linked source inode, so later names become hardlinks.
    links: Mutex<HashMap<(u64, u64), PathBuf>>,
    /// Directories scheduled but not listed yet; the walk is complete when it drops to zero.
    dirs_pending: AtomicUsize,
    last_group: bool,
    items: &'a [Arc<ItemState>],
}

pub(crate) fn run(req: &Request, sh: &Shared) -> Summary {
    os::raise_fd_limit();
    let cores = thread::available_parallelism().map_or(1, |n| n.get());
    let items: Vec<Arc<ItemState>> = req.items.iter().map(|_| Arc::default()).collect();
    let mut prepared = vec![false; req.items.len()];
    let mut parents = Parents::default();
    let mut tops: Vec<Top> = Vec::new();
    for (i, item) in req.items.iter().enumerate() {
        if sh.is_cancelled() {
            break;
        }
        match prepare(req, i, &item.src, &item.dst, cores, &mut parents) {
            Ok(Some(top)) => {
                prepared[i] = true;
                tops.push(top);
            }
            // Moving an item onto itself: nothing to do.
            Ok(None) => prepared[i] = true,
            Err(e) => {
                sh.record(&item.src, ErrorOp::Preflight, &e, false);
                items[i].errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    // Same-volume moves: one rename per item, whatever the tree size.
    let mut copies = Vec::new();
    for top in tops {
        if !top.plan.renames() {
            copies.push(top);
            continue;
        }
        let job = Job::new(sh, top.plan, &top.dst_vol, req.policy, false, &items);
        match job.rename_top(&top) {
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                // Bind mounts and btrfs subvolumes share st_dev semantics only partly: copy instead.
                let plan =
                    CopyPlan::for_moved_data_on(Os::CURRENT, &top.src_vol, &top.dst_vol, cores)
                        .with(&req.overrides);
                copies.push(Top { plan, ..top });
            }
            Err(e) => job.fail(
                top.item,
                &top.src_parent.join(&top.name),
                ErrorOp::Rename,
                &e,
            ),
            Ok(()) => {}
        }
    }

    // One group per distinct (pair, plan): usually exactly one.
    let mut groups: Vec<(CopyPlan, VolumeInfo, VolumeInfo, Vec<Top>)> = Vec::new();
    for top in copies {
        match groups.iter_mut().find(|(plan, s, d, _)| {
            *plan == top.plan && s.id == top.src_vol.id && d.id == top.dst_vol.id
        }) {
            Some(group) => group.3.push(top),
            None => groups.push((top.plan, top.src_vol, top.dst_vol, vec![top])),
        }
    }
    let last = groups.len().saturating_sub(1);
    let mut kept_sources = Vec::new();
    let plans: Vec<CopyPlan> = groups.iter().map(|g| g.0).collect();
    for (gi, (plan, src_vol, dst_vol, tops)) in groups.iter().enumerate() {
        let job = Job::new(sh, *plan, dst_vol, req.policy, gi == last, &items);
        match plan.schedule {
            Schedule::Streaming => job.streaming(tops),
            Schedule::TwoPhase { order } => job.two_phase(tops, order, src_vol.id == dst_vol.id),
        }
        if plan.durability == Durability::BeforeSourceDelete {
            kept_sources.extend(job.delete_moved_sources(tops, &req.items));
        }
    }
    sh.progress.walk_complete.store(true, Ordering::Release);

    let cancelled = sh.is_cancelled();
    let snapshot = sh.snapshot();
    let items_done = if cancelled {
        0
    } else {
        items
            .iter()
            .zip(&prepared)
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
        plans,
        elapsed: snapshot.elapsed,
    }
}

fn entry_kind(t: fs::FileType) -> EntryKind {
    if t.is_symlink() {
        EntryKind::Symlink
    } else if t.is_dir() {
        EntryKind::Dir
    } else if t.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

/// Per-parent results: the items of one paste nearly always share their source and destination folder.
#[derive(Default)]
struct Parents {
    canonical: HashMap<PathBuf, PathBuf>,
    volumes: HashMap<PathBuf, VolumeInfo>,
}

impl Parents {
    fn canonical(&mut self, dir: &Path) -> io::Result<PathBuf> {
        if let Some(real) = self.canonical.get(dir) {
            return Ok(real.clone());
        }
        let real = fs::canonicalize(dir)?;
        self.canonical.insert(dir.to_path_buf(), real.clone());
        Ok(real)
    }
}

/// Pre-flight checks and planning for one item. `Ok(None)`: a move onto itself, already done.
fn prepare(
    req: &Request,
    item: usize,
    src: &Path,
    dst: &Path,
    cores: usize,
    parents: &mut Parents,
) -> io::Result<Option<Top>> {
    let kind = entry_kind(fs::symlink_metadata(src)?.file_type());
    let name = src.file_name().ok_or_else(|| invalid("invalid name"))?;
    let dst_name = dst
        .file_name()
        .ok_or_else(|| invalid("invalid destination name"))?;
    let (Some(src_parent), Some(dst_parent)) = (src.parent(), dst.parent()) else {
        return Err(invalid("invalid name"));
    };
    let src_parent = if src_parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        src_parent
    };
    let dst_parent = if dst_parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dst_parent
    };
    let real_src_parent = parents.canonical(src_parent)?;
    let real_dst_parent = parents.canonical(dst_parent)?;
    let same_entry = real_src_parent.join(name) == real_dst_parent.join(dst_name);
    // A duplicate next to itself (`dir (2)`) is not inside itself.
    if !same_entry
        && kind == EntryKind::Dir
        && real_dst_parent
            .join(dst_name)
            .starts_with(real_src_parent.join(name))
    {
        return Err(invalid("a folder cannot be copied into itself"));
    }
    if same_entry && req.op == Op::Move {
        return Ok(None);
    }
    let mut volume = |dir: &Path| {
        *parents.volumes.entry(dir.to_path_buf()).or_insert_with(|| {
            let mut v = volume_info(dir);
            if let Some(storage) = req.overrides.storage {
                v.storage = storage;
            }
            v
        })
    };
    let (src_vol, dst_vol) = (volume(src_parent), volume(dst_parent));
    let mut plan = CopyPlan::for_pair(&src_vol, &dst_vol, req.op, cores);
    if plan.renames() && req.overrides.method.is_some() {
        plan = CopyPlan::for_moved_data_on(Os::CURRENT, &src_vol, &dst_vol, cores);
    }
    Ok(Some(Top {
        item,
        src_parent: src_parent.to_path_buf(),
        name: name.to_os_string(),
        dst_parent: dst_parent.to_path_buf(),
        dst_name: dst_name.to_os_string(),
        kind,
        keep_both: same_entry,
        src_vol,
        dst_vol,
        plan: plan.with(&req.overrides),
    }))
}

impl<'a> Job<'a> {
    fn new(
        sh: &'a Shared,
        plan: CopyPlan,
        dst: &VolumeInfo,
        policy: ConflictPolicy,
        last_group: bool,
        items: &'a [Arc<ItemState>],
    ) -> Self {
        Self {
            sh,
            plan,
            cache: MethodCache::default(),
            caps: dst.caps,
            policy,
            links: Mutex::default(),
            dirs_pending: AtomicUsize::new(0),
            last_group,
            items,
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

    fn skip(&self, item: &ItemState) {
        item.skipped.fetch_add(1, Ordering::Relaxed);
        Progress::add(&self.sh.progress.skipped, 1);
    }

    fn warn(&self, path: &Path, op: ErrorOp, warnings: Vec<io::Error>) {
        for w in warnings {
            self.sh.record(path, op, &w, true);
        }
    }

    /// The policy's answer, or the user's when the policy is `Ask`.
    fn decide(
        &self,
        src: &Path,
        src_meta: &Meta,
        dst: &Path,
        dst_meta: &Meta,
    ) -> io::Result<Resolution> {
        match conflict::resolve(
            self.policy,
            src_meta,
            dst_meta,
            self.caps.mtime_granularity_ns,
        ) {
            Some(r) => Ok(r),
            None => self.sh.ask(Conflict::new(src, src_meta, dst, dst_meta)),
        }
    }

    /// Runs `create` for `dst_name`, resolving a clash by the policy. `create` must create its target
    /// exclusively and fail with `AlreadyExists` when the name is taken.
    fn place<T>(
        &self,
        e: &Entry<'_>,
        src_path: &Path,
        src_meta: &Meta,
        op: ErrorOp,
        mut create: impl FnMut(&OsStr) -> io::Result<T>,
    ) -> Result<Placed<T>, Failure> {
        let (dst_dir, dst_name) = (e.dst_dir, e.dst_name);
        let resolution = if e.keep_both {
            Resolution::KeepBoth
        } else {
            match create(dst_name) {
                Ok(t) => return Ok(Placed::Done(t, dst_name.to_os_string())),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(Failure { op, err: e }),
            }
            let dst_path = dst_dir.path().join(dst_name);
            let dst_meta = dst_dir.stat_at(dst_name).at(op)?;
            self.decide(src_path, src_meta, &dst_path, &dst_meta)
                .at(op)?
        };
        match resolution {
            Resolution::Skip => Ok(Placed::Skipped),
            Resolution::KeepBoth => {
                for n in 2.. {
                    let candidate = keep_both_name(dst_name, n);
                    match create(&candidate) {
                        Ok(t) => return Ok(Placed::Done(t, candidate)),
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                        Err(e) => return Err(Failure { op, err: e }),
                    }
                }
                unreachable!("unbounded range")
            }
            Resolution::Overwrite => {
                if dst_dir.stat_at(dst_name).at(op)?.kind == EntryKind::Dir {
                    return Err(Failure {
                        op,
                        err: io::Error::new(
                            io::ErrorKind::IsADirectory,
                            "a folder with this name already exists",
                        ),
                    });
                }
                // Never truncate the target in place: a failed overwrite keeps the old content.
                let tmp = temp_name(dst_name);
                // A leftover from a crash; the `.fx-part` name is ours by convention.
                let _ = dst_dir.remove_at(&tmp);
                let t = create(&tmp).at(op)?;
                if let Err(e) = dst_dir.rename_at(&tmp, dst_name) {
                    let _ = dst_dir.remove_at(&tmp);
                    return Err(Failure { op, err: e });
                }
                Ok(Placed::Done(t, dst_name.to_os_string()))
            }
        }
    }

    /// Creates (or merges into) the destination directory. `None`: skipped by the conflict policy.
    fn create_dir(
        &self,
        parent: &os::Dir,
        dst_name: &OsStr,
        src_path: &Path,
        src_meta: &Meta,
        keep_both: bool,
    ) -> Result<Option<os::Dir>, Failure> {
        let op = ErrorOp::CreateDir;
        if !keep_both {
            match parent.create_child(dst_name) {
                Ok(dir) => return Ok(Some(dir)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(Failure { op, err: e }),
            }
            let dst_meta = parent.stat_at(dst_name).at(op)?;
            if dst_meta.kind == EntryKind::Dir {
                return parent.open_child(dst_name).map(Some).at(op);
            }
            let dst_path = parent.path().join(dst_name);
            match self
                .decide(src_path, src_meta, &dst_path, &dst_meta)
                .at(op)?
            {
                Resolution::Skip => return Ok(None),
                Resolution::Overwrite => {
                    return Err(Failure {
                        op,
                        err: io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            "a file with this name already exists",
                        ),
                    });
                }
                Resolution::KeepBoth => {}
            }
        }
        for n in 2.. {
            match parent.create_child(&keep_both_name(dst_name, n)) {
                Ok(dir) => return Ok(Some(dir)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(Failure { op, err: e }),
            }
        }
        unreachable!("unbounded range")
    }

    /// Opens the source child and creates its destination. `None` when skipped, failed or cancelled.
    fn open_dir_pair(
        &self,
        parent: &Fds,
        name: &OsStr,
        dst_name: &OsStr,
        keep_both: bool,
        item: &ItemState,
    ) -> Option<(Fds, Meta)> {
        self.sh.checkpoint().ok()?;
        let src_path = parent.src.path().join(name);
        let opened = parent.src.open_child(name).and_then(|src| {
            let meta = src.stat()?;
            Ok((src, meta))
        });
        let (src, meta) = match opened {
            Ok(pair) => pair,
            Err(e) => {
                self.fail_state(item, &src_path, ErrorOp::List, &e);
                return None;
            }
        };
        match self.create_dir(&parent.dst, dst_name, &src_path, &meta, keep_both) {
            Ok(Some(dst)) => {
                let warnings = os::copy_dir_attrs(&self.ctx(), &src, &dst, &meta);
                self.warn(dst.path(), ErrorOp::Metadata, warnings);
                Some((Fds { src, dst }, meta))
            }
            Ok(None) => {
                self.skip(item);
                None
            }
            Err(f) => {
                self.fail_state(item, &src_path, f.op, &f.err);
                None
            }
        }
    }

    /// Copies one non-directory entry with retries; returns the bytes copied.
    fn copy_entry(
        &self,
        e: Entry<'_>,
        item: &ItemState,
        mut prefetched: Option<(os::Src, Vec<u8>)>,
        mut counted: bool,
    ) -> u64 {
        let path = e.src_dir.path().join(e.name);
        let mut attempt = 0;
        loop {
            if self.sh.checkpoint().is_err() {
                return 0;
            }
            let pre = prefetched.take();
            let result = self.try_entry(&e, &path, pre, &mut counted);
            match result {
                Ok((Done::Copied { cloned }, size)) => {
                    Progress::add(&self.sh.progress.files_done, 1);
                    if cloned {
                        Progress::add(&self.sh.progress.cloned, 1);
                    }
                    return size;
                }
                Ok((Done::Skipped, _)) => {
                    self.skip(item);
                    return 0;
                }
                Err(f) if is_cancelled(&f.err) => return 0,
                Err(f) if is_transient(&f.err) && attempt < RETRY_BACKOFF.len() => {
                    if self.sh.sleep(RETRY_BACKOFF[attempt]).is_err() {
                        return 0;
                    }
                    attempt += 1;
                }
                Err(f) if is_disk_full(&f.err) => match self.sh.disk_full() {
                    Ok(DiskFull::Retry) => {}
                    Ok(DiskFull::Skip) => {
                        self.fail_state(item, &path, f.op, &f.err);
                        return 0;
                    }
                    Err(_) => return 0,
                },
                Err(f) if f.op == ErrorOp::Unsupported => {
                    self.sh.record(&path, f.op, &f.err, true);
                    self.skip(item);
                    return 0;
                }
                Err(f) => {
                    self.fail_state(item, &path, f.op, &f.err);
                    return 0;
                }
            }
        }
    }

    /// One attempt. Returns the outcome and the logical size (for task batching).
    fn try_entry(
        &self,
        e: &Entry<'_>,
        path: &Path,
        prefetched: Option<(os::Src, Vec<u8>)>,
        counted: &mut bool,
    ) -> Result<(Done, u64), Failure> {
        let (src_dir, name, dst_dir) = (e.src_dir, e.name, e.dst_dir);
        let placed_done = |placed: Placed<()>| match placed {
            Placed::Done(..) => Done::Copied { cloned: false },
            Placed::Skipped => Done::Skipped,
        };
        match e.kind {
            EntryKind::Symlink => {
                let meta = src_dir.stat_at(name).at(ErrorOp::Copy)?;
                let placed = self.place(e, path, &meta, ErrorOp::Copy, |n| {
                    os::copy_symlink(src_dir, name, &meta, dst_dir, n)
                })?;
                return Ok((placed_done(placed), 0));
            }
            EntryKind::Other => {
                let meta = src_dir.stat_at(name).at(ErrorOp::Copy)?;
                let placed = self.place(e, path, &meta, ErrorOp::Unsupported, |n| {
                    os::copy_special(&meta, dst_dir, n)
                })?;
                return Ok((placed_done(placed), 0));
            }
            EntryKind::File | EntryKind::Dir => {}
        }
        let (src, data) = match prefetched {
            Some((src, data)) => (src, Some(data)),
            None => (os::open_file(src_dir, name).at(ErrorOp::Copy)?, None),
        };
        let meta = *src.meta();
        if meta.kind != EntryKind::File {
            return Err(Failure {
                op: ErrorOp::Copy,
                err: io::Error::other("the item changed while it was being copied"),
            });
        }
        let progress = &self.sh.progress;
        if !*counted {
            Progress::add(&progress.bytes_total, meta.size);
            *counted = true;
        }
        if meta.size > self.caps.max_file_size {
            return Err(Failure {
                op: ErrorOp::Preflight,
                err: io::Error::new(
                    io::ErrorKind::FileTooLarge,
                    "the file is too large for the destination filesystem",
                ),
            });
        }
        progress.set_current(path, meta.size);

        let link_key = (meta.nlink > 1).then_some((meta.dev, meta.ino));
        let existing = link_key.and_then(|k| lock(&self.links).get(&k).cloned());
        if let Some(existing) = existing {
            match self.place(e, path, &meta, ErrorOp::Link, |n| {
                os::hard_link(&existing, dst_dir, n)
            }) {
                Ok(Placed::Done(..)) => {
                    Progress::add(&progress.bytes_done, meta.size);
                    return Ok((Done::Copied { cloned: false }, 0));
                }
                Ok(Placed::Skipped) => return Ok((Done::Skipped, 0)),
                // The first copy is gone or linking is not allowed: copy the data instead.
                Err(_) => {}
            }
        }

        let ctx = self.ctx();
        let placed = self.place(e, path, &meta, ErrorOp::Copy, |n| {
            os::copy_file(&ctx, &src, src_dir, name, dst_dir, n, data.as_deref())
        })?;
        match placed {
            Placed::Skipped => Ok((Done::Skipped, 0)),
            Placed::Done(copied, final_name) => {
                let dst_path = dst_dir.path().join(&final_name);
                if let Some(key) = link_key {
                    lock(&self.links)
                        .entry(key)
                        .or_insert_with(|| dst_path.clone());
                }
                self.warn(&dst_path, ErrorOp::Metadata, copied.warnings);
                Ok((
                    Done::Copied {
                        cloned: copied.cloned,
                    },
                    meta.size,
                ))
            }
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

    /// Marks one unit of `node` done; the last one applies the final mode and times, bottom-up.
    fn node_done(&self, node: Arc<Node>) {
        let mut current = Some(node);
        while let Some(node) = current {
            if node.pending.fetch_sub(1, Ordering::AcqRel) != 1 {
                return;
            }
            if let Err(e) = os::finish_dir(&node.dst_path, &node.meta, &self.caps) {
                self.sh.record(&node.dst_path, ErrorOp::Metadata, &e, true);
            }
            current = node.parent.clone();
        }
    }

    /// Items of one paste share their parent folders: open each pair once, not once per item,
    /// so thousands of selected files do not hold thousands of descriptors.
    fn open_top(
        &self,
        top: &Top,
        open: &mut HashMap<(PathBuf, PathBuf), Arc<Fds>>,
    ) -> Option<Arc<Fds>> {
        let key = (top.src_parent.clone(), top.dst_parent.clone());
        if let Some(fds) = open.get(&key) {
            return Some(Arc::clone(fds));
        }
        let opened = os::Dir::open(&top.src_parent)
            .and_then(|src| Ok((src, os::Dir::open(&top.dst_parent)?)));
        match opened {
            Ok((src, dst)) => {
                let fds = Arc::new(Fds { src, dst });
                open.insert(key, Arc::clone(&fds));
                Some(fds)
            }
            Err(e) => {
                self.fail(
                    top.item,
                    &top.src_parent.join(&top.name),
                    ErrorOp::Preflight,
                    &e,
                );
                None
            }
        }
    }

    // ---- Streaming -------------------------------------------------------------------------------

    fn streaming(&self, tops: &[Top]) {
        copy_pool(self.plan.workers).install(|| {
            rayon::scope(|s| {
                // Held while top-level items are scheduled, so the walk cannot look complete too early.
                self.dirs_pending.fetch_add(1, Ordering::AcqRel);
                let mut parents = HashMap::new();
                for top in tops {
                    let Some(fds) = self.open_top(top, &mut parents) else {
                        continue;
                    };
                    let item = Arc::clone(&self.items[top.item]);
                    if top.kind == EntryKind::Dir {
                        self.dirs_pending.fetch_add(1, Ordering::AcqRel);
                        s.spawn(move |s| {
                            self.visit_dir(
                                s,
                                Parent::Open(fds),
                                None,
                                &top.name,
                                &top.dst_name,
                                item,
                                top.keep_both,
                            )
                        });
                    } else {
                        Progress::add(&self.sh.progress.files_total, 1);
                        s.spawn(move |_| {
                            let e = Entry {
                                src_dir: &fds.src,
                                name: &top.name,
                                kind: top.kind,
                                dst_dir: &fds.dst,
                                dst_name: &top.dst_name,
                                keep_both: top.keep_both,
                            };
                            self.copy_entry(e, &item, None, false);
                        });
                    }
                }
                self.dir_listed();
            });
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn visit_dir<'s>(
        &'s self,
        s: &rayon::Scope<'s>,
        parent: Parent,
        parent_node: Option<Arc<Node>>,
        name: &OsStr,
        dst_name: &OsStr,
        item: Arc<ItemState>,
        keep_both: bool,
    ) {
        let parent = match parent {
            Parent::Open(fds) => Ok(fds),
            Parent::Closed(pair) => pair.open().map(Arc::new),
        };
        let opened = match &parent {
            Ok(fds) => self.open_dir_pair(fds, name, dst_name, keep_both, &item),
            Err(e) => {
                self.fail_state(
                    &item,
                    &parent_path(&parent_node).join(name),
                    ErrorOp::List,
                    e,
                );
                None
            }
        };
        drop(parent);
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
            item: Arc::clone(&item),
        });
        let entries = fds.src.entries().unwrap_or_else(|e| {
            self.fail_state(&item, fds.src.path(), ErrorOp::List, &e);
            Vec::new()
        });
        let pair = Pair::of(&fds);
        drop(fds);
        let mut files = Vec::new();
        for (child, kind) in entries {
            if kind == EntryKind::Dir {
                self.dirs_pending.fetch_add(1, Ordering::AcqRel);
                node.pending.fetch_add(1, Ordering::AcqRel);
                let (pair, node, item) = (Arc::clone(&pair), Arc::clone(&node), Arc::clone(&item));
                s.spawn(move |s| {
                    self.visit_dir(
                        s,
                        Parent::Closed(pair),
                        Some(node),
                        &child,
                        &child,
                        item,
                        false,
                    )
                });
            } else {
                files.push((child, kind));
            }
        }
        Progress::add(&self.sh.progress.files_total, files.len() as u64);
        while !files.is_empty() {
            let rest = files.split_off(files.len().saturating_sub(FILES_PER_TASK));
            node.pending.fetch_add(1, Ordering::AcqRel);
            let (pair, node) = (Arc::clone(&pair), Arc::clone(&node));
            s.spawn(move |s| self.copy_batch(s, pair, node, rest));
        }
        self.dir_listed();
        self.node_done(node);
    }

    fn copy_batch<'s>(
        &'s self,
        s: &rayon::Scope<'s>,
        pair: Arc<Pair>,
        node: Arc<Node>,
        entries: Vec<(OsString, EntryKind)>,
    ) {
        let fds = match pair.open() {
            Ok(fds) => fds,
            Err(e) => {
                self.fail_state(&node.item, &pair.src, ErrorOp::List, &e);
                return self.node_done(node);
            }
        };
        let mut bytes = 0;
        let mut rest = entries.into_iter();
        while let Some((name, kind)) = rest.next() {
            let e = Entry {
                src_dir: &fds.src,
                name: &name,
                kind,
                dst_dir: &fds.dst,
                dst_name: &name,
                keep_both: false,
            };
            bytes += self.copy_entry(e, &node.item, None, false);
            if bytes >= TASK_BYTES && rest.len() > 0 {
                let remaining: Vec<_> = rest.collect();
                node.pending.fetch_add(1, Ordering::AcqRel);
                let (pair, node) = (Arc::clone(&pair), Arc::clone(&node));
                s.spawn(move |s| self.copy_batch(s, pair, node, remaining));
                break;
            }
        }
        self.node_done(node);
    }

    // ---- TwoPhase --------------------------------------------------------------------------------

    /// Walk and create the whole tree, then copy files one at a time in disk order.
    fn two_phase(&self, tops: &[Top], order: Order, same_volume: bool) {
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
        let progress = &self.sh.progress;
        let mut dirs: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut finals: Vec<(PathBuf, Meta)> = Vec::new();
        let mut files: Vec<File> = Vec::new();
        let add_file = |files: &mut Vec<File>,
                        dir: usize,
                        fds: &Fds,
                        name: OsString,
                        dst_name: OsString,
                        kind,
                        item,
                        keep_both| {
            let (meta, physical) = if kind == EntryKind::File && order == Order::Extent {
                match os::open_file(&fds.src, &name) {
                    Ok(src) => (Ok(*src.meta()), os::extent_key(&src)),
                    Err(e) => (Err(e), None),
                }
            } else {
                (fds.src.stat_at(&name), None)
            };
            let (size, ino) = meta.map_or((0, 0), |m| (m.size, m.ino));
            Progress::add(&progress.files_total, 1);
            Progress::add(&progress.bytes_total, size);
            files.push(File {
                dir,
                name,
                dst_name,
                kind,
                size,
                ino,
                physical,
                item,
                keep_both,
            });
        };

        let mut parents = HashMap::new();
        for top in tops {
            if self.sh.is_cancelled() {
                return;
            }
            let Some(fds) = self.open_top(top, &mut parents) else {
                continue;
            };
            dirs.push((fds.src.path().to_path_buf(), fds.dst.path().to_path_buf()));
            let parent = dirs.len() - 1;
            if top.kind != EntryKind::Dir {
                add_file(
                    &mut files,
                    parent,
                    &fds,
                    top.name.clone(),
                    top.dst_name.clone(),
                    top.kind,
                    top.item,
                    top.keep_both,
                );
                continue;
            }
            let item = &self.items[top.item];
            let mut stack = vec![(fds, top.name.clone(), top.dst_name.clone(), top.keep_both)];
            while let Some((parent, name, dst_name, keep_both)) = stack.pop() {
                let Some((fds, meta)) =
                    self.open_dir_pair(&parent, &name, &dst_name, keep_both, item)
                else {
                    continue;
                };
                finals.push((fds.dst.path().to_path_buf(), meta));
                dirs.push((fds.src.path().to_path_buf(), fds.dst.path().to_path_buf()));
                let di = dirs.len() - 1;
                let entries = fds.src.entries().unwrap_or_else(|e| {
                    self.fail_state(item, fds.src.path(), ErrorOp::List, &e);
                    Vec::new()
                });
                let fds = Arc::new(fds);
                for (child, kind) in entries {
                    if kind == EntryKind::Dir {
                        stack.push((Arc::clone(&fds), child.clone(), child, false));
                    } else {
                        add_file(
                            &mut files,
                            di,
                            &fds,
                            child.clone(),
                            child,
                            kind,
                            top.item,
                            false,
                        );
                    }
                }
            }
        }
        if self.last_group {
            progress.walk_complete.store(true, Ordering::Release);
        }
        if let Some(top) = tops.first() {
            self.check_space(&top.dst_parent);
        }

        // Disk order where the filesystem reports extents, else inode order (as `stat_all` does).
        if files.iter().any(|f| f.physical.is_some()) {
            files.sort_by_key(|f| f.physical.unwrap_or(0));
        } else {
            files.sort_by_key(|f| f.ino);
        }

        let mut open: HashMap<usize, Arc<Fds>> = HashMap::new();
        let mut get = |di: usize, item: usize| -> Option<Arc<Fds>> {
            if let Some(fds) = open.get(&di) {
                return Some(Arc::clone(fds));
            }
            if open.len() >= DIR_CACHE {
                open.clear();
            }
            let (src, dst) = &dirs[di];
            match os::Dir::open(src).and_then(|s| Ok((s, os::Dir::open(dst)?))) {
                Ok((src, dst)) => {
                    let fds = Arc::new(Fds { src, dst });
                    open.insert(di, Arc::clone(&fds));
                    Some(fds)
                }
                Err(e) => {
                    self.fail(item, src, ErrorOp::List, &e);
                    None
                }
            }
        };
        type Batched = (Arc<Fds>, usize, os::Src, Vec<u8>);
        let mut batch: Vec<Batched> = Vec::new();
        let mut batch_bytes = 0;
        let flush = |batch: &mut Vec<Batched>, files: &[File]| {
            for (fds, fi, src, data) in batch.drain(..) {
                let f = &files[fi];
                let e = Entry {
                    src_dir: &fds.src,
                    name: &f.name,
                    kind: f.kind,
                    dst_dir: &fds.dst,
                    dst_name: &f.dst_name,
                    keep_both: false,
                };
                self.copy_entry(e, &self.items[f.item], Some((src, data)), true);
            }
        };
        for fi in 0..files.len() {
            if self.sh.is_cancelled() {
                break;
            }
            let f = &files[fi];
            let Some(fds) = get(f.dir, f.item) else {
                continue;
            };
            let batchable = same_volume
                && f.kind == EntryKind::File
                && !f.keep_both
                && f.size <= BATCH_FILE_MAX;
            if batchable
                && let Some(prefetched) = os::open_file(&fds.src, &f.name).ok().and_then(|src| {
                    let mut data = Vec::new();
                    os::read_all(&src, &mut data).ok()?;
                    Some((src, data))
                })
            {
                batch_bytes += prefetched.1.len() as u64;
                batch.push((fds, fi, prefetched.0, prefetched.1));
                if batch_bytes >= BATCH_BYTES || batch.len() >= BATCH_FILES {
                    flush(&mut batch, &files);
                    batch_bytes = 0;
                }
                continue;
            }
            flush(&mut batch, &files);
            batch_bytes = 0;
            let e = Entry {
                src_dir: &fds.src,
                name: &f.name,
                kind: f.kind,
                dst_dir: &fds.dst,
                dst_name: &f.dst_name,
                keep_both: f.keep_both,
            };
            self.copy_entry(e, &self.items[f.item], None, true);
        }
        flush(&mut batch, &files);

        for (path, meta) in finals.iter().rev() {
            if let Err(e) = os::finish_dir(path, meta, &self.caps) {
                self.sh.record(path, ErrorOp::Metadata, &e, true);
            }
        }
    }

    /// Totals are exact after the TwoPhase walk, so a shortfall is reported before any data moves.
    /// Streaming learns sizes while copying; there a full disk pauses the job instead.
    fn check_space(&self, dst: &Path) {
        let p = &self.sh.progress;
        let needed = p.bytes_total.load(Ordering::Relaxed);
        if let Some(free) = os::free_space(dst)
            && needed > free
        {
            let e = io::Error::new(
                io::ErrorKind::StorageFull,
                format!("{needed} bytes to copy but only {free} bytes free"),
            );
            self.sh.record(dst, ErrorOp::Preflight, &e, true);
        }
    }

    // ---- Moves -----------------------------------------------------------------------------------

    /// Renames a top-level item without ever replacing anything; folders onto folders merge.
    fn rename_top(&self, top: &Top) -> io::Result<()> {
        let src = top.src_parent.join(&top.name);
        let dst = top.dst_parent.join(&top.dst_name);
        Progress::add(&self.sh.progress.files_total, 1);
        self.rename_merge(top.item, &src, &dst, true)
    }

    fn rename_merge(&self, item: usize, src: &Path, dst: &Path, top: bool) -> io::Result<()> {
        self.sh.checkpoint()?;
        match os::rename_no_replace(src, dst) {
            Ok(()) => {
                if top {
                    Progress::add(&self.sh.progress.files_done, 1);
                }
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) if top || e.kind() != io::ErrorKind::CrossesDevices => return Err(e),
            Err(e) => {
                self.fail(item, src, ErrorOp::Rename, &e);
                return Ok(());
            }
        }
        let src_meta = os::lstat(src)?;
        let dst_meta = os::lstat(dst)?;
        if src_meta.kind == EntryKind::Dir && dst_meta.kind == EntryKind::Dir {
            for entry in fs::read_dir(src)? {
                let name = entry?.file_name();
                if let Err(e) = self.rename_merge(item, &src.join(&name), &dst.join(&name), false) {
                    if is_cancelled(&e) {
                        return Err(e);
                    }
                    self.fail(item, &src.join(&name), ErrorOp::Rename, &e);
                }
            }
            // Entries kept by skips or errors keep the folder; that is not a failure of its own.
            let _ = fs::remove_dir(src);
            if top {
                Progress::add(&self.sh.progress.files_done, 1);
            }
            return Ok(());
        }
        match self.decide(src, &src_meta, dst, &dst_meta)? {
            Resolution::Skip => {
                self.skip(&self.items[item]);
                Ok(())
            }
            Resolution::Overwrite
                if (src_meta.kind == EntryKind::Dir) != (dst_meta.kind == EntryKind::Dir) =>
            {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "an item of a different kind with this name already exists",
                ))
            }
            // rename(2) replaces the target atomically.
            Resolution::Overwrite => fs::rename(src, dst),
            Resolution::KeepBoth => {
                let (parent, name) = (
                    dst.parent().unwrap_or(dst),
                    dst.file_name().unwrap_or_default(),
                );
                for n in 2.. {
                    match os::rename_no_replace(src, &parent.join(keep_both_name(name, n))) {
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                        other => return other,
                    }
                }
                unreachable!("unbounded range")
            }
        }
    }

    /// Deletes the sources of cleanly copied items after one durability barrier; returns the kept ones.
    fn delete_moved_sources(&self, tops: &[Top], items: &[super::Item]) -> Vec<PathBuf> {
        let mut kept = Vec::new();
        let clean: Vec<&Top> = tops
            .iter()
            .filter(|t| {
                let ok = !self.sh.is_cancelled() && self.items[t.item].clean();
                if !ok {
                    kept.push(items[t.item].src.clone());
                }
                ok
            })
            .collect();
        let Some(first) = clean.first() else {
            return kept;
        };
        if let Err(e) = os::barrier(&first.dst_parent) {
            self.sh.record(&first.dst_parent, ErrorOp::Sync, &e, false);
            kept.extend(clean.iter().map(|t| items[t.item].src.clone()));
            return kept;
        }
        for top in clean {
            let src = top.src_parent.join(&top.name);
            if let Err(e) = remove_source(&src, top.kind) {
                self.fail(top.item, &src, ErrorOp::Remove, &e);
            }
        }
        kept
    }
}

/// The source folder of a directory node, for error messages when its parent cannot be reopened.
fn parent_path(node: &Option<Arc<Node>>) -> PathBuf {
    node.as_ref()
        .map(|n| n.src_path.clone())
        .unwrap_or_default()
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Never follows a symlink: the link itself is removed.
fn remove_source(path: &Path, kind: EntryKind) -> io::Result<()> {
    match kind {
        EntryKind::Dir => fs::remove_dir_all(path),
        // Windows needs remove_dir for links to directories; `metadata` follows the link only to learn its kind.
        EntryKind::Symlink if cfg!(windows) && fs::metadata(path).is_ok_and(|m| m.is_dir()) => {
            fs::remove_dir(path)
        }
        _ => fs::remove_file(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_classes() {
        assert!(is_transient(&io::ErrorKind::TimedOut.into()));
        assert!(is_transient(&io::ErrorKind::ConnectionReset.into()));
        assert!(!is_transient(&super::super::progress::cancelled()));
        assert!(!is_transient(&io::ErrorKind::PermissionDenied.into()));
        assert!(is_disk_full(&io::ErrorKind::StorageFull.into()));
        assert!(is_disk_full(&io::ErrorKind::QuotaExceeded.into()));
        assert!(is_unsupported(&io::ErrorKind::Unsupported.into()));
        assert!(is_unsupported(&io::ErrorKind::CrossesDevices.into()));
        assert!(!is_unsupported(&io::ErrorKind::PermissionDenied.into()));
        #[cfg(target_os = "linux")]
        for errno in [
            rustix::io::Errno::XDEV,
            rustix::io::Errno::OPNOTSUPP,
            rustix::io::Errno::NOSYS,
            rustix::io::Errno::INVAL,
        ] {
            assert!(is_unsupported(&errno.into()), "{errno:?}");
        }
    }
}
