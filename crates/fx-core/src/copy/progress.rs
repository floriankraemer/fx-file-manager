//! State shared between the copy workers and whoever watches the transfer: lock-free progress counters,
//! pause/cancel control, the conflict queue and the per-item error report.

use super::conflict::{Conflict, Resolution};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Files at least this large show up as the "current file".
const CURRENT_FILE_MIN: u64 = 1024 * 1024;

#[derive(Debug, Default)]
pub(crate) struct Progress {
    pub(crate) bytes_done: AtomicU64,
    pub(crate) files_done: AtomicU64,
    pub(crate) bytes_total: AtomicU64,
    pub(crate) files_total: AtomicU64,
    pub(crate) errors: AtomicU64,
    pub(crate) skipped: AtomicU64,
    pub(crate) cloned: AtomicU64,
    pub(crate) walk_complete: AtomicBool,
    current: Mutex<Option<PathBuf>>,
}

impl Progress {
    pub(crate) fn add(counter: &AtomicU64, n: u64) {
        counter.fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn sub(counter: &AtomicU64, n: u64) {
        counter.fetch_sub(n, Ordering::Relaxed);
    }

    /// Records `path` as being copied when it is large enough for anyone to notice.
    pub(crate) fn set_current(&self, path: &Path, size: u64) {
        if size >= CURRENT_FILE_MIN {
            *lock(&self.current) = Some(path.to_path_buf());
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    Running,
    Paused,
    Cancelled,
}

const RUNNING: u8 = 0;
const PAUSED: u8 = 1;
const CANCELLED: u8 = 2;

/// A point-in-time copy of the counters. Totals grow while `walk_complete` is false ("N of at least M").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub bytes_done: u64,
    pub files_done: u64,
    pub bytes_total: u64,
    pub files_total: u64,
    pub errors: u64,
    pub skipped: u64,
    pub cloned: u64,
    pub walk_complete: bool,
    pub state: RunState,
    /// The job is paused because the destination is full; `resume` retries, `skip_disk_full` gives up on those files.
    pub disk_full: bool,
    pub pending_conflicts: usize,
    pub current: Option<PathBuf>,
    pub elapsed: Duration,
}

/// Which step of an item failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorOp {
    /// Pre-flight check before any data moved (pasting into itself, too large for the filesystem).
    Preflight,
    List,
    CreateDir,
    Copy,
    Link,
    Metadata,
    Rename,
    Sync,
    Remove,
    /// Recorded but not copied (sockets, devices without privilege).
    Unsupported,
}

impl fmt::Display for ErrorOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Preflight => "check",
            Self::List => "read folder",
            Self::CreateDir => "create folder",
            Self::Copy => "copy",
            Self::Link => "link",
            Self::Metadata => "copy attributes",
            Self::Rename => "move",
            Self::Sync => "flush",
            Self::Remove => "remove source",
            Self::Unsupported => "copy special file",
        })
    }
}

/// One per-item failure. Errors never abort the job; warnings (metadata) do not count as errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyError {
    pub path: PathBuf,
    pub op: ErrorOp,
    pub kind: io::ErrorKind,
    pub os_code: Option<i32>,
    pub message: String,
    pub warning: bool,
}

/// `<path>: could not <op>: <message>`.
impl fmt::Display for CopyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: could not {}: {}",
            self.path.display(),
            self.op,
            self.message
        )
    }
}

/// Marker payload of the error that unwinds a worker when the job is cancelled.
#[derive(Debug)]
struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

pub(crate) fn cancelled() -> io::Error {
    io::Error::other(Cancelled)
}

pub(crate) fn is_cancelled(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|inner| inner.is::<Cancelled>())
}

/// Strips the `(os error N)` suffix so messages read as "Permission denied".
pub(crate) fn plain(err: &io::Error) -> String {
    let text = err.to_string();
    match text.rsplit_once(" (os error ") {
        Some((head, _)) => head.to_owned(),
        None => text,
    }
}

/// What a worker waiting on a full disk does once the user answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiskFull {
    Retry,
    Skip,
}

#[derive(Default)]
struct Waits {
    disk_full: bool,
    skip_disk_full: bool,
    conflicts: VecDeque<Conflict>,
    answers: HashMap<u64, Resolution>,
    apply_all: Option<Resolution>,
    next_id: u64,
}

type Notify = Box<dyn Fn() + Send + Sync>;

pub(crate) struct Shared {
    pub(crate) progress: Progress,
    state: AtomicU8,
    waits: Mutex<Waits>,
    cv: Condvar,
    report: Mutex<Vec<CopyError>>,
    notify: Notify,
    started: Instant,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Shared {
    /// `notify` runs (from worker threads) whenever a watcher should look again: conflict, disk full, state change.
    pub(crate) fn new(notify: Notify) -> Self {
        Self {
            progress: Progress::default(),
            state: AtomicU8::new(RUNNING),
            waits: Mutex::default(),
            cv: Condvar::new(),
            report: Mutex::default(),
            notify,
            started: Instant::now(),
        }
    }

    pub(crate) fn state(&self) -> RunState {
        match self.state.load(Ordering::Acquire) {
            RUNNING => RunState::Running,
            PAUSED => RunState::Paused,
            _ => RunState::Cancelled,
        }
    }

    fn set_state(&self, state: u8) {
        let waits = lock(&self.waits);
        // Cancel is final: a late resume must not revive the job.
        if self.state.load(Ordering::Acquire) != CANCELLED {
            self.state.store(state, Ordering::Release);
        }
        drop(waits);
        self.cv.notify_all();
        (self.notify)();
    }

    pub(crate) fn pause(&self) {
        self.set_state(PAUSED);
    }

    pub(crate) fn resume(&self) {
        self.set_state(RUNNING);
    }

    pub(crate) fn cancel(&self) {
        self.set_state(CANCELLED);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire) == CANCELLED
    }

    /// Called between files and between chunks: blocks while paused, errors once cancelled.
    pub(crate) fn checkpoint(&self) -> io::Result<()> {
        match self.state.load(Ordering::Acquire) {
            RUNNING => Ok(()),
            CANCELLED => Err(cancelled()),
            _ => {
                let mut waits = lock(&self.waits);
                while self.state.load(Ordering::Acquire) == PAUSED {
                    waits = self.cv.wait(waits).unwrap_or_else(|p| p.into_inner());
                }
                drop(waits);
                self.checkpoint()
            }
        }
    }

    /// Sleeps for `d` unless cancelled first (retry backoff).
    pub(crate) fn sleep(&self, d: Duration) -> io::Result<()> {
        let deadline = Instant::now() + d;
        let mut waits = lock(&self.waits);
        while !self.is_cancelled() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(());
            }
            waits = self
                .cv
                .wait_timeout(waits, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        Err(cancelled())
    }

    /// Pauses the whole job on a full destination and waits for retry, skip or cancel.
    pub(crate) fn disk_full(&self) -> io::Result<DiskFull> {
        let mut waits = lock(&self.waits);
        if waits.skip_disk_full {
            return Ok(DiskFull::Skip);
        }
        waits.disk_full = true;
        if self.state.load(Ordering::Acquire) == RUNNING {
            self.state.store(PAUSED, Ordering::Release);
        }
        (self.notify)();
        while self.state.load(Ordering::Acquire) == PAUSED {
            waits = self.cv.wait(waits).unwrap_or_else(|p| p.into_inner());
        }
        waits.disk_full = false;
        if self.is_cancelled() {
            return Err(cancelled());
        }
        Ok(if waits.skip_disk_full {
            DiskFull::Skip
        } else {
            DiskFull::Retry
        })
    }

    /// From now on files that hit a full disk fail instead of pausing; resumes the job.
    pub(crate) fn skip_disk_full(&self) {
        lock(&self.waits).skip_disk_full = true;
        self.resume();
    }

    /// Queues `conflict` for the user and blocks this worker until it is answered (other workers go on).
    pub(crate) fn ask(&self, mut conflict: Conflict) -> io::Result<Resolution> {
        let mut waits = lock(&self.waits);
        if let Some(all) = waits.apply_all {
            return Ok(all);
        }
        let id = waits.next_id;
        waits.next_id += 1;
        conflict.id = id;
        waits.conflicts.push_back(conflict);
        (self.notify)();
        loop {
            if let Some(answer) = waits.answers.remove(&id) {
                return Ok(answer);
            }
            if self.is_cancelled() {
                waits.conflicts.retain(|c| c.id != id);
                return Err(cancelled());
            }
            waits = self.cv.wait(waits).unwrap_or_else(|p| p.into_inner());
        }
    }

    /// Answers conflict `id`; `apply_to_all` answers every queued and future conflict the same way.
    pub(crate) fn answer(&self, id: u64, resolution: Resolution, apply_to_all: bool) {
        let mut waits = lock(&self.waits);
        let ids: Vec<u64> = if apply_to_all {
            waits.apply_all = Some(resolution);
            waits.conflicts.drain(..).map(|c| c.id).collect()
        } else {
            waits.conflicts.retain(|c| c.id != id);
            vec![id]
        };
        for id in ids {
            waits.answers.insert(id, resolution);
        }
        drop(waits);
        self.cv.notify_all();
        (self.notify)();
    }

    pub(crate) fn conflicts(&self) -> Vec<Conflict> {
        lock(&self.waits).conflicts.iter().cloned().collect()
    }

    pub(crate) fn record(&self, path: &Path, op: ErrorOp, err: &io::Error, warning: bool) {
        if !warning {
            Progress::add(&self.progress.errors, 1);
        }
        lock(&self.report).push(CopyError {
            path: path.to_path_buf(),
            op,
            kind: err.kind(),
            os_code: err.raw_os_error(),
            message: plain(err),
            warning,
        });
    }

    pub(crate) fn report(&self) -> Vec<CopyError> {
        lock(&self.report).clone()
    }

    pub(crate) fn notify(&self) {
        (self.notify)();
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let p = &self.progress;
        let load = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let waits = lock(&self.waits);
        Snapshot {
            bytes_done: load(&p.bytes_done),
            files_done: load(&p.files_done),
            bytes_total: load(&p.bytes_total),
            files_total: load(&p.files_total),
            errors: load(&p.errors),
            skipped: load(&p.skipped),
            cloned: load(&p.cloned),
            walk_complete: p.walk_complete.load(Ordering::Acquire),
            state: self.state(),
            disk_full: waits.disk_full,
            pending_conflicts: waits.conflicts.len(),
            current: lock(&p.current).clone(),
            elapsed: self.started.elapsed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    fn shared() -> Arc<Shared> {
        Arc::new(Shared::new(Box::new(|| {})))
    }

    #[test]
    fn pause_blocks_checkpoints_until_resume_and_cancel_is_final() {
        let sh = shared();
        sh.checkpoint().unwrap();
        sh.pause();
        let worker = {
            let sh = Arc::clone(&sh);
            thread::spawn(move || sh.checkpoint().is_ok())
        };
        thread::sleep(Duration::from_millis(20));
        assert!(!worker.is_finished(), "a paused checkpoint must block");
        sh.resume();
        assert!(worker.join().unwrap());

        sh.cancel();
        sh.resume();
        assert_eq!(sh.state(), RunState::Cancelled);
        assert!(is_cancelled(&sh.checkpoint().unwrap_err()));
        assert!(sh.sleep(Duration::from_secs(5)).is_err());
    }

    #[test]
    fn cancel_wakes_a_paused_worker() {
        let sh = shared();
        sh.pause();
        let worker = {
            let sh = Arc::clone(&sh);
            thread::spawn(move || sh.checkpoint())
        };
        thread::sleep(Duration::from_millis(10));
        sh.cancel();
        assert!(is_cancelled(&worker.join().unwrap().unwrap_err()));
    }

    #[test]
    fn disk_full_pauses_until_retry_or_skip() {
        let sh = shared();
        let worker = {
            let sh = Arc::clone(&sh);
            thread::spawn(move || sh.disk_full().unwrap())
        };
        while !sh.snapshot().disk_full {
            thread::yield_now();
        }
        assert_eq!(sh.state(), RunState::Paused);
        sh.resume();
        assert_eq!(worker.join().unwrap(), DiskFull::Retry);
        sh.skip_disk_full();
        assert_eq!(sh.disk_full().unwrap(), DiskFull::Skip);
        assert_eq!(sh.state(), RunState::Running);
    }

    #[test]
    fn conflicts_wait_for_an_answer_and_apply_to_all_sticks() {
        let sh = shared();
        let ask = |sh: &Arc<Shared>| {
            let sh = Arc::clone(sh);
            thread::spawn(move || sh.ask(Conflict::default()).unwrap())
        };
        let first = ask(&sh);
        while sh.conflicts().is_empty() {
            thread::yield_now();
        }
        let id = sh.conflicts()[0].id;
        sh.answer(id, Resolution::KeepBoth, false);
        assert_eq!(first.join().unwrap(), Resolution::KeepBoth);

        let second = ask(&sh);
        while sh.conflicts().is_empty() {
            thread::yield_now();
        }
        sh.answer(sh.conflicts()[0].id, Resolution::Skip, true);
        assert_eq!(second.join().unwrap(), Resolution::Skip);
        assert_eq!(sh.ask(Conflict::default()).unwrap(), Resolution::Skip);
        assert!(sh.conflicts().is_empty());
    }

    #[test]
    fn errors_count_and_warnings_do_not() {
        let sh = shared();
        let e = io::Error::from_raw_os_error(13);
        sh.record(Path::new("a"), ErrorOp::Copy, &e, false);
        sh.record(Path::new("b"), ErrorOp::Metadata, &e, true);
        assert_eq!(sh.snapshot().errors, 1);
        let report = sh.report();
        assert_eq!(report.len(), 2);
        assert_eq!(
            report[0].to_string(),
            "a: could not copy: Permission denied"
        );
        assert!(report[1].warning);
    }
}
