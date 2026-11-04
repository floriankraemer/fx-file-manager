//! Use case: storage-aware copy and move.
//!
//! A pure [`CopyPlan`] is chosen per (source volume, destination volume, operation); files then run through the
//! plan's ordered method chain on a dedicated pool. The OS backend lives in `sys::copy`.
//!
//! Safety rules: nothing is ever overwritten unless the conflict policy says so (exclusive creates, overwrite
//! via temp + rename), and a move deletes a source only after its copy finished without errors or skips and
//! the destination passed the durability barrier.

mod conflict;
pub(crate) mod pipeline;
pub(crate) mod plan;
pub(crate) mod progress;

pub use conflict::{Conflict, ConflictPolicy, Resolution, keep_both_name};
pub use plan::{CopyPlan, Durability, Method, Op, Order, Os, Overrides, Schedule};
pub use progress::{CopyError, ErrorOp, RunState, Snapshot};

pub(crate) use progress::Shared;

use crate::EntryKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Seconds and nanoseconds since the Unix epoch, as the OS stores them (no rounding through `SystemTime`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Time {
    pub(crate) secs: i64,
    pub(crate) nanos: u32,
}

impl Time {
    pub(crate) fn nanos_since(self, other: Time) -> i128 {
        (i128::from(self.secs) - i128::from(other.secs)) * 1_000_000_000 + i128::from(self.nanos)
            - i128::from(other.nanos)
    }

    pub(crate) fn system_time(self) -> Option<SystemTime> {
        let secs = Duration::from_secs(self.secs.unsigned_abs());
        if self.secs >= 0 {
            UNIX_EPOCH.checked_add(secs)
        } else {
            UNIX_EPOCH.checked_sub(secs)
        }?
        .checked_add(Duration::from_nanos(u64::from(self.nanos)))
    }

    #[cfg_attr(unix, allow(dead_code))]
    pub(crate) fn from_system_time(t: SystemTime) -> Self {
        match t.duration_since(UNIX_EPOCH) {
            Ok(d) => Self {
                secs: d.as_secs() as i64,
                nanos: d.subsec_nanos(),
            },
            Err(e) => {
                let d = e.duration();
                // Seconds count backwards, nanoseconds forwards.
                let (secs, nanos) = if d.subsec_nanos() == 0 {
                    (-(d.as_secs() as i64), 0)
                } else {
                    (-(d.as_secs() as i64) - 1, 1_000_000_000 - d.subsec_nanos())
                };
                Self { secs, nanos }
            }
        }
    }
}

/// What the engine needs to know about one entry, filled by the OS backend from one `fstat`/`lstat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Meta {
    pub(crate) kind: EntryKind,
    pub(crate) size: u64,
    pub(crate) mtime: Time,
    pub(crate) atime: Time,
    /// Full `st_mode` on Unix (type bits included, for `mknod`); file attributes on Windows.
    pub(crate) mode: u32,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) dev: u64,
    pub(crate) ino: u64,
    /// 1 where the OS does not report link counts cheaply (Windows).
    pub(crate) nlink: u64,
    pub(crate) rdev: u64,
    /// Fewer blocks allocated than the size needs (`st_blocks * 512 < st_size`, `FILE_ATTRIBUTE_SPARSE_FILE`).
    pub(crate) sparse: bool,
}

impl Default for Meta {
    fn default() -> Self {
        Self {
            kind: EntryKind::Other,
            size: 0,
            mtime: Time::default(),
            atime: Time::default(),
            mode: 0,
            uid: 0,
            gid: 0,
            dev: 0,
            ino: 0,
            nlink: 1,
            rdev: 0,
            sparse: false,
        }
    }
}

/// One top-level entry: `src` is copied (or moved) to exactly `dst`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub src: PathBuf,
    pub dst: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub items: Vec<Item>,
    pub op: Op,
    pub policy: ConflictPolicy,
    pub overrides: Overrides,
}

impl Request {
    /// Copies or moves each source into the folder `dest`, keeping its name (a GUI paste).
    pub fn into_dir(sources: &[PathBuf], dest: &Path, op: Op, policy: ConflictPolicy) -> Self {
        Self {
            items: sources
                .iter()
                .map(|src| Item {
                    src: src.clone(),
                    dst: dest.join(src.file_name().unwrap_or(src.as_os_str())),
                })
                .collect(),
            op,
            policy,
            overrides: Overrides::default(),
        }
    }
}

/// The result of a finished (or cancelled) transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub op: Op,
    /// Top-level items requested.
    pub items: usize,
    /// Top-level items copied or moved completely (no error, nothing skipped).
    pub items_done: usize,
    pub files: u64,
    pub bytes: u64,
    pub cloned: u64,
    pub skipped: u64,
    /// Errors and warnings, in the order they happened.
    pub errors: Vec<CopyError>,
    pub cancelled: bool,
    /// Move sources left in place because their copy was incomplete.
    pub kept_sources: Vec<PathBuf>,
    /// One per distinct volume pair, for reporting.
    pub plans: Vec<CopyPlan>,
    pub elapsed: Duration,
}

impl Summary {
    pub fn error_count(&self) -> usize {
        self.errors.iter().filter(|e| !e.warning).count()
    }
}

/// Renames without ever replacing an existing entry: atomic on Linux (`renameat2(RENAME_NOREPLACE)`),
/// macOS (`renamex_np(RENAME_EXCL)`) and Windows (`MoveFileExW` without `REPLACE_EXISTING`).
pub fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    crate::sys::copy::rename_no_replace(from, to)
}

/// Watches and steers a transfer from any thread. Cheap to clone.
#[derive(Clone)]
pub struct TransferHandle(Arc<Shared>);

impl TransferHandle {
    /// `notify` runs on a worker thread whenever a watcher should look again (conflict, disk full, state change).
    pub fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(Shared::new(Box::new(notify))))
    }

    /// Runs `req` on the calling thread (with its own copy pool) until done or cancelled.
    pub fn run(&self, req: &Request) -> Summary {
        pipeline::run(req, &self.0)
    }

    pub fn snapshot(&self) -> Snapshot {
        self.0.snapshot()
    }

    /// Takes effect at the next file or chunk boundary of each worker.
    pub fn pause(&self) {
        self.0.pause();
    }

    pub fn resume(&self) {
        self.0.resume();
    }

    /// Stops within one chunk per worker. In-flight destination files are removed; finished ones stay.
    pub fn cancel(&self) {
        self.0.cancel();
    }

    /// Answers "disk full" with skip: files that do not fit fail instead of pausing the job.
    pub fn skip_disk_full(&self) {
        self.0.skip_disk_full();
    }

    /// Conflicts waiting for an answer, oldest first.
    pub fn conflicts(&self) -> Vec<Conflict> {
        self.0.conflicts()
    }

    pub fn answer_conflict(&self, id: u64, resolution: Resolution, apply_to_all: bool) {
        self.0.answer(id, resolution, apply_to_all);
    }

    /// Errors and warnings so far.
    pub fn report(&self) -> Vec<CopyError> {
        self.0.report()
    }
}

/// A transfer running on its own thread.
pub struct Transfer {
    handle: TransferHandle,
    thread: thread::JoinHandle<Summary>,
}

impl Transfer {
    pub fn start(req: Request, notify: impl Fn() + Send + Sync + 'static) -> Self {
        let handle = TransferHandle::new(notify);
        let worker = handle.clone();
        let thread = thread::Builder::new()
            .name("fx-transfer".into())
            .spawn(move || {
                let summary = worker.run(&req);
                worker.0.notify();
                summary
            })
            .expect("failed to spawn transfer thread");
        Self { handle, thread }
    }

    pub fn handle(&self) -> &TransferHandle {
        &self.handle
    }

    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// Waits for the end. A panic in the engine is re-raised here.
    pub fn join(self) -> Summary {
        self.thread
            .join()
            .unwrap_or_else(|p| std::panic::resume_unwind(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_round_trip_through_system_time() {
        for t in [
            Time {
                secs: 1_700_000_000,
                nanos: 123_456_789,
            },
            Time {
                secs: -2,
                nanos: 500_000_000,
            },
            Time { secs: 0, nanos: 0 },
        ] {
            assert_eq!(Time::from_system_time(t.system_time().unwrap()), t);
        }
        let a = Time { secs: 2, nanos: 0 };
        let b = Time {
            secs: 1,
            nanos: 999_999_999,
        };
        assert_eq!(a.nanos_since(b), 1);
        assert_eq!(b.nanos_since(a), -1);
    }

    #[test]
    fn into_dir_keeps_names() {
        let req = Request::into_dir(
            &[PathBuf::from("/a/x.txt"), PathBuf::from("/b/dir")],
            Path::new("/dst"),
            Op::Copy,
            ConflictPolicy::Skip,
        );
        assert_eq!(req.items[0].dst, Path::new("/dst/x.txt"));
        assert_eq!(req.items[1].dst, Path::new("/dst/dir"));
    }
}
