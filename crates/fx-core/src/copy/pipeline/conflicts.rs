//! Name clashes: exclusive creates, the conflict policy, `name (2)` and overwrite via temp + rename.

use super::entry::Entry;
use super::errors::{At, Failure};
use super::{Job, lock, os, same_inode};
use crate::EntryKind;
use crate::copy::Meta;
use crate::copy::conflict::{self, Conflict, Resolution, keep_both_name, temp_name};
use crate::copy::plan::Op;
use crate::copy::progress::ErrorOp;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;

pub(super) enum Placed<T> {
    Done(T, OsString),
    Skipped,
}

/// An exclusive create: `Ok(None)` when the name is taken.
pub(super) fn created<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(e),
    }
}

/// Tries `name (2)`, `name (3)`, … until `create` finds a free one; returns its result and the name.
pub(super) fn first_free<T>(
    name: &OsStr,
    mut create: impl FnMut(&OsStr) -> io::Result<T>,
) -> io::Result<(T, OsString)> {
    for n in 2.. {
        let candidate = keep_both_name(name, n);
        if let Some(t) = created(create(&candidate))? {
            return Ok((t, candidate));
        }
    }
    unreachable!("unbounded range")
}

impl Job<'_> {
    /// The policy's answer, or the user's when the policy is `Ask`.
    pub(super) fn decide(
        &self,
        src: &Path,
        src_meta: &Meta,
        dst: &Path,
        dst_meta: &Meta,
    ) -> io::Result<Resolution> {
        let policy = conflict::resolve(
            self.policy,
            src_meta,
            dst_meta,
            self.caps.mtime_granularity_ns,
        );
        match policy {
            Some(r) => Ok(r),
            None => self.sh.ask(Conflict::new(src, src_meta, dst, dst_meta)),
        }
    }

    /// Like [`Job::decide`], except that the "existing" item may be the source itself: overwriting it
    /// would destroy it, and a move would then delete the only copy.
    fn resolve(
        &self,
        src: &Path,
        src_meta: &Meta,
        dst: &Path,
        dst_meta: &Meta,
    ) -> io::Result<Resolution> {
        if same_inode(src_meta, dst_meta) {
            return Ok(self.same_entry());
        }
        self.decide(src, src_meta, dst, dst_meta)
    }

    /// How a clash with the source itself resolves: a copy becomes `name (2)`, a move leaves it alone.
    fn same_entry(&self) -> Resolution {
        match self.op {
            Op::Copy => Resolution::KeepBoth,
            Op::Move => Resolution::Skip,
        }
    }

    /// Runs `create` for `dst_name`, resolving a clash by the policy. `create` must create its target
    /// exclusively and fail with `AlreadyExists` when the name is taken.
    pub(super) fn place<T>(
        &self,
        e: &Entry<'_>,
        src_path: &Path,
        src_meta: &Meta,
        op: ErrorOp,
        mut create: impl FnMut(&OsStr) -> io::Result<T>,
    ) -> Result<Placed<T>, Failure> {
        let (dst_dir, dst_name) = (e.dst_dir, e.dst_name);
        if e.keep_both {
            return self
                .place_beside(dst_dir, dst_name, src_path, create)
                .at(op);
        }
        if let Some(t) = created(create(dst_name)).at(op)? {
            return Ok(Placed::Done(t, dst_name.to_os_string()));
        }
        let dst_path = dst_dir.path().join(dst_name);
        let dst_meta = dst_dir.stat_at(dst_name).at(op)?;
        let resolution = self
            .resolve(src_path, src_meta, &dst_path, &dst_meta)
            .at(op)?;
        match resolution {
            Resolution::Skip => Ok(Placed::Skipped),
            Resolution::KeepBoth => self
                .place_beside(dst_dir, dst_name, src_path, create)
                .at(op),
            Resolution::Overwrite => place_over(dst_dir, dst_name, create).at(op),
        }
    }

    fn place_beside<T>(
        &self,
        dst_dir: &os::Dir,
        dst_name: &OsStr,
        src_path: &Path,
        create: impl FnMut(&OsStr) -> io::Result<T>,
    ) -> io::Result<Placed<T>> {
        let (t, name) = first_free(dst_name, create)?;
        self.note_rename(src_path, &dst_dir.path().join(&name));
        Ok(Placed::Done(t, name))
    }

    /// Creates (or merges into) the destination directory; the flag is true for a new folder.
    /// `None`: skipped by the conflict policy.
    pub(super) fn create_dir(
        &self,
        parent: &os::Dir,
        dst_name: &OsStr,
        src_path: &Path,
        src_meta: &Meta,
        keep_both: bool,
    ) -> Result<Option<(os::Dir, bool)>, Failure> {
        let op = ErrorOp::CreateDir;
        if keep_both {
            return self.create_dir_beside(parent, dst_name, src_path).at(op);
        }
        if let Some(dir) = created(parent.create_child(dst_name)).at(op)? {
            return Ok(Some((dir, true)));
        }
        let dst_meta = parent.stat_at(dst_name).at(op)?;
        if dst_meta.kind == EntryKind::Dir && !same_inode(src_meta, &dst_meta) {
            // An existing folder is merged into, and keeps its own mode and times.
            return parent.open_child(dst_name).map(|d| Some((d, false))).at(op);
        }
        let dst_path = parent.path().join(dst_name);
        match self
            .resolve(src_path, src_meta, &dst_path, &dst_meta)
            .at(op)?
        {
            Resolution::Skip => Ok(None),
            Resolution::Overwrite => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a file with this name already exists",
            ))
            .at(op),
            Resolution::KeepBoth => self.create_dir_beside(parent, dst_name, src_path).at(op),
        }
    }

    fn create_dir_beside(
        &self,
        parent: &os::Dir,
        dst_name: &OsStr,
        src_path: &Path,
    ) -> io::Result<Option<(os::Dir, bool)>> {
        let (dir, name) = first_free(dst_name, |n| parent.create_child(n))?;
        self.note_rename(src_path, &parent.path().join(&name));
        Ok(Some((dir, true)))
    }

    fn note_rename(&self, src: &Path, dst: &Path) {
        if self.op == Op::Move {
            lock(&self.renamed).insert(src.to_path_buf(), dst.to_path_buf());
        }
    }
}

/// Never truncates the target in place: a failed overwrite keeps the old content.
/// The temp name is unique, so parallel overwrites of one name never share it.
fn place_over<T>(
    dst_dir: &os::Dir,
    dst_name: &OsStr,
    mut create: impl FnMut(&OsStr) -> io::Result<T>,
) -> io::Result<Placed<T>> {
    if dst_dir.stat_at(dst_name)?.kind == EntryKind::Dir {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "a folder with this name already exists",
        ));
    }
    let tmp = temp_name(dst_name);
    let t = create(&tmp)?;
    if let Err(e) = dst_dir.rename_at(&tmp, dst_name) {
        let _ = dst_dir.remove_at(&tmp);
        return Err(e);
    }
    Ok(Placed::Done(t, dst_name.to_os_string()))
}
