//! Portable, path-based copy backend on `std::fs`: the read/write method only.
//! Windows builds on it (`windows_copy`) and replaces the data path with `CopyFile2`.

use crate::EntryKind;
use crate::copy::pipeline::{Copied, FileCtx, Meter};
use crate::copy::{Meta, Time};
use crate::volume::FsCaps;
use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub(crate) struct Dir {
    path: PathBuf,
}

/// The source file's path and metadata; files are opened when copied.
pub(crate) struct Src {
    pub(crate) path: PathBuf,
    pub(crate) meta: Meta,
}

impl Src {
    pub(crate) fn meta(&self) -> &Meta {
        &self.meta
    }
}

fn not_a_dir() -> io::Error {
    io::Error::new(io::ErrorKind::NotADirectory, "not a folder")
}

impl Dir {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        if !fs::metadata(path)?.is_dir() {
            return Err(not_a_dir());
        }
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    /// Like `open`, but a symlink in place of the folder is refused.
    pub(crate) fn open_nofollow(path: &Path) -> io::Result<Self> {
        if !fs::symlink_metadata(path)?.is_dir() {
            return Err(not_a_dir());
        }
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Never follows a symlink, so a link to a folder is not entered.
    pub(crate) fn open_child(&self, name: &OsStr) -> io::Result<Self> {
        let path = self.path.join(name);
        if !fs::symlink_metadata(&path)?.is_dir() {
            return Err(not_a_dir());
        }
        Ok(Self { path })
    }

    pub(crate) fn create_child(&self, name: &OsStr) -> io::Result<Self> {
        let path = self.path.join(name);
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    pub(crate) fn stat(&self) -> io::Result<Meta> {
        lstat(&self.path)
    }

    pub(crate) fn stat_at(&self, name: &OsStr) -> io::Result<Meta> {
        lstat(&self.path.join(name))
    }

    pub(crate) fn remove_at(&self, name: &OsStr) -> io::Result<()> {
        fs::remove_file(self.path.join(name))
    }

    /// Replaces `to` (`MoveFileExW(REPLACE_EXISTING)` on Windows).
    pub(crate) fn rename_at(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
        fs::rename(self.path.join(from), self.path.join(to))
    }

    pub(crate) fn entries(&self) -> io::Result<Vec<(OsString, EntryKind)>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.path)? {
            let entry = entry?;
            let kind = entry.file_type().map_or(EntryKind::Other, kind_of);
            out.push((entry.file_name(), kind));
        }
        Ok(out)
    }
}

fn kind_of(t: fs::FileType) -> EntryKind {
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

pub(crate) fn meta_of(md: &fs::Metadata) -> Meta {
    let time =
        |t: io::Result<std::time::SystemTime>| t.map(Time::from_system_time).unwrap_or_default();
    #[allow(unused_mut)]
    let mut meta = Meta {
        kind: kind_of(md.file_type()),
        size: md.len(),
        mtime: time(md.modified()),
        atime: time(md.accessed()),
        ..Meta::default()
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.mode = md.mode();
        meta.uid = md.uid();
        meta.gid = md.gid();
        meta.dev = md.dev();
        meta.ino = md.ino();
        meta.nlink = md.nlink();
        meta.rdev = md.rdev();
        meta.sparse = md.blocks().saturating_mul(512) < md.len();
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;
        meta.mode = md.file_attributes();
        meta.sparse = meta.mode & FILE_ATTRIBUTE_SPARSE_FILE != 0;
    }
    meta
}

pub(crate) fn lstat(path: &Path) -> io::Result<Meta> {
    Ok(meta_of(&fs::symlink_metadata(path)?))
}

pub(crate) fn open_file(dir: &Dir, name: &OsStr) -> io::Result<Src> {
    let path = dir.path.join(name);
    let meta = lstat(&path)?;
    Ok(Src { path, meta })
}

pub(crate) fn read_all(src: &Src, out: &mut Vec<u8>) -> io::Result<()> {
    out.clear();
    File::open(&src.path)?.read_to_end(out).map(|_| ())
}

thread_local! {
    static BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Creates `dst_name` exclusively, copies in `buf_size` chunks, sets times and the read-only bit,
/// and removes the file again on any failure.
pub(crate) fn copy_file(
    ctx: &FileCtx<'_>,
    src: &Src,
    _src_dir: &Dir,
    _name: &OsStr,
    dst: &Dir,
    dst_name: &OsStr,
    prefetched: Option<&[u8]>,
) -> io::Result<Copied> {
    let dst_path = dst.path.join(dst_name);
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&dst_path)?;
    let mut meter = ctx.meter();
    let result = (|| {
        match prefetched {
            Some(data) => {
                ctx.shared.checkpoint()?;
                out.write_all(data)?;
                meter.add(data.len() as u64);
            }
            None => read_write(ctx, &src.path, &mut out, &mut meter)?,
        }
        let warnings = set_file_metadata(&out, &dst_path, &src.meta, &ctx.caps);
        if ctx.durable {
            out.sync_all()?;
        }
        Ok(Copied {
            cloned: false,
            warnings,
        })
    })();
    if result.is_err() {
        meter.rollback();
        drop(out);
        let _ = fs::remove_file(&dst_path);
    }
    result
}

pub(crate) fn read_write(
    ctx: &FileCtx<'_>,
    src: &Path,
    out: &mut File,
    meter: &mut Meter<'_>,
) -> io::Result<()> {
    let mut input = File::open(src)?;
    BUF.with_borrow_mut(|buf| {
        if buf.len() < ctx.plan.buf_size {
            buf.resize(ctx.plan.buf_size, 0);
        }
        loop {
            ctx.shared.checkpoint()?;
            let n = match input.read(buf) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            out.write_all(&buf[..n])?;
            meter.add(n as u64);
        }
    })
}

/// Times last, so an interrupted copy never carries the source mtime.
pub(crate) fn set_file_metadata(
    out: &File,
    path: &Path,
    m: &Meta,
    caps: &FsCaps,
) -> Vec<io::Error> {
    let mut warnings = Vec::new();
    if (caps.posix_perms || cfg!(windows))
        && let Err(e) = set_permissions(path, m)
    {
        warnings.push(e);
    }
    if let Err(e) = out.set_times(file_times(m)) {
        warnings.push(e);
    }
    warnings
}

fn file_times(m: &Meta) -> FileTimes {
    let mut times = FileTimes::new();
    if let Some(t) = m.atime.system_time() {
        times = times.set_accessed(t);
    }
    if let Some(t) = m.mtime.system_time() {
        times = times.set_modified(t);
    }
    times
}

#[cfg(unix)]
fn set_permissions(path: &Path, m: &Meta) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(m.mode & 0o7777))
}

/// Off Unix only the read-only attribute is portable.
#[cfg(not(unix))]
fn set_permissions(path: &Path, m: &Meta) -> io::Result<()> {
    const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
    let mut p = fs::metadata(path)?.permissions();
    p.set_readonly(m.mode & FILE_ATTRIBUTE_READONLY != 0);
    fs::set_permissions(path, p)
}

pub(crate) fn copy_symlink(
    src_dir: &Dir,
    name: &OsStr,
    _m: &Meta,
    dst: &Dir,
    dst_name: &OsStr,
) -> io::Result<()> {
    let src = src_dir.path.join(name);
    let target = fs::read_link(&src)?;
    let link = dst.path.join(dst_name);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        // ponytail: junctions are recreated as directory symlinks (needs Developer Mode or admin);
        // a junction-specific reparse point write is the upgrade if users copy junctions.
        if fs::metadata(&src).is_ok_and(|m| m.is_dir()) {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "symlinks are not supported here",
        ))
    }
}

pub(crate) fn copy_special(_m: &Meta, _dst: &Dir, _dst_name: &OsStr) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "special files are not copied on this system",
    ))
}

pub(crate) fn hard_link(existing: &Path, dst: &Dir, dst_name: &OsStr) -> io::Result<()> {
    fs::hard_link(existing, dst.path.join(dst_name))
}

pub(crate) fn copy_dir_attrs(
    _ctx: &FileCtx<'_>,
    _src: &Dir,
    _dst: &Dir,
    _m: &Meta,
) -> Vec<io::Error> {
    Vec::new()
}

#[cfg(not(windows))]
pub(crate) fn finish_dir(path: &Path, m: &Meta, caps: &FsCaps) -> io::Result<()> {
    if caps.posix_perms {
        set_permissions(path, m)?;
    }
    File::open(path)?.set_times(file_times(m))
}

/// Check-then-rename: a file created in the gap can be replaced.
#[cfg(not(windows))]
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    if fs::symlink_metadata(to).is_ok() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    fs::rename(from, to)
}

pub(crate) fn raise_fd_limit() {}

/// Each file was synced as it was written.
#[cfg(not(windows))]
pub(crate) fn barrier(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn free_space(_path: &Path) -> Option<u64> {
    None
}

#[cfg(not(windows))]
pub(crate) fn extent_key(_src: &Src) -> Option<u64> {
    None
}
