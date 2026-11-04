//! Copy backend for Linux and macOS: dirfd-relative `openat`/`mkdirat`/`linkat` throughout, so no full
//! path is resolved per file. Data goes through the plan's method chain:
//! Linux `FICLONE` -> `copy_file_range` -> `sendfile` -> read/write, macOS `fclonefileat` -> read/write.

use crate::EntryKind;
use crate::copy::pipeline::{Copied, FileCtx, is_unsupported};
use crate::copy::{Meta, Method, Time};
use crate::volume::FsCaps;
use rustix::fs::{
    AtFlags, CWD, FileType, Mode, OFlags, SeekFrom, Stat, Timespec, Timestamps, fstat, ftruncate,
    futimens, linkat, mkdirat, openat, readlinkat, renameat, renameat_with, seek, statat,
    symlinkat, unlinkat, utimensat,
};
use rustix::io::Errno;
use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Bounds cancel latency and progress granularity of in-kernel copies.
const CHUNK: usize = 16 * 1024 * 1024;

pub(crate) struct Dir {
    fd: OwnedFd,
    path: PathBuf,
}

/// An open regular source file and the `fstat` taken from that very descriptor.
pub(crate) struct Src {
    fd: OwnedFd,
    meta: Meta,
}

impl Src {
    pub(crate) fn meta(&self) -> &Meta {
        &self.meta
    }
}

const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

impl Dir {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        // The root itself may be reached through a symlink (`~/link-to-dir`); its children never are.
        let fd = openat(
            CWD,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok(Self {
            fd,
            path: path.to_path_buf(),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn open_child(&self, name: &OsStr) -> io::Result<Self> {
        let fd = openat(&self.fd, name, DIR_FLAGS, Mode::empty())?;
        Ok(Self {
            fd,
            path: self.path.join(name),
        })
    }

    /// `0700` so a read-only source folder does not block creating its children; the final mode
    /// comes in post-order (`finish_dir`).
    pub(crate) fn create_child(&self, name: &OsStr) -> io::Result<Self> {
        mkdirat(&self.fd, name, Mode::RWXU)?;
        self.open_child(name)
    }

    pub(crate) fn stat(&self) -> io::Result<Meta> {
        Ok(meta(&fstat(&self.fd)?))
    }

    pub(crate) fn stat_at(&self, name: &OsStr) -> io::Result<Meta> {
        Ok(meta(&statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW)?))
    }

    pub(crate) fn remove_at(&self, name: &OsStr) -> io::Result<()> {
        Ok(unlinkat(&self.fd, name, AtFlags::empty())?)
    }

    /// Replaces `to` atomically.
    pub(crate) fn rename_at(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
        Ok(renameat(&self.fd, from, &self.fd, to)?)
    }

    /// Names and kinds from `d_type`; `DT_UNKNOWN` costs one `fstatat`.
    pub(crate) fn entries(&self) -> io::Result<Vec<(OsString, EntryKind)>> {
        let mut out = Vec::new();
        for entry in rustix::fs::Dir::read_from(&self.fd)? {
            let entry = entry?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            let name = OsStr::from_bytes(name);
            let kind = match entry.file_type() {
                FileType::Unknown => self.stat_at(name)?.kind,
                t => kind_of(t),
            };
            out.push((name.to_os_string(), kind));
        }
        Ok(out)
    }
}

fn kind_of(t: FileType) -> EntryKind {
    match t {
        FileType::RegularFile => EntryKind::File,
        FileType::Directory => EntryKind::Dir,
        FileType::Symlink => EntryKind::Symlink,
        _ => EntryKind::Other,
    }
}

#[allow(clippy::unnecessary_cast, clippy::useless_conversion)]
fn meta(st: &Stat) -> Meta {
    let mode = st.st_mode as u32;
    let size = u64::try_from(st.st_size).unwrap_or(0);
    Meta {
        kind: kind_of(FileType::from_raw_mode(mode as _)),
        size,
        mtime: Time {
            secs: st.st_mtime as i64,
            nanos: st.st_mtime_nsec as u32,
        },
        atime: Time {
            secs: st.st_atime as i64,
            nanos: st.st_atime_nsec as u32,
        },
        mode,
        uid: st.st_uid as u32,
        gid: st.st_gid as u32,
        dev: st.st_dev as u64,
        ino: st.st_ino as u64,
        nlink: st.st_nlink as u64,
        rdev: st.st_rdev as u64,
        sparse: (st.st_blocks as u64).saturating_mul(512) < size,
    }
}

pub(crate) fn lstat(path: &Path) -> io::Result<Meta> {
    Ok(meta(&statat(CWD, path, AtFlags::SYMLINK_NOFOLLOW)?))
}

/// `O_NONBLOCK` keeps a FIFO swapped in after the listing from hanging the worker; regular files ignore it.
pub(crate) fn open_file(dir: &Dir, name: &OsStr) -> io::Result<Src> {
    let fd = openat(
        &dir.fd,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let meta = meta(&fstat(&fd)?);
    Ok(Src { fd, meta })
}

pub(crate) fn read_all(src: &Src, out: &mut Vec<u8>) -> io::Result<()> {
    out.clear();
    out.reserve(src.meta.size as usize);
    let mut chunk = vec![0u8; (src.meta.size as usize).clamp(4096, CHUNK)];
    let mut off = 0u64;
    loop {
        match rustix::io::pread(&src.fd, &mut chunk[..], off) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                out.extend_from_slice(&chunk[..n]);
                off += n as u64;
            }
            Err(Errno::INTR) => {}
            Err(e) => return Err(e.into()),
        }
    }
}

fn timestamps(m: &Meta) -> Timestamps {
    let ts = |t: Time| Timespec {
        tv_sec: t.secs as _,
        tv_nsec: t.nanos as _,
    };
    Timestamps {
        last_access: ts(m.atime),
        last_modification: ts(m.mtime),
    }
}

fn ids() -> (u32, u32) {
    static IDS: OnceLock<(u32, u32)> = OnceLock::new();
    *IDS.get_or_init(|| {
        (
            rustix::process::geteuid().as_raw(),
            rustix::process::getegid().as_raw(),
        )
    })
}

/// Creates `dst_name` exclusively in `dst`, copies data and metadata, and removes it again on any failure
/// (a cancel included). `AlreadyExists` means the name is taken and nothing was touched.
pub(crate) fn copy_file(
    ctx: &FileCtx<'_>,
    src: &Src,
    src_dir: &Dir,
    name: &OsStr,
    dst: &Dir,
    dst_name: &OsStr,
    prefetched: Option<&[u8]>,
) -> io::Result<Copied> {
    let _ = (src_dir, name);
    #[cfg(target_os = "macos")]
    if prefetched.is_none()
        && let Some(copied) = macos::try_clone(ctx, src, dst, dst_name)?
    {
        return Ok(copied);
    }
    let out = openat(
        &dst.fd,
        dst_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )?;
    let mut meter = ctx.meter();
    let result = (|| {
        let cloned = match prefetched {
            Some(data) => {
                ctx.shared.checkpoint()?;
                write_at(out.as_fd(), data, 0)?;
                meter.add(data.len() as u64);
                false
            }
            None => write_data(ctx, src, out.as_fd(), &mut meter)?,
        };
        let warnings = copy_metadata(ctx, src, out.as_fd(), cloned);
        if ctx.durable {
            rustix::fs::fsync(&out)?;
        }
        Ok(Copied { cloned, warnings })
    })();
    if result.is_err() {
        meter.rollback();
        drop(out);
        let _ = unlinkat(&dst.fd, dst_name, AtFlags::empty());
    }
    result
}

fn write_at(fd: BorrowedFd<'_>, mut data: &[u8], mut off: u64) -> io::Result<()> {
    while !data.is_empty() {
        match rustix::io::pwrite(fd, data, off) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => {
                data = &data[n..];
                off += n as u64;
            }
            Err(Errno::INTR) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Byte ranges holding data; `[(0, u64::MAX)]` means "everything, to EOF" (dense files and growing files).
fn data_regions(fd: BorrowedFd<'_>, size: u64) -> io::Result<Vec<(u64, u64)>> {
    let mut regions = Vec::new();
    let mut pos = 0;
    while pos < size {
        let start = match seek(fd, SeekFrom::Data(pos)) {
            Ok(start) => start,
            Err(Errno::NXIO) => break,
            // No SEEK_DATA support: treat the file as dense.
            Err(_) => return Ok(vec![(0, u64::MAX)]),
        };
        let end = match seek(fd, SeekFrom::Hole(start)) {
            Ok(end) => end.min(size),
            Err(_) => size,
        };
        if end <= start {
            break;
        }
        regions.push((start, end));
        pos = end;
    }
    Ok(regions)
}

/// Runs the method chain. Returns whether the data was cloned.
fn write_data(
    ctx: &FileCtx<'_>,
    src: &Src,
    out: BorrowedFd<'_>,
    meter: &mut crate::copy::pipeline::Meter<'_>,
) -> io::Result<bool> {
    let size = src.meta.size;
    // copy_file_range alone fills holes: walk the data regions of sparse files instead.
    let regions = if src.meta.sparse {
        data_regions(src.fd.as_fd(), size)?
    } else {
        vec![(0, u64::MAX)]
    };
    let large = size >= ctx.plan.large_file;
    let uncached = ctx.plan.uncached_from.is_some_and(|limit| size >= limit);
    let mut prepared = false;
    let mut tried = false;
    for &method in ctx.plan.methods {
        let native = match method {
            Method::ReadWrite => true,
            Method::Ficlone | Method::CopyFileRange | Method::Sendfile => cfg!(target_os = "linux"),
            // CloneFileAt runs before the destination is created (`macos::try_clone`).
            _ => false,
        };
        if !native || !ctx.cache.enabled(method) {
            continue;
        }
        if tried {
            // A failed method may have written part of the file: start over.
            ftruncate(out, 0)?;
            meter.rollback();
        }
        if !method.clones() && !prepared {
            prepared = true;
            if large && !src.meta.sparse {
                prepare_large(src.fd.as_fd(), out, size, uncached)?;
            }
        }
        tried = true;
        let result = match method {
            #[cfg(target_os = "linux")]
            Method::Ficlone => rustix::fs::ioctl_ficlone(out, &src.fd)
                .map(|()| meter.add(size))
                .map_err(io::Error::from),
            #[cfg(target_os = "linux")]
            Method::CopyFileRange => linux::copy_ranges(
                ctx,
                src,
                out,
                &regions,
                meter,
                linux::Kind::CopyFileRange,
                uncached,
            ),
            #[cfg(target_os = "linux")]
            Method::Sendfile => linux::copy_ranges(
                ctx,
                src,
                out,
                &regions,
                meter,
                linux::Kind::Sendfile,
                uncached,
            ),
            _ => read_write(ctx, src, out, &regions, meter),
        };
        match result {
            Ok(()) => {
                if src.meta.sparse && !method.clones() {
                    // Restores a trailing hole.
                    ftruncate(out, size)?;
                }
                return Ok(method.clones());
            }
            Err(e) if method != Method::ReadWrite && is_unsupported(&e) => {
                ctx.cache.disable(method)
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no copy method of the plan works for this file",
    ))
}

/// Preallocation surfaces ENOSPC before any data moves and cuts fragmentation.
fn prepare_large(
    src: BorrowedFd<'_>,
    out: BorrowedFd<'_>,
    size: u64,
    uncached: bool,
) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{Advice, FallocateFlags, fadvise, fallocate};
        match fallocate(out, FallocateFlags::empty(), 0, size) {
            Ok(()) | Err(Errno::OPNOTSUPP) | Err(Errno::NOSYS) | Err(Errno::INVAL) => {}
            Err(e) => return Err(e.into()),
        }
        let _ = fadvise(src, 0, None, Advice::Sequential);
        let _ = uncached;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = size;
        if uncached {
            let _ = rustix::fs::fcntl_nocache(src, true);
            let _ = rustix::fs::fcntl_nocache(out, true);
        }
    }
    Ok(())
}

thread_local! {
    /// One buffer per worker, grown to the plan's size once.
    static BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn read_write(
    ctx: &FileCtx<'_>,
    src: &Src,
    out: BorrowedFd<'_>,
    regions: &[(u64, u64)],
    meter: &mut crate::copy::pipeline::Meter<'_>,
) -> io::Result<()> {
    BUF.with_borrow_mut(|buf| {
        if buf.len() < ctx.plan.buf_size {
            buf.resize(ctx.plan.buf_size, 0);
        }
        for &(start, end) in regions {
            let mut off = start;
            while off < end {
                ctx.shared.checkpoint()?;
                let want = (end - off).min(buf.len() as u64) as usize;
                let n = match rustix::io::pread(&src.fd, &mut buf[..want], off) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(Errno::INTR) => continue,
                    Err(e) => return Err(e.into()),
                };
                write_at(out, &buf[..n], off)?;
                meter.add(n as u64);
                off += n as u64;
            }
        }
        Ok(())
    })
}

/// Xattrs and ACLs (Linux: `system.posix_acl_*` travel as xattrs), owner, mode, and the times last, so an
/// interrupted copy never carries the source mtime. Failures are warnings: the content is complete.
fn copy_metadata(
    ctx: &FileCtx<'_>,
    src: &Src,
    out: BorrowedFd<'_>,
    cloned: bool,
) -> Vec<io::Error> {
    let mut warnings = Vec::new();
    let m = &src.meta;
    #[cfg(target_os = "macos")]
    if !cloned && let Err(e) = fcopyfile(src.fd.as_fd(), out, libc::COPYFILE_METADATA) {
        warnings.push(e);
    }
    #[cfg(target_os = "linux")]
    {
        let _ = cloned;
        if ctx.caps.xattrs {
            linux::copy_xattrs(src.fd.as_fd(), out, &mut warnings);
        }
    }
    if ctx.caps.posix_perms {
        set_owner(out, m);
        if let Err(e) = rustix::fs::fchmod(
            out,
            Mode::from_raw_mode(m.mode as _) & Mode::from_bits_truncate(0o7777),
        ) {
            warnings.push(e.into());
        }
    }
    if let Err(e) = futimens(out, &timestamps(m)) {
        warnings.push(e.into());
    }
    warnings
}

/// `fcopyfile(3)` for metadata only (ACLs, xattrs, flags, resource forks).
#[cfg(target_os = "macos")]
fn fcopyfile(
    from: BorrowedFd<'_>,
    to: BorrowedFd<'_>,
    flags: libc::copyfile_flags_t,
) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: both fds are open for the call; a null state is allowed.
    if unsafe {
        libc::fcopyfile(
            from.as_raw_fd(),
            to.as_raw_fd(),
            std::ptr::null_mut(),
            flags,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// As root: the source owner. Otherwise only the group, and only when we own the file anyway.
/// Anything else is skipped silently, like `cp -a` as a normal user.
fn set_owner(fd: BorrowedFd<'_>, m: &Meta) {
    use rustix::fs::{Gid, Uid, fchown};
    let (euid, egid) = ids();
    let (uid, gid) = (Uid::from_raw(m.uid), Gid::from_raw(m.gid));
    if euid == 0 {
        let _ = fchown(fd, Some(uid), Some(gid));
    } else if m.uid == euid && m.gid != egid {
        let _ = fchown(fd, None, Some(gid));
    }
}

pub(crate) fn copy_symlink(
    src_dir: &Dir,
    name: &OsStr,
    m: &Meta,
    dst: &Dir,
    dst_name: &OsStr,
) -> io::Result<()> {
    let target = readlinkat(&src_dir.fd, name, Vec::new())?;
    symlinkat(target.as_c_str(), &dst.fd, dst_name)?;
    // Best effort: not every filesystem stores symlink times.
    let _ = utimensat(&dst.fd, dst_name, &timestamps(m), AtFlags::SYMLINK_NOFOLLOW);
    Ok(())
}

fn unsupported(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, msg)
}

/// FIFOs and (as root) devices are recreated; sockets cannot be.
pub(crate) fn copy_special(m: &Meta, dst: &Dir, dst_name: &OsStr) -> io::Result<()> {
    let kind = FileType::from_raw_mode(m.mode as _);
    if kind == FileType::Socket {
        return Err(unsupported("sockets are not copied"));
    }
    #[cfg(target_os = "linux")]
    {
        let perms = Mode::from_raw_mode(m.mode as _) & Mode::from_bits_truncate(0o7777);
        match rustix::fs::mknodat(&dst.fd, dst_name, kind, perms, m.rdev) {
            Ok(()) => {
                let _ = utimensat(&dst.fd, dst_name, &timestamps(m), AtFlags::SYMLINK_NOFOLLOW);
                Ok(())
            }
            Err(Errno::PERM) => Err(unsupported("device files can only be copied as root")),
            Err(e) => Err(e.into()),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        // ponytail: no mknodat in rustix on Apple; add libc::mkfifo/mknod if anyone copies FIFOs on macOS.
        let _ = (dst, dst_name);
        Err(unsupported("special files are not copied on this system"))
    }
}

pub(crate) fn hard_link(existing: &Path, dst: &Dir, dst_name: &OsStr) -> io::Result<()> {
    Ok(linkat(CWD, existing, &dst.fd, dst_name, AtFlags::empty())?)
}

/// Owner and xattrs at creation time (children do not change them); mode and times come in `finish_dir`.
pub(crate) fn copy_dir_attrs(ctx: &FileCtx<'_>, src: &Dir, dst: &Dir, m: &Meta) -> Vec<io::Error> {
    let mut warnings = Vec::new();
    if ctx.caps.posix_perms {
        set_owner(dst.fd.as_fd(), m);
    }
    #[cfg(target_os = "linux")]
    if ctx.caps.xattrs {
        linux::copy_xattrs(src.fd.as_fd(), dst.fd.as_fd(), &mut warnings);
    }
    #[cfg(target_os = "macos")]
    {
        if let Err(e) = fcopyfile(
            src.fd.as_fd(),
            dst.fd.as_fd(),
            libc::COPYFILE_XATTR | libc::COPYFILE_ACL,
        ) {
            warnings.push(e);
        }
    }
    let _ = src;
    warnings
}

/// Post-order: creating children bumped the mtime, and `0700` was only for creating them.
pub(crate) fn finish_dir(path: &Path, m: &Meta, caps: &FsCaps) -> io::Result<()> {
    let chmod = if caps.posix_perms {
        rustix::fs::chmodat(
            CWD,
            path,
            Mode::from_raw_mode(m.mode as _) & Mode::from_bits_truncate(0o7777),
            AtFlags::empty(),
        )
    } else {
        Ok(())
    };
    let times = utimensat(CWD, path, &timestamps(m), AtFlags::empty());
    chmod.and(times).map_err(io::Error::from)
}

/// Atomic no-replace rename: Linux `renameat2(RENAME_NOREPLACE)`, macOS `renamex_np(RENAME_EXCL)`.
/// Filesystems without the flag fall back to check-then-rename, which has a small race.
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    match renameat_with(CWD, from, CWD, to, rustix::fs::RenameFlags::NOREPLACE) {
        Ok(()) => Ok(()),
        Err(Errno::INVAL | Errno::NOSYS | Errno::NOTSUP) => {
            if lstat(to).is_ok() {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            Ok(std::fs::rename(from, to)?)
        }
        Err(e) => Err(e.into()),
    }
}

/// Makes everything copied to the volume of `path` durable before sources are deleted.
/// Linux: one `syncfs`. macOS: files were fsynced by their workers; `F_FULLFSYNC` flushes the drive cache.
pub(crate) fn barrier(path: &Path) -> io::Result<()> {
    let fd = openat(CWD, path, OFlags::RDONLY | OFlags::CLOEXEC, Mode::empty())?;
    #[cfg(target_os = "linux")]
    rustix::fs::syncfs(&fd)?;
    #[cfg(target_os = "macos")]
    if rustix::fs::fcntl_fullfsync(&fd).is_err() {
        rustix::fs::fsync(&fd)?;
    }
    Ok(())
}

pub(crate) fn free_space(path: &Path) -> Option<u64> {
    let st = rustix::fs::statvfs(path).ok()?;
    #[allow(clippy::unnecessary_cast)]
    Some((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
}

/// Physical byte offset of the first extent, for disk-order scheduling. `None` where unknown.
pub(crate) fn extent_key(src: &Src) -> Option<u64> {
    #[cfg(target_os = "linux")]
    return linux::first_extent(src.fd.as_fd());
    #[cfg(not(target_os = "linux"))]
    {
        // ponytail: no F_LOG2PHYS_EXT probe on macOS; TwoPhase falls back to inode order there.
        let _ = src;
        None
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{CHUNK, FileCtx, Src};
    use crate::copy::pipeline::Meter;
    use rustix::fs::{Advice, SeekFrom, copy_file_range, fadvise, seek, sendfile};
    use rustix::io::Errno;
    use std::io;
    use std::num::NonZeroU64;
    use std::os::fd::{AsFd, BorrowedFd};

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) enum Kind {
        CopyFileRange,
        Sendfile,
    }

    /// In-kernel copy of `regions`, looping to EOF for the open-ended dense region.
    pub(super) fn copy_ranges(
        ctx: &FileCtx<'_>,
        src: &Src,
        out: BorrowedFd<'_>,
        regions: &[(u64, u64)],
        meter: &mut Meter<'_>,
        kind: Kind,
        uncached: bool,
    ) -> io::Result<()> {
        let mut copied = 0u64;
        for &(start, end) in regions {
            let mut off = start;
            if kind == Kind::Sendfile {
                // sendfile writes at the output's file position.
                seek(out, SeekFrom::Start(start))?;
            }
            while off < end {
                ctx.shared.checkpoint()?;
                let len = (end - off).min(CHUNK as u64) as usize;
                let before = off;
                let n = match kind {
                    Kind::CopyFileRange => {
                        let mut off_out = off;
                        copy_file_range(&src.fd, Some(&mut off), out, Some(&mut off_out), len)
                    }
                    Kind::Sendfile => sendfile(out, &src.fd, Some(&mut off), len),
                };
                let n = match n {
                    Ok(0) => break,
                    Ok(n) => n as u64,
                    Err(Errno::INTR) => continue,
                    Err(e) => return Err(e.into()),
                };
                // sendfile/copy_file_range advance `off` themselves; keep it exact either way.
                off = before + n;
                copied += n;
                meter.add(n);
                if uncached {
                    let _ = fadvise(&src.fd, before, NonZeroU64::new(n), Advice::DontNeed);
                }
            }
        }
        // 5.3-5.18 returned 0 without copying on some virtual filesystems.
        if copied == 0 && src.meta.size > 0 && kind == Kind::CopyFileRange {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "copy_file_range copied nothing",
            ));
        }
        Ok(())
    }

    /// `security.*` labels belong to the destination's policy, so their failures are ignored.
    pub(super) fn copy_xattrs(
        src: BorrowedFd<'_>,
        out: BorrowedFd<'_>,
        warnings: &mut Vec<io::Error>,
    ) {
        use rustix::fs::{XattrFlags, fgetxattr, flistxattr, fsetxattr};
        let mut names = vec![0u8; 1024];
        let len = loop {
            match flistxattr(src, &mut names[..]) {
                Ok(len) => break len,
                Err(Errno::RANGE) => names.resize(names.len() * 4, 0),
                // No xattrs support on the source filesystem: nothing to copy.
                Err(Errno::OPNOTSUPP | Errno::NOSYS) => return,
                Err(e) => {
                    warnings.push(e.into());
                    return;
                }
            }
        };
        let mut value = vec![0u8; 4096];
        for name in names[..len].split(|&b| b == 0).filter(|n| !n.is_empty()) {
            let Ok(name) =
                std::ffi::CStr::from_bytes_with_nul(&[name, b"\0"].concat()).map(|c| c.to_owned())
            else {
                continue;
            };
            let n = loop {
                match fgetxattr(src, name.as_c_str(), &mut value[..]) {
                    Ok(n) => break Some(n),
                    Err(Errno::RANGE) => value.resize(value.len() * 4, 0),
                    Err(_) => break None,
                }
            };
            let Some(n) = n else { continue };
            match fsetxattr(out, name.as_c_str(), &value[..n], XattrFlags::empty()) {
                Ok(()) => {}
                Err(_) if name.to_bytes().starts_with(b"security.") => {}
                Err(Errno::OPNOTSUPP) => {
                    warnings.push(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "the destination does not support extended attributes",
                    ));
                    return;
                }
                Err(e) => warnings.push(e.into()),
            }
        }
    }

    #[repr(C)]
    struct FiemapExtent {
        logical: u64,
        physical: u64,
        length: u64,
        reserved64: [u64; 2],
        flags: u32,
        reserved: [u32; 3],
    }

    /// `struct fiemap` with room for exactly one extent.
    #[repr(C)]
    struct Fiemap {
        start: u64,
        length: u64,
        flags: u32,
        mapped_extents: u32,
        extent_count: u32,
        reserved: u32,
        extents: [FiemapExtent; 1],
    }

    /// `_IOWR('f', 11, struct fiemap)`; the size in the opcode is the 32-byte header only.
    const FS_IOC_FIEMAP: rustix::ioctl::Opcode =
        rustix::ioctl::opcode::read_write::<[u8; 32]>(b'f', 11);

    pub(super) fn first_extent(fd: BorrowedFd<'_>) -> Option<u64> {
        let mut fm = Fiemap {
            start: 0,
            length: u64::MAX,
            flags: 0,
            mapped_extents: 0,
            extent_count: 1,
            reserved: 0,
            extents: [FiemapExtent {
                logical: 0,
                physical: 0,
                length: 0,
                reserved64: [0; 2],
                flags: 0,
                reserved: [0; 3],
            }],
        };
        // SAFETY: FS_IOC_FIEMAP reads the header and writes at most `extent_count` extents into `fm`.
        let ok = unsafe {
            let ioctl = rustix::ioctl::Updater::<FS_IOC_FIEMAP, Fiemap>::new(&mut fm);
            rustix::ioctl::ioctl(fd.as_fd(), ioctl)
        };
        (ok.is_ok() && fm.mapped_extents > 0).then_some(fm.extents[0].physical)
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{Dir, FileCtx, Src, timestamps};
    use crate::copy::Method;
    use crate::copy::pipeline::{Copied, is_unsupported};
    use rustix::fs::{AtFlags, unlinkat, utimensat};
    use rustix::fs::{CloneFlags, fclonefileat};
    use std::ffi::OsStr;
    use std::io;

    /// `fclonefileat` creates the destination itself, with data and metadata, on the same APFS volume.
    /// `Ok(None)`: not available, go on with the chain.
    pub(super) fn try_clone(
        ctx: &FileCtx<'_>,
        src: &Src,
        dst: &Dir,
        dst_name: &OsStr,
    ) -> io::Result<Option<Copied>> {
        if !ctx.plan.methods.contains(&Method::CloneFileAt)
            || !ctx.cache.enabled(Method::CloneFileAt)
        {
            return Ok(None);
        }
        ctx.shared.checkpoint()?;
        match fclonefileat(&src.fd, &dst.fd, dst_name, CloneFlags::NOFOLLOW) {
            Ok(()) => {
                let finish = || -> io::Result<Vec<io::Error>> {
                    let mut warnings = Vec::new();
                    // The clone carries the source times already; this only covers volumes that do not.
                    if let Err(e) = utimensat(
                        &dst.fd,
                        dst_name,
                        &timestamps(&src.meta),
                        AtFlags::SYMLINK_NOFOLLOW,
                    ) {
                        warnings.push(e.into());
                    }
                    if ctx.durable {
                        rustix::fs::fsync(dst.open_file_for_sync(dst_name)?)?;
                    }
                    Ok(warnings)
                };
                match finish() {
                    Ok(warnings) => {
                        ctx.meter().add(src.meta.size);
                        Ok(Some(Copied {
                            cloned: true,
                            warnings,
                        }))
                    }
                    Err(e) => {
                        let _ = unlinkat(&dst.fd, dst_name, AtFlags::empty());
                        Err(e)
                    }
                }
            }
            Err(e) => {
                let e = io::Error::from(e);
                if e.kind() == io::ErrorKind::AlreadyExists || !is_unsupported(&e) {
                    return Err(e);
                }
                ctx.cache.disable(Method::CloneFileAt);
                Ok(None)
            }
        }
    }

    impl Dir {
        fn open_file_for_sync(&self, name: &OsStr) -> io::Result<std::os::fd::OwnedFd> {
            use rustix::fs::{Mode, OFlags, openat};
            Ok(openat(
                &self.fd,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::copy::plan::MethodCache;
    use crate::copy::progress::{Shared, is_cancelled};
    use crate::copy::{CopyPlan, Durability, Schedule};
    use crate::volume::FsKind;

    fn plan(methods: &'static [Method]) -> CopyPlan {
        CopyPlan {
            workers: 1,
            schedule: Schedule::Streaming,
            methods,
            buf_size: 4096,
            // Every file takes the large-file, uncached path.
            large_file: 1,
            uncached_from: Some(1),
            durability: Durability::BeforeSourceDelete,
        }
    }

    fn copy_one(ctx: &FileCtx<'_>, dir: &Path, name: &str, data: &[u8]) -> io::Result<Copied> {
        std::fs::write(dir.join(name), data).unwrap();
        let (src_dir, dst_dir) = (
            Dir::open(dir).unwrap(),
            Dir::open(&dir.join("out")).unwrap(),
        );
        let src = open_file(&src_dir, name.as_ref()).unwrap();
        copy_file(
            ctx,
            &src,
            &src_dir,
            name.as_ref(),
            &dst_dir,
            name.as_ref(),
            None,
        )
    }

    #[test]
    fn chain_falls_back_and_caches_unsupported_methods() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("out")).unwrap();
        let sh = Shared::new(Box::new(|| {}));
        let cache = MethodCache::default();
        let plan = plan(&[
            Method::CloneFileAt,
            Method::Ficlone,
            Method::CopyFileRange,
            Method::Sendfile,
            Method::ReadWrite,
        ]);
        let ctx = FileCtx {
            plan: &plan,
            cache: &cache,
            shared: &sh,
            caps: FsCaps::for_fs(FsKind::Ext4),
            durable: true,
        };
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let copied = copy_one(&ctx, t.path(), "a", &data).unwrap();
        assert_eq!(std::fs::read(t.path().join("out/a")).unwrap(), data);
        if !copied.cloned {
            assert!(
                !cfg!(target_os = "linux") || !cache.enabled(Method::Ficlone),
                "an unsupported clone is cached"
            );
        }
        copy_one(&ctx, t.path(), "b", b"second").unwrap();
        assert_eq!(std::fs::read(t.path().join("out/b")).unwrap(), b"second");
        assert_eq!(
            copy_one(&ctx, t.path(), "b", b"again").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            std::fs::read(t.path().join("out/b")).unwrap(),
            b"second",
            "an existing file is never touched"
        );
    }

    #[test]
    fn a_cancelled_copy_removes_its_partial_file() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("out")).unwrap();
        let sh = Shared::new(Box::new(|| {}));
        sh.cancel();
        let cache = MethodCache::default();
        let plan = plan(&[Method::ReadWrite]);
        let ctx = FileCtx {
            plan: &plan,
            cache: &cache,
            shared: &sh,
            caps: FsCaps::for_fs(FsKind::Ext4),
            durable: false,
        };
        let err = copy_one(&ctx, t.path(), "a", &[7u8; 10_000]).unwrap_err();
        assert!(is_cancelled(&err));
        assert!(!t.path().join("out/a").exists());
        assert_eq!(sh.snapshot().bytes_done, 0);
    }

    #[test]
    fn regions_of_a_sparse_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sparse");
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(8 << 20).unwrap();
        std::os::unix::fs::FileExt::write_all_at(&f, b"data", 4 << 20).unwrap();
        let regions = data_regions(f.as_fd(), 8 << 20).unwrap();
        // Filesystems without SEEK_DATA report one dense region; real ones report the data block only.
        assert!(!regions.is_empty());
        assert!(
            regions == [(0, u64::MAX)] || regions.iter().all(|&(s, e)| s <= 4 << 20 && e > 4 << 20),
            "{regions:?}"
        );
    }
}
