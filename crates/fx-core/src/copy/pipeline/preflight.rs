//! Pre-flight: checks and plans each top-level item before anything is written, and warns early when
//! the destination is short of space.

use super::errors::invalid;
use super::{Job, os, same_inode};
use crate::EntryKind;
use crate::copy::plan::{CopyPlan, Op, Os};
use crate::copy::progress::ErrorOp;
use crate::copy::{Meta, Overrides, Request};
use crate::sys::volume_info;
use crate::volume::VolumeInfo;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

/// A prepared top-level item.
pub(super) struct Top {
    pub(super) item: usize,
    pub(super) src_parent: PathBuf,
    pub(super) name: OsString,
    pub(super) dst_parent: PathBuf,
    pub(super) dst_name: OsString,
    pub(super) kind: EntryKind,
    /// Source and destination are the same entry: copy as `name copy`.
    pub(super) keep_both: bool,
    /// Bytes the item will occupy that are known without walking: a dense top-level file's size.
    pub(super) known_bytes: u64,
    pub(super) src_vol: VolumeInfo,
    pub(super) dst_vol: VolumeInfo,
    pub(super) plan: CopyPlan,
}

impl Top {
    pub(super) fn src_path(&self) -> PathBuf {
        self.src_parent.join(&self.name)
    }

    pub(super) fn dst_path(&self) -> PathBuf {
        self.dst_parent.join(&self.dst_name)
    }

    /// The same item, copied because its rename crossed devices after all.
    pub(super) fn into_copy(self, req: &Request, cores: usize) -> Self {
        let plan = CopyPlan::for_moved_data_on(Os::CURRENT, &self.src_vol, &self.dst_vol, cores)
            .with(&req.overrides);
        Self { plan, ..self }
    }
}

/// Per-parent results: the items of one paste nearly always share their source and destination folder.
#[derive(Default)]
pub(super) struct Parents {
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

    fn volume(&mut self, dir: &Path, overrides: &Overrides) -> VolumeInfo {
        *self.volumes.entry(dir.to_path_buf()).or_insert_with(|| {
            let mut v = volume_info(dir);
            if let Some(storage) = overrides.storage {
                v.storage = storage;
            }
            v
        })
    }
}

/// Pre-flight checks and planning for one item. `Ok(None)`: a move onto itself, already done.
pub(super) fn prepare(
    req: &Request,
    item: usize,
    src: &Path,
    dst: &Path,
    cores: usize,
    parents: &mut Parents,
) -> io::Result<Option<Top>> {
    let kind = entry_kind(fs::symlink_metadata(src)?.file_type());
    let (src_parent, name) = split(src, "invalid name")?;
    let (dst_parent, dst_name) = split(dst, "invalid destination name")?;
    let real_src = parents.canonical(src_parent)?.join(name);
    let real_dst_parent = parents.canonical(dst_parent)?;
    let real_dst = real_dst_parent.join(dst_name);
    let src_meta = os::lstat(src)?;
    let same_entry =
        real_src == real_dst || os::lstat(dst).is_ok_and(|d| same_inode(&src_meta, &d));
    if !same_entry && kind == EntryKind::Dir && is_inside(&real_src, &real_dst, &src_meta) {
        return Err(invalid("a folder cannot be copied into itself"));
    }
    if same_entry && req.op == Op::Move {
        return Ok(None);
    }
    let src_vol = parents.volume(src_parent, &req.overrides);
    let dst_vol = parents.volume(dst_parent, &req.overrides);
    Ok(Some(Top {
        item,
        src_parent: src_parent.to_path_buf(),
        name: name.to_os_string(),
        dst_parent: dst_parent.to_path_buf(),
        dst_name: dst_name.to_os_string(),
        kind,
        keep_both: same_entry,
        known_bytes: known_bytes(kind, &src_meta),
        src_vol,
        dst_vol,
        plan: plan_for(req, &src_vol, &dst_vol, cores),
    }))
}

/// The folder holding `path` (`.` for a bare name) and its file name.
fn split<'p>(path: &'p Path, what: &'static str) -> io::Result<(&'p Path, &'p OsStr)> {
    let name = path.file_name().ok_or_else(|| invalid(what))?;
    let parent = path.parent().ok_or_else(|| invalid(what))?;
    if parent.as_os_str().is_empty() {
        return Ok((Path::new("."), name));
    }
    Ok((parent, name))
}

/// A duplicate next to itself (`dir (2)`) is not inside itself. Bind mounts can hide the nesting
/// from paths, so the destination's ancestors are compared by inode as well.
fn is_inside(real_src: &Path, real_dst: &Path, src_meta: &Meta) -> bool {
    real_dst.starts_with(real_src)
        || real_dst
            .ancestors()
            .skip(1)
            .any(|a| os::lstat(a).is_ok_and(|m| same_inode(src_meta, &m)))
}

fn known_bytes(kind: EntryKind, meta: &Meta) -> u64 {
    if kind == EntryKind::File && !meta.sparse {
        meta.size
    } else {
        0
    }
}

fn plan_for(req: &Request, src_vol: &VolumeInfo, dst_vol: &VolumeInfo, cores: usize) -> CopyPlan {
    let plan = CopyPlan::for_pair(src_vol, dst_vol, req.op, cores);
    // A forced method needs the data to move, so a move does not take the rename shortcut.
    let plan = if plan.renames() && req.overrides.method.is_some() {
        CopyPlan::for_moved_data_on(Os::CURRENT, src_vol, dst_vol, cores)
    } else {
        plan
    };
    plan.with(&req.overrides)
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

impl Job<'_> {
    /// Totals are exact after the TwoPhase walk, so a shortfall is reported before any data moves.
    pub(super) fn check_space(&self, dst: &Path) {
        let needed = self.sh.progress.bytes_total.load(Ordering::Relaxed);
        self.warn_if_short(dst, needed);
    }

    /// Streaming learns sizes while copying, so only the dense top-level files are known up front.
    /// That is a lower bound: a shortfall against it is certain, and the job still starts (the user may
    /// free space, or the files may be hardlinked), with a full disk pausing it later as before.
    pub(super) fn early_space_check(&self, tops: &[Top]) {
        let mut needed: Vec<(&Path, u64)> = Vec::new();
        for top in tops.iter().filter(|t| t.known_bytes > 0) {
            match needed.iter_mut().find(|(d, _)| *d == top.dst_parent) {
                Some((_, bytes)) => *bytes += top.known_bytes,
                None => needed.push((&top.dst_parent, top.known_bytes)),
            }
        }
        for (dst, bytes) in needed {
            self.warn_if_short(dst, bytes);
        }
    }

    fn warn_if_short(&self, dst: &Path, needed: u64) {
        if let Some(e) = space_shortfall(needed, os::free_space(dst)) {
            self.sh.record(dst, ErrorOp::Preflight, &e, true);
        }
    }
}

/// A `StorageFull` warning when `needed` bytes cannot fit in `free`; `None` when free space is unknown.
fn space_shortfall(needed: u64, free: Option<u64>) -> Option<io::Error> {
    let free = free.filter(|&free| needed > free)?;
    Some(io::Error::new(
        io::ErrorKind::StorageFull,
        format!("{needed} bytes to copy but only {free} bytes free"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_shortfall_warns_only_when_known_and_too_small() {
        let e = space_shortfall(100, Some(99)).expect("100 bytes do not fit in 99");
        assert_eq!(e.kind(), io::ErrorKind::StorageFull);
        assert!(e.to_string().contains("100 bytes to copy but only 99"));
        assert!(space_shortfall(100, Some(100)).is_none());
        assert!(space_shortfall(100, None).is_none());
    }

    #[test]
    fn split_uses_dot_for_a_bare_name() {
        let (parent, name) = split(Path::new("a"), "x").unwrap();
        assert_eq!((parent, name), (Path::new("."), OsStr::new("a")));
        let (parent, name) = split(Path::new("/d/a"), "x").unwrap();
        assert_eq!((parent, name), (Path::new("/d"), OsStr::new("a")));
        assert!(split(Path::new("/"), "x").is_err());
    }
}
