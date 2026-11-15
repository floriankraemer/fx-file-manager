//! Directory pairs: opening a source folder with its destination counterpart, shared by both schedules.

use super::errors::Failure;
use super::preflight::Top;
use super::{ItemState, Job, lock, os};
use crate::EntryKind;
use crate::copy::Meta;
use crate::copy::conflict::ConflictPolicy;
use crate::copy::plan::Durability;
use crate::copy::progress::ErrorOp;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// An open source directory and its destination counterpart.
pub(super) struct Fds {
    pub(super) src: os::Dir,
    pub(super) dst: os::Dir,
}

impl Fds {
    pub(super) fn open(src: &Path, dst: &Path) -> io::Result<Self> {
        Ok(Self {
            src: os::Dir::open(src)?,
            dst: os::Dir::open(dst)?,
        })
    }
}

/// The paths of an `Fds`, for queued tasks. Descriptors are opened when a task starts and closed when it
/// ends, so a walk that runs ahead of the copy cannot pile up open directories (and hit `EMFILE`).
/// The per-file work inside a task stays relative to the open descriptors.
pub(super) struct Pair {
    pub(super) src: PathBuf,
    pub(super) dst: PathBuf,
}

impl Pair {
    pub(super) fn of(fds: &Fds) -> Arc<Self> {
        Arc::new(Self {
            src: fds.src.path().to_path_buf(),
            dst: fds.dst.path().to_path_buf(),
        })
    }

    pub(super) fn open(&self) -> io::Result<Fds> {
        Ok(Fds {
            src: os::Dir::open_nofollow(&self.src)?,
            dst: os::Dir::open_nofollow(&self.dst)?,
        })
    }
}

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

/// Top-level parent folders already opened by a schedule, keyed by (source, destination).
pub(super) type OpenParents = HashMap<(PathBuf, PathBuf), Arc<Fds>>;

impl Job<'_> {
    /// Items of one paste share their parent folders: open each pair once, not once per item,
    /// so thousands of selected files do not hold thousands of descriptors.
    pub(super) fn open_top(&self, top: &Top, open: &mut OpenParents) -> Option<Arc<Fds>> {
        let key = (top.src_parent.clone(), top.dst_parent.clone());
        if let Some(fds) = open.get(&key) {
            return Some(Arc::clone(fds));
        }
        let opened = Fds::open(&top.src_parent, &top.dst_parent);
        let item = &self.items[top.item];
        let fds = Arc::new(self.ok_or_fail(item, &top.src_path(), ErrorOp::Preflight, opened)?);
        open.insert(key, Arc::clone(&fds));
        Some(fds)
    }

    /// Opens the source child and creates its destination. `None` when skipped, failed or cancelled.
    /// The meta is set when the destination should end up with the source folder's mode and times.
    pub(super) fn open_dir_pair(
        &self,
        parent: &Fds,
        name: &OsStr,
        dst_name: &OsStr,
        keep_both: bool,
        item: &ItemState,
    ) -> Option<(Fds, Option<Meta>)> {
        self.sh.checkpoint().ok()?;
        let src_path = parent.src.path().join(name);
        let opened = open_src_dir(&parent.src, name);
        let (src, meta) = self.ok_or_fail(item, &src_path, ErrorOp::List, opened)?;
        let (dst, created) =
            match self.create_dir(&parent.dst, dst_name, &src_path, &meta, keep_both) {
                Ok(Some(made)) => made,
                Ok(None) => {
                    self.skip(item);
                    return None;
                }
                Err(Failure { op, err }) => {
                    self.fail_state(item, &src_path, op, &err);
                    return None;
                }
            };
        self.note_device(&dst);
        // A merged folder keeps its own metadata, except when resuming: then the destination
        // is an earlier run of this very copy and should end up like the source.
        if !created && self.policy != ConflictPolicy::SkipIdentical {
            return Some((Fds { src, dst }, None));
        }
        let warnings = os::copy_dir_attrs(&self.ctx(), &src, &dst, &meta);
        self.warn(dst.path(), ErrorOp::Metadata, warnings);
        Some((Fds { src, dst }, Some(meta)))
    }

    /// Remembers one destination folder per device for the move barrier.
    fn note_device(&self, dst: &os::Dir) {
        if self.plan.durability != Durability::BeforeSourceDelete {
            return;
        }
        let Ok(m) = dst.stat() else {
            return;
        };
        lock(&self.dst_devices)
            .entry(m.dev)
            .or_insert_with(|| dst.path().to_path_buf());
    }

    /// The entries of an open source folder; empty after a recorded listing error.
    pub(super) fn list(&self, fds: &Fds, item: &ItemState) -> Vec<(OsString, EntryKind)> {
        let entries = fds.src.entries();
        self.ok_or_fail(item, fds.src.path(), ErrorOp::List, entries)
            .unwrap_or_default()
    }
}

fn open_src_dir(parent: &os::Dir, name: &OsStr) -> io::Result<(os::Dir, Meta)> {
    let src = parent.open_child(name)?;
    let meta = src.stat()?;
    Ok((src, meta))
}
