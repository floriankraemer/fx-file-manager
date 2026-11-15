//! What the OS copy backend receives from the pipeline and hands back for one file.

use crate::copy::plan::{CopyPlan, MethodCache};
use crate::copy::progress::{Progress, Shared};
use crate::volume::FsCaps;
use std::io;

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

    /// Takes over the count of another meter whose bytes are already in the progress.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn adopt(&mut self, other: Meter<'_>) {
        self.added += other.added;
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
