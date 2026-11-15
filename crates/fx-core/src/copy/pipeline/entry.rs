//! Copying one non-directory entry: retries, hardlink groups, verification and progress.

use super::conflicts::Placed;
use super::dirs::Fds;
use super::errors::{At, Failure, is_disk_full, is_transient};
use super::links::{Claim, LinkLeader};
use super::{ItemState, Job, os};
use crate::EntryKind;
use crate::copy::Meta;
use crate::copy::progress::{DiskFull, ErrorOp, Progress, is_cancelled};
use crate::copy::verify;
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::time::Duration;

/// Transient network errors restart the file after these pauses.
const RETRY_BACKOFF: [Duration; 3] = [
    Duration::from_millis(250),
    Duration::from_secs(1),
    Duration::from_secs(4),
];

/// One non-directory entry to copy: `name` in `src_dir` becomes `dst_name` in `dst_dir`.
pub(super) struct Entry<'e> {
    pub(super) src_dir: &'e os::Dir,
    pub(super) name: &'e OsStr,
    pub(super) kind: EntryKind,
    pub(super) dst_dir: &'e os::Dir,
    pub(super) dst_name: &'e OsStr,
    /// Source and destination are the same entry: go straight to `name (2)`.
    pub(super) keep_both: bool,
}

impl<'e> Entry<'e> {
    /// A child copied under its own name.
    pub(super) fn child(fds: &'e Fds, name: &'e OsStr, kind: EntryKind) -> Self {
        Self {
            src_dir: &fds.src,
            name,
            kind,
            dst_dir: &fds.dst,
            dst_name: name,
            keep_both: false,
        }
    }
}

/// A source file opened (and read) ahead of its copy.
pub(super) type Prefetched = (os::Src, Vec<u8>);

enum Done {
    Copied { cloned: bool },
    Skipped,
}

impl<T> Placed<T> {
    /// The outcome of an entry that carries no data (links, special files).
    fn outcome(&self) -> Done {
        match self {
            Placed::Done(..) => Done::Copied { cloned: false },
            Placed::Skipped => Done::Skipped,
        }
    }
}

impl Job<'_> {
    /// Copies one non-directory entry with retries; returns the bytes copied. `counted`: the file's
    /// size is already in the progress total.
    pub(super) fn copy_entry(
        &self,
        e: Entry<'_>,
        item: &ItemState,
        mut prefetched: Option<Prefetched>,
        mut counted: bool,
    ) -> u64 {
        let path = e.src_dir.path().join(e.name);
        let mut attempt = 0;
        loop {
            if self.sh.checkpoint().is_err() {
                return 0;
            }
            let failure = match self.try_entry(&e, &path, prefetched.take(), &mut counted) {
                Ok((done, size)) => return self.count_done(done, size, item),
                Err(f) => f,
            };
            if !self.should_retry(failure, &path, item, &mut attempt) {
                return 0;
            }
        }
    }

    fn count_done(&self, done: Done, size: u64, item: &ItemState) -> u64 {
        let progress = &self.sh.progress;
        match done {
            Done::Skipped => {
                self.skip(item);
                0
            }
            Done::Copied { cloned } => {
                Progress::add(&progress.files_done, 1);
                if cloned {
                    Progress::add(&progress.cloned, 1);
                }
                size
            }
        }
    }

    /// Records a failed attempt, or waits before the next one; true when the entry should be retried.
    fn should_retry(&self, f: Failure, path: &Path, item: &ItemState, attempt: &mut usize) -> bool {
        if is_cancelled(&f.err) {
            return false;
        }
        if is_transient(&f.err) && *attempt < RETRY_BACKOFF.len() {
            let pause = RETRY_BACKOFF[*attempt];
            *attempt += 1;
            return self.sh.sleep(pause).is_ok();
        }
        if is_disk_full(&f.err) {
            return self.retry_on_disk_full(f, path, item);
        }
        if f.op == ErrorOp::Unsupported {
            self.sh.record(path, f.op, &f.err, true);
            self.skip(item);
            return false;
        }
        self.fail_state(item, path, f.op, &f.err);
        false
    }

    /// Pauses the job until the user frees space (retry) or gives up on the file.
    fn retry_on_disk_full(&self, f: Failure, path: &Path, item: &ItemState) -> bool {
        match self.sh.disk_full() {
            Ok(DiskFull::Retry) => true,
            Ok(DiskFull::Skip) => {
                self.fail_state(item, path, f.op, &f.err);
                false
            }
            Err(_) => false,
        }
    }

    /// One attempt. Returns the outcome and the logical size (for task batching).
    fn try_entry(
        &self,
        e: &Entry<'_>,
        path: &Path,
        prefetched: Option<Prefetched>,
        counted: &mut bool,
    ) -> Result<(Done, u64), Failure> {
        match e.kind {
            EntryKind::Symlink => self.copy_symlink(e, path),
            EntryKind::Other => self.copy_special(e, path),
            EntryKind::File | EntryKind::Dir => self.copy_file(e, path, prefetched, counted),
        }
    }

    fn copy_symlink(&self, e: &Entry<'_>, path: &Path) -> Result<(Done, u64), Failure> {
        let meta = e.src_dir.stat_at(e.name).at(ErrorOp::Copy)?;
        let placed = self.place(e, path, &meta, ErrorOp::Copy, |n| {
            os::copy_symlink(e.src_dir, e.name, &meta, e.dst_dir, n, &self.roots)
        })?;
        Ok((placed.outcome(), 0))
    }

    fn copy_special(&self, e: &Entry<'_>, path: &Path) -> Result<(Done, u64), Failure> {
        let meta = e.src_dir.stat_at(e.name).at(ErrorOp::Copy)?;
        let placed = self.place(e, path, &meta, ErrorOp::Unsupported, |n| {
            os::copy_special(&meta, e.dst_dir, n)
        })?;
        Ok((placed.outcome(), 0))
    }

    fn copy_file(
        &self,
        e: &Entry<'_>,
        path: &Path,
        prefetched: Option<Prefetched>,
        counted: &mut bool,
    ) -> Result<(Done, u64), Failure> {
        let (src, data) = match prefetched {
            Some((src, data)) => (src, Some(data)),
            None => (os::open_file(e.src_dir, e.name).at(ErrorOp::Copy)?, None),
        };
        let meta = *src.meta();
        self.admit(&meta, path, counted)?;

        // The first name of a group is copied by one worker; the others wait for it, then link.
        let claim = self.claim_link(&meta)?;
        if let Some(Claim::Existing(first)) = &claim
            && let Some(done) = self.link_to_first(e, path, &meta, first)
        {
            return Ok((done, 0));
        }
        let leader = match claim {
            Some(Claim::Leader(leader)) => Some(leader),
            _ => None,
        };

        let ctx = self.ctx();
        let placed = self.place(e, path, &meta, ErrorOp::Copy, |n| {
            os::copy_file(&ctx, &src, e.src_dir, e.name, e.dst_dir, n, data.as_deref())
        })?;
        let Placed::Done(copied, final_name) = placed else {
            return Ok((Done::Skipped, 0));
        };
        let cloned = copied.cloned;
        self.finish_file(e, path, &final_name, copied.warnings, leader)?;
        Ok((Done::Copied { cloned }, meta.size))
    }

    /// Checks an opened source file before any data moves and makes it the current one.
    fn admit(&self, meta: &Meta, path: &Path, counted: &mut bool) -> Result<(), Failure> {
        if meta.kind != EntryKind::File {
            return Err(io::Error::other(
                "the item changed while it was being copied",
            ))
            .at(ErrorOp::Copy);
        }
        let progress = &self.sh.progress;
        if !*counted {
            Progress::add(&progress.bytes_total, meta.size);
            *counted = true;
        }
        if meta.size > self.caps.max_file_size {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "the file is too large for the destination filesystem",
            ))
            .at(ErrorOp::Preflight);
        }
        progress.set_current(path, meta.size);
        Ok(())
    }

    /// `None` for a file with a single name.
    fn claim_link(&self, meta: &Meta) -> Result<Option<Claim<'_>>, Failure> {
        if meta.nlink <= 1 {
            return Ok(None);
        }
        self.links
            .claim((meta.dev, meta.ino), self.sh)
            .map(Some)
            .at(ErrorOp::Link)
    }

    /// Links a later name of a group to its first copy. `None`: the first copy is gone or linking is
    /// not allowed, so the data is copied instead.
    fn link_to_first(&self, e: &Entry<'_>, path: &Path, meta: &Meta, first: &Path) -> Option<Done> {
        let linked = self.place(e, path, meta, ErrorOp::Link, |n| {
            os::hard_link(first, e.dst_dir, n)
        });
        match linked {
            Ok(Placed::Done(..)) => {
                Progress::add(&self.sh.progress.bytes_done, meta.size);
                Some(Done::Copied { cloned: false })
            }
            Ok(Placed::Skipped) => Some(Done::Skipped),
            Err(_) => None,
        }
    }

    /// Verifies the copy, publishes it to its hardlink group and records metadata warnings.
    fn finish_file(
        &self,
        e: &Entry<'_>,
        path: &Path,
        final_name: &OsStr,
        warnings: Vec<io::Error>,
        leader: Option<LinkLeader<'_>>,
    ) -> Result<(), Failure> {
        let dst_path = e.dst_dir.path().join(final_name);
        if self.plan.verify
            && let Err(err) = verify::check(self.sh, path, &dst_path)
        {
            // A copy that cannot be trusted is not left behind (a cancel included).
            let _ = e.dst_dir.remove_at(final_name);
            return Err(err).at(ErrorOp::Verify);
        }
        if let Some(leader) = leader {
            leader.finish(dst_path.clone());
        }
        self.warn(&dst_path, ErrorOp::Metadata, warnings);
        Ok(())
    }
}
