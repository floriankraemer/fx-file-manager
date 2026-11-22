//! Moves: same-volume renames that merge into existing folders, and the source deletion after a
//! cross-volume copy, behind one durability barrier and a per-entry match check.

use super::conflicts::first_free;
use super::preflight::Top;
use super::{Job, lock, os, same_inode};
use crate::EntryKind;
use crate::copy::conflict::{Beside, Resolution};
use crate::copy::progress::{ErrorOp, Progress, is_cancelled};
use crate::copy::{Item, Meta};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

impl Job<'_> {
    /// Renames a top-level item without ever replacing anything; folders onto folders merge.
    pub(super) fn rename_top(&self, top: &Top) -> io::Result<()> {
        self.rename_merge(top.item, &top.src_path(), &top.dst_path(), true)?;
        Progress::add(&self.sh.progress.files_total, 1);
        Ok(())
    }

    fn rename_merge(&self, item: usize, src: &Path, dst: &Path, top: bool) -> io::Result<()> {
        self.sh.checkpoint()?;
        match os::rename_no_replace(src, dst) {
            Ok(()) => {
                self.count_top(top);
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) if top || e.kind() != io::ErrorKind::CrossesDevices => return Err(e),
            Err(e) => {
                self.fail(item, src, ErrorOp::Rename, &e);
                return Ok(());
            }
        }
        self.rename_onto_existing(item, src, dst, top)
    }

    fn rename_onto_existing(
        &self,
        item: usize,
        src: &Path,
        dst: &Path,
        top: bool,
    ) -> io::Result<()> {
        let src_meta = os::lstat(src)?;
        let dst_meta = os::lstat(dst)?;
        if same_inode(&src_meta, &dst_meta) {
            self.skip(&self.items[item]);
            return Ok(());
        }
        let (src_dir, dst_dir) = (
            src_meta.kind == EntryKind::Dir,
            dst_meta.kind == EntryKind::Dir,
        );
        if src_dir && dst_dir {
            self.merge_dirs(item, src, dst)?;
            self.count_top(top);
            return Ok(());
        }
        match self.decide(src, &src_meta, dst, &dst_meta)? {
            Resolution::Skip => {
                self.skip(&self.items[item]);
                Ok(())
            }
            Resolution::Overwrite if src_dir != dst_dir => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "an item of a different kind with this name already exists",
            )),
            // rename(2) replaces the target atomically.
            Resolution::Overwrite => fs::rename(src, dst),
            Resolution::KeepBoth => rename_beside(src, dst),
        }
    }

    /// Moves each entry of `src` into the existing folder `dst`, then removes `src` if it is empty.
    fn merge_dirs(&self, item: usize, src: &Path, dst: &Path) -> io::Result<()> {
        for entry in fs::read_dir(src)? {
            let name = entry?.file_name();
            let child = src.join(&name);
            match self.rename_merge(item, &child, &dst.join(&name), false) {
                Ok(()) => {}
                Err(e) if is_cancelled(&e) => return Err(e),
                Err(e) => self.fail(item, &child, ErrorOp::Rename, &e),
            }
        }
        // Entries kept by skips or errors keep the folder; that is not a failure of its own.
        if let Err(e) = fs::remove_dir(src) {
            log::debug!("{} stays after the merge: {e}", src.display());
        }
        Ok(())
    }

    fn count_top(&self, top: bool) {
        if top {
            Progress::add(&self.sh.progress.files_done, 1);
        }
    }

    /// Deletes the sources of cleanly copied items after one durability barrier; returns the kept ones.
    pub(super) fn delete_moved_sources(&self, tops: &[Top], items: &[Item]) -> Vec<PathBuf> {
        let source = |t: &Top| items[t.item].src.clone();
        let (clean, dirty): (Vec<&Top>, Vec<&Top>) = tops
            .iter()
            .partition(|t| !self.sh.is_cancelled() && self.items[t.item].clean());
        let mut kept: Vec<PathBuf> = dirty.into_iter().map(source).collect();
        if clean.is_empty() {
            return kept;
        }
        if !self.flush_destinations(&clean) {
            kept.extend(clean.into_iter().map(source));
            return kept;
        }
        let unverified = clean
            .into_iter()
            .filter(|t| !self.remove_verified(t.item, &t.src_path(), &t.dst_path()));
        kept.extend(unverified.map(source));
        kept
    }

    /// The durability barrier on every destination device written to. False after a recorded failure.
    fn flush_destinations(&self, clean: &[&Top]) -> bool {
        let mut devices = std::mem::take(&mut *lock(&self.dst_devices));
        for top in clean {
            let m = match os::lstat(&top.dst_parent) {
                Ok(m) => m,
                Err(e) => {
                    log::warn!(
                        "cannot stat {} for the flush: {e}",
                        top.dst_parent.display()
                    );
                    continue;
                }
            };
            devices
                .entry(m.dev)
                .or_insert_with(|| top.dst_parent.clone());
        }
        for dir in devices.values() {
            if let Err(e) = os::barrier(dir) {
                log::warn!("flush of {} failed, sources are kept: {e}", dir.display());
                self.sh.record(dir, ErrorOp::Sync, &e, false);
                return false;
            }
        }
        true
    }

    /// Deletes the moved `src` entry by entry, each only where its copy at `dst` matches (kind, size,
    /// mtime, link target). Entries created or changed after they were copied stay. True if all is gone.
    fn remove_verified(&self, item: usize, src: &Path, dst: &Path) -> bool {
        let dst = lock(&self.renamed)
            .get(src)
            .cloned()
            .unwrap_or_else(|| dst.to_path_buf());
        self.try_remove_verified(item, src, &dst)
            .unwrap_or_else(|e| {
                self.fail(item, src, ErrorOp::Remove, &e);
                false
            })
    }

    fn try_remove_verified(&self, item: usize, src: &Path, dst: &Path) -> io::Result<bool> {
        let (s, d) = (os::lstat(src)?, os::lstat(dst)?);
        if s.kind != d.kind || same_inode(&s, &d) {
            return Ok(kept(
                src,
                "the copy is of another kind or is the source itself",
            ));
        }
        if s.kind == EntryKind::Dir {
            return self.remove_verified_dir(item, src, dst);
        }
        if !self.copy_matches(&s, &d, src, dst)? {
            return Ok(kept(
                src,
                "it differs from its copy (size, time or link target)",
            ));
        }
        remove_source(src, s.kind)?;
        Ok(true)
    }

    fn remove_verified_dir(&self, item: usize, src: &Path, dst: &Path) -> io::Result<bool> {
        let mut all = true;
        for entry in fs::read_dir(src)? {
            let name = entry?.file_name();
            all &= self.remove_verified(item, &src.join(&name), &dst.join(&name));
        }
        // Fails if an entry appeared meanwhile, which then stays with its folder.
        if !all {
            return Ok(false);
        }
        Ok(fs::remove_dir(src)
            .inspect_err(|e| log::warn!("source folder kept: {}: {e}", src.display()))
            .is_ok())
    }

    /// The copy of a non-directory still matches its source.
    fn copy_matches(&self, s: &Meta, d: &Meta, src: &Path, dst: &Path) -> io::Result<bool> {
        Ok(match s.kind {
            EntryKind::Symlink => {
                fs::read_link(src)? == fs::read_link(dst)? || link_rewritten(&self.roots, src, dst)
            }
            EntryKind::File => {
                let step = u128::from(self.caps.mtime_granularity_ns.max(1));
                s.size == d.size && s.mtime.nanos_since(d.mtime).unsigned_abs() < step
            }
            EntryKind::Dir | EntryKind::Other => true,
        })
    }
}

/// A move source that stays in place; always false, for the "all removed" tally.
fn kept(src: &Path, reason: &str) -> bool {
    log::warn!("source kept: {}: {reason}", src.display());
    false
}

/// Renames `src` to the first free `name (n)` next to `dst`.
fn rename_beside(src: &Path, dst: &Path) -> io::Result<()> {
    let parent = dst.parent().unwrap_or(dst);
    let name = dst.file_name().unwrap_or_default();
    first_free(name, Beside::Numbered, |n| {
        os::rename_no_replace(src, &parent.join(n))
    })
    .map(drop)
}

/// A Windows junction whose target inside the moved tree was rewritten onto the copy.
#[cfg(windows)]
fn link_rewritten(roots: &[PathBuf], src: &Path, dst: &Path) -> bool {
    os::rewritten_link_matches(roots, src, dst)
}

#[cfg(not(windows))]
fn link_rewritten(_roots: &[PathBuf], _src: &Path, _dst: &Path) -> bool {
    false
}

/// Never follows a symlink: the link itself is removed.
fn remove_source(path: &Path, kind: EntryKind) -> io::Result<()> {
    match kind {
        EntryKind::Dir => fs::remove_dir(path),
        // Windows needs remove_dir for links to directories; `metadata` follows the link only to learn its kind.
        EntryKind::Symlink if cfg!(windows) && fs::metadata(path).is_ok_and(|m| m.is_dir()) => {
            fs::remove_dir(path)
        }
        _ => fs::remove_file(path),
    }
}
