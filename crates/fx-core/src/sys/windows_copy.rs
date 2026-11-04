//! Windows copy backend: the portable `std_copy` walk plus `CopyFile2` (ADS, attributes, timestamps,
//! SMB copychunk/ODX, block clone on 24H2+), ReFS `FSCTL_DUPLICATE_EXTENTS_TO_FILE`, a sparse-aware path,
//! `MoveFileExW` no-replace renames and `FlushFileBuffers` durability for moves.

pub(crate) use super::std_copy::{
    Dir, Src, copy_dir_attrs, copy_special, copy_symlink, hard_link, lstat, open_file,
    raise_fd_limit, read_all,
};

use super::windows::wide_path;
use crate::copy::pipeline::{Copied, FileCtx, Meter, is_unsupported};
use crate::copy::{Meta, Method, progress::cancelled};
use crate::volume::FsCaps;
use std::ffi::{OsStr, c_void};
use std::fs::{self, File, FileTimes, OpenOptions};
use std::io;
use std::os::windows::fs::{FileExt, FileTimesExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::{mem, ptr};
use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_REQUEST_ABORTED, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{
    COPY_FILE_COPY_SYMLINK, COPY_FILE_FAIL_IF_EXISTS, COPY_FILE_NO_BUFFERING,
    COPYFILE2_CALLBACK_CHUNK_FINISHED, COPYFILE2_EXTENDED_PARAMETERS, COPYFILE2_MESSAGE,
    COPYFILE2_MESSAGE_ACTION, COPYFILE2_PROGRESS_CANCEL, COPYFILE2_PROGRESS_CONTINUE, CopyFile2,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_WRITE_ATTRIBUTES, GetDiskFreeSpaceExW, MoveFileExW,
    SetFileAttributesW,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    DUPLICATE_EXTENTS_DATA, FILE_ALLOCATED_RANGE_BUFFER, FSCTL_DUPLICATE_EXTENTS_TO_FILE,
    FSCTL_GET_INTEGRITY_INFORMATION, FSCTL_GET_INTEGRITY_INFORMATION_BUFFER,
    FSCTL_GET_RETRIEVAL_POINTERS, FSCTL_QUERY_ALLOCATED_RANGES, FSCTL_SET_INTEGRITY_INFORMATION,
    FSCTL_SET_INTEGRITY_INFORMATION_BUFFER, FSCTL_SET_SPARSE, RETRIEVAL_POINTERS_BUFFER,
    STARTING_VCN_INPUT_BUFFER,
};

/// Attributes a copy carries over; the rest (sparse, compressed, reparse) are set by other means.
const COPIED_ATTRIBUTES: u32 = 0x1 | 0x2 | 0x4 | 0x20 | 0x2000;
/// `FSCTL_DUPLICATE_EXTENTS_TO_FILE` clones at most this much per call (4 GiB minus one 64 KiB cluster).
const MAX_CLONE_RANGE: u64 = (4 << 30) - (64 << 10);
const CHUNK: usize = 16 * 1024 * 1024;

/// `DeviceIoControl` with typed in/out buffers. `ERROR_MORE_DATA` counts as success (partial output).
fn ioctl<I, O>(h: HANDLE, code: u32, input: Option<&I>, output: Option<&mut O>) -> io::Result<u32> {
    let (inp, in_len) = input.map_or((ptr::null(), 0), |i| {
        (
            ptr::from_ref(i).cast::<c_void>(),
            mem::size_of::<I>() as u32,
        )
    });
    let (out, out_len) = output.map_or((ptr::null_mut(), 0), |o| {
        (
            ptr::from_mut(o).cast::<c_void>(),
            mem::size_of::<O>() as u32,
        )
    });
    let mut returned = 0u32;
    // SAFETY: buffers are valid for the sizes passed; no overlapped I/O.
    let ok = unsafe {
        DeviceIoControl(
            h,
            code,
            inp,
            in_len,
            out,
            out_len,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() != Some(ERROR_MORE_DATA as i32) {
            return Err(e);
        }
    }
    Ok(returned)
}

fn raw(f: &File) -> HANDLE {
    f.as_raw_handle() as HANDLE
}

/// Data via the plan's chain; `AlreadyExists` means the name is taken and nothing was touched.
pub(crate) fn copy_file(
    ctx: &FileCtx<'_>,
    src: &Src,
    src_dir: &Dir,
    name: &OsStr,
    dst: &Dir,
    dst_name: &OsStr,
    prefetched: Option<&[u8]>,
) -> io::Result<Copied> {
    if prefetched.is_some() {
        return super::std_copy::copy_file(ctx, src, src_dir, name, dst, dst_name, prefetched);
    }
    let dst_path = dst.path().join(dst_name);
    if src.meta.sparse {
        // CopyFile2 is not documented to keep holes, so sparse files take the range-aware path.
        return created(ctx, &dst_path, sparse_copy(ctx, src, &dst_path));
    }
    for &method in ctx.plan.methods {
        if !ctx.cache.enabled(method) {
            continue;
        }
        let result = match method {
            Method::CopyFile2 => created(ctx, &dst_path, copy_file2(ctx, src, &dst_path)),
            Method::DuplicateExtents => {
                created(ctx, &dst_path, duplicate_extents(ctx, src, &dst_path))
            }
            Method::ReadWrite => {
                return super::std_copy::copy_file(ctx, src, src_dir, name, dst, dst_name, None);
            }
            _ => continue,
        };
        match result {
            Err(e) if e.kind() != io::ErrorKind::AlreadyExists && is_unsupported(&e) => {
                ctx.cache.disable(method)
            }
            other => return other,
        }
    }
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no copy method of the plan works for this file",
    ))
}

/// Shared tail of the methods that create `dst` themselves: durability for moves, cleanup on failure.
/// The destination did not exist before (exclusive create), so on any error but `AlreadyExists` it is ours to remove.
fn created(ctx: &FileCtx<'_>, dst: &Path, result: io::Result<bool>) -> io::Result<Copied> {
    let result = result.and_then(|cloned| {
        if ctx.durable {
            OpenOptions::new().write(true).open(dst)?.sync_all()?;
        }
        Ok(Copied {
            cloned,
            warnings: Vec::new(),
        })
    });
    if let Err(e) = &result
        && e.kind() != io::ErrorKind::AlreadyExists
    {
        let _ = fs::remove_file(dst);
    }
    result
}

struct Callback<'a, 'b> {
    ctx: &'a FileCtx<'b>,
    meter: Meter<'a>,
    reported: u64,
}

unsafe extern "system" fn progress(
    msg: *const COPYFILE2_MESSAGE,
    context: *const c_void,
) -> COPYFILE2_MESSAGE_ACTION {
    // SAFETY: `context` is the `Callback` passed to CopyFile2 below, alive for the whole call;
    // CopyFile2 invokes the routine on the calling thread only.
    let cb = unsafe { &mut *(context as *mut Callback<'_, '_>) };
    // SAFETY: CopyFile2 passes a valid message; the union member matches its type.
    let msg = unsafe { &*msg };
    if msg.Type == COPYFILE2_CALLBACK_CHUNK_FINISHED {
        // SAFETY: ChunkFinished is the active member for this message type.
        let total = unsafe { msg.Info.ChunkFinished.uliTotalBytesTransferred };
        cb.meter.add(total.saturating_sub(cb.reported));
        cb.reported = total;
    }
    // Blocks here while paused, so pause works mid-file (NO_BUFFERING copies included).
    match cb.ctx.shared.checkpoint() {
        Ok(()) => COPYFILE2_PROGRESS_CONTINUE,
        Err(_) => COPYFILE2_PROGRESS_CANCEL,
    }
}

fn copy_file2(ctx: &FileCtx<'_>, src: &Src, dst: &Path) -> io::Result<bool> {
    ctx.shared.checkpoint()?;
    let (from, to) = (wide_path(&src.path), wide_path(dst));
    let mut flags = COPY_FILE_FAIL_IF_EXISTS | COPY_FILE_COPY_SYMLINK;
    if ctx
        .plan
        .uncached_from
        .is_some_and(|limit| src.meta.size >= limit)
    {
        flags |= COPY_FILE_NO_BUFFERING;
    }
    let mut cb = Callback {
        ctx,
        meter: ctx.meter(),
        reported: 0,
    };
    let params = COPYFILE2_EXTENDED_PARAMETERS {
        dwSize: mem::size_of::<COPYFILE2_EXTENDED_PARAMETERS>() as u32,
        dwCopyFlags: flags,
        pfCancel: ptr::null_mut(),
        pProgressRoutine: Some(progress),
        pvCallbackContext: ptr::from_mut(&mut cb).cast(),
    };
    // SAFETY: both paths are NUL-terminated; `params` and `cb` outlive the call.
    let hr = unsafe { CopyFile2(from.as_ptr(), to.as_ptr(), &params) };
    if hr >= 0 {
        // Small files may finish without a chunk callback.
        cb.meter.add(src.meta.size.saturating_sub(cb.reported));
        return Ok(false);
    }
    cb.meter.rollback();
    // HRESULT_FROM_WIN32: facility 7 carries the Win32 code in the low word.
    let code = if (hr as u32) & 0xFFFF_0000 == 0x8007_0000 {
        (hr as u32 & 0xFFFF) as i32
    } else {
        hr
    };
    if code == ERROR_REQUEST_ABORTED as i32 && ctx.shared.is_cancelled() {
        return Err(cancelled());
    }
    Err(io::Error::from_raw_os_error(code))
}

/// Exclusive create plus the source's times and attributes, for the paths that write data themselves.
fn create_like(src: &Src, dst: &Path) -> io::Result<(File, File)> {
    let input = File::open(&src.path)?;
    let out = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(dst)?;
    Ok((input, out))
}

fn finish_like(src: &Src, input: &File, out: &File, dst: &Path) -> io::Result<()> {
    let md = input.metadata()?;
    let mut times = FileTimes::new()
        .set_accessed(md.accessed()?)
        .set_modified(md.modified()?);
    if let Ok(created) = md.created() {
        times = times.set_created(created);
    }
    out.set_times(times)?;
    set_attributes(dst, src.meta.mode)
}

fn set_attributes(path: &Path, attributes: u32) -> io::Result<()> {
    let w = wide_path(path);
    // SAFETY: `w` is NUL-terminated.
    if unsafe { SetFileAttributesW(w.as_ptr(), attributes & COPIED_ATTRIBUTES) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Allocated ranges of `f` (`FSCTL_QUERY_ALLOCATED_RANGES`), queried in batches.
fn allocated_ranges(f: &File, size: u64) -> io::Result<Vec<(u64, u64)>> {
    let mut ranges = Vec::new();
    let mut from = 0u64;
    loop {
        let query = FILE_ALLOCATED_RANGE_BUFFER {
            FileOffset: from as i64,
            Length: (size - from) as i64,
        };
        let mut out = [FILE_ALLOCATED_RANGE_BUFFER {
            FileOffset: 0,
            Length: 0,
        }; 64];
        let returned = ioctl(
            raw(f),
            FSCTL_QUERY_ALLOCATED_RANGES,
            Some(&query),
            Some(&mut out),
        )?;
        let n = returned as usize / mem::size_of::<FILE_ALLOCATED_RANGE_BUFFER>();
        for r in &out[..n] {
            ranges.push((r.FileOffset as u64, (r.FileOffset + r.Length) as u64));
        }
        match out[..n].last() {
            Some(last) if n == out.len() && ((last.FileOffset + last.Length) as u64) < size => {
                from = (last.FileOffset + last.Length) as u64;
            }
            _ => return Ok(ranges),
        }
    }
}

fn sparse_copy(ctx: &FileCtx<'_>, src: &Src, dst: &Path) -> io::Result<bool> {
    let (input, out) = create_like(src, dst)?;
    ioctl::<(), ()>(raw(&out), FSCTL_SET_SPARSE, None, None)?;
    let size = src.meta.size;
    out.set_len(size)?;
    let mut meter = ctx.meter();
    let result = (|| {
        let mut buf = vec![0u8; CHUNK.min(ctx.plan.buf_size.max(64 << 10))];
        for (start, end) in allocated_ranges(&input, size)? {
            let mut off = start;
            while off < end {
                ctx.shared.checkpoint()?;
                let want = (end - off).min(buf.len() as u64) as usize;
                let n = input.seek_read(&mut buf[..want], off)?;
                if n == 0 {
                    break;
                }
                let mut written = 0;
                while written < n {
                    written += out.seek_write(&buf[written..n], off + written as u64)?;
                }
                meter.add(n as u64);
                off += n as u64;
            }
        }
        finish_like(src, &input, &out, dst)
    })();
    if result.is_err() {
        meter.rollback();
    }
    result.map(|()| false)
}

/// ReFS block cloning: same volume, cluster-aligned ranges, matching integrity-stream settings.
fn duplicate_extents(ctx: &FileCtx<'_>, src: &Src, dst: &Path) -> io::Result<bool> {
    ctx.shared.checkpoint()?;
    let (input, out) = create_like(src, dst)?;
    let mut integrity = FSCTL_GET_INTEGRITY_INFORMATION_BUFFER::default();
    ioctl::<(), _>(
        raw(&input),
        FSCTL_GET_INTEGRITY_INFORMATION,
        None,
        Some(&mut integrity),
    )?;
    let set = FSCTL_SET_INTEGRITY_INFORMATION_BUFFER {
        ChecksumAlgorithm: integrity.ChecksumAlgorithm,
        Reserved: 0,
        Flags: integrity.Flags,
    };
    ioctl::<_, ()>(raw(&out), FSCTL_SET_INTEGRITY_INFORMATION, Some(&set), None)?;
    let size = src.meta.size;
    out.set_len(size)?;
    let cluster = u64::from(integrity.ClusterSizeInBytes.max(4096));
    // The last range may run to the cluster boundary past EOF; EOF itself stays at `size`.
    let total = size.div_ceil(cluster) * cluster;
    let mut off = 0;
    while off < total {
        let len = (total - off).min(MAX_CLONE_RANGE / cluster * cluster);
        let data = DUPLICATE_EXTENTS_DATA {
            FileHandle: raw(&input),
            SourceFileOffset: off as i64,
            TargetFileOffset: off as i64,
            ByteCount: len as i64,
        };
        ioctl::<_, ()>(
            raw(&out),
            FSCTL_DUPLICATE_EXTENTS_TO_FILE,
            Some(&data),
            None,
        )?;
        off += len;
    }
    ctx.meter().add(size);
    finish_like(src, &input, &out, dst)?;
    Ok(true)
}

/// Post-order times and attributes; a directory handle needs `FILE_FLAG_BACKUP_SEMANTICS`.
pub(crate) fn finish_dir(path: &Path, m: &Meta, _caps: &FsCaps) -> io::Result<()> {
    let dir = OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let mut times = FileTimes::new();
    if let Some(t) = m.atime.system_time() {
        times = times.set_accessed(t);
    }
    if let Some(t) = m.mtime.system_time() {
        times = times.set_modified(t);
    }
    dir.set_times(times)?;
    set_attributes(path, m.mode)
}

/// `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` fails atomically if `to` exists.
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    let (from, to) = (wide_path(from), wide_path(to));
    // SAFETY: both paths are NUL-terminated.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Every file of a move was flushed with `FlushFileBuffers` by its worker.
pub(crate) fn barrier(_path: &Path) -> io::Result<()> {
    Ok(())
}

pub(crate) fn free_space(path: &Path) -> Option<u64> {
    let w = wide_path(path);
    let mut free = 0u64;
    // SAFETY: `w` is NUL-terminated; unused out-pointers are null.
    let ok =
        unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut free, ptr::null_mut(), ptr::null_mut()) };
    (ok != 0).then_some(free)
}

/// Logical cluster of the first extent (`FSCTL_GET_RETRIEVAL_POINTERS`); `None` for MFT-resident files.
pub(crate) fn extent_key(src: &Src) -> Option<u64> {
    let f = File::open(&src.path).ok()?;
    let input = STARTING_VCN_INPUT_BUFFER { StartingVcn: 0 };
    let mut out = RETRIEVAL_POINTERS_BUFFER::default();
    ioctl(
        raw(&f),
        FSCTL_GET_RETRIEVAL_POINTERS,
        Some(&input),
        Some(&mut out),
    )
    .ok()?;
    let lcn = out.Extents[0].Lcn;
    (out.ExtentCount > 0 && lcn >= 0).then_some(lcn as u64)
}
