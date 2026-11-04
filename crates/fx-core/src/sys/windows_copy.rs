//! Windows copy backend: the portable `std_copy` walk plus `CopyFile2` (ADS, attributes, timestamps,
//! SMB copychunk/ODX, block clone on 24H2+), ReFS `FSCTL_DUPLICATE_EXTENTS_TO_FILE`, a sparse-aware path,
//! `MoveFileExW` no-replace renames and `FlushFileBuffers` durability for moves.

pub(crate) use super::std_copy::{
    Dir, Src, copy_dir_attrs, copy_special, hard_link, lstat, raise_fd_limit, read_all,
};

use super::windows::wide_path;
use crate::EntryKind;
use crate::copy::pipeline::{Copied, FileCtx, Meter, is_unsupported};
use crate::copy::{Meta, Method, progress::cancelled};
use crate::volume::FsCaps;
use std::ffi::{OsStr, OsString, c_void};
use std::fs::{self, File, FileTimes, OpenOptions};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::{FileExt, FileTimesExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf};
use std::{mem, ptr};
use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_REQUEST_ABORTED, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, COPY_FILE_COPY_SYMLINK, COPY_FILE_FAIL_IF_EXISTS,
    COPY_FILE_NO_BUFFERING, COPYFILE2_CALLBACK_CHUNK_FINISHED, COPYFILE2_EXTENDED_PARAMETERS,
    COPYFILE2_MESSAGE, COPYFILE2_MESSAGE_ACTION, COPYFILE2_PROGRESS_CANCEL,
    COPYFILE2_PROGRESS_CONTINUE, CopyFile2, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_WRITE_ATTRIBUTES,
    FileIdInfo, GetDiskFreeSpaceExW, GetFileInformationByHandle, GetFileInformationByHandleEx,
    MoveFileExW, SetFileAttributesW,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    DUPLICATE_EXTENTS_DATA, FILE_ALLOCATED_RANGE_BUFFER, FSCTL_DUPLICATE_EXTENTS_TO_FILE,
    FSCTL_GET_INTEGRITY_INFORMATION, FSCTL_GET_INTEGRITY_INFORMATION_BUFFER,
    FSCTL_GET_REPARSE_POINT, FSCTL_GET_RETRIEVAL_POINTERS, FSCTL_QUERY_ALLOCATED_RANGES,
    FSCTL_SET_INTEGRITY_INFORMATION, FSCTL_SET_INTEGRITY_INFORMATION_BUFFER,
    FSCTL_SET_REPARSE_POINT, FSCTL_SET_SPARSE, RETRIEVAL_POINTERS_BUFFER,
    STARTING_VCN_INPUT_BUFFER,
};

/// Attributes a copy carries over; the rest (sparse, compressed, reparse) are set by other means.
const COPIED_ATTRIBUTES: u32 = 0x1 | 0x2 | 0x4 | 0x20 | 0x2000;
/// `FSCTL_DUPLICATE_EXTENTS_TO_FILE` clones at most this much per call (4 GiB minus one 64 KiB cluster).
const MAX_CLONE_RANGE: u64 = (4 << 30) - (64 << 10);
/// Files below this size may take the experimental raw path.
const RAW_SMALL_MAX: u64 = 64 * 1024;
const CHUNK: usize = 16 * 1024 * 1024;
const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
const GENERIC_WRITE: u32 = 0x4000_0000;
const SHARE_ALL: u32 = 0x7;
/// Header of a `REPARSE_DATA_BUFFER` (tag, data length, reserved) and of its mount-point part
/// (substitute and print name offset and length), both before the name characters.
const REPARSE_HEADER: usize = 8;
const MOUNT_POINT_HEADER: usize = 8;
const MAX_REPARSE_SIZE: usize = 16 * 1024;
/// How the NT namespace spells a Win32 path in a reparse point.
const NT_PREFIX: &str = r"\??\";

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
    // The methods below open the source before they create the destination, so a source error must
    // not be mistaken for "we created this file": an existing name is reported, and never cleaned up.
    if fs::symlink_metadata(&dst_path).is_ok() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    if src.meta.sparse {
        // CopyFile2 is not documented to keep holes, so sparse files take the range-aware path;
        // destinations without sparse support (FAT, exFAT, shares) fall back to the dense chain.
        match created(ctx, &dst_path, sparse_copy(ctx, src, &dst_path)) {
            Err(e) if e.kind() == io::ErrorKind::Unsupported => {}
            other => return other,
        }
    }
    for method in ctx.plan.chain() {
        if !ctx.cache.enabled(method) {
            continue;
        }
        let result = match method {
            Method::CopyFile2 => created(ctx, &dst_path, copy_file2(ctx, src, &dst_path)),
            Method::DuplicateExtents => {
                created(ctx, &dst_path, duplicate_extents(ctx, src, &dst_path))
            }
            Method::RawSmall if src.meta.size < RAW_SMALL_MAX => {
                created(ctx, &dst_path, raw_small(ctx, src, &dst_path))
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
/// `copy_file` checked that `dst` did not exist, and every method creates it exclusively, so on any
/// error but `AlreadyExists` whatever is at `dst` is ours to remove.
fn created(ctx: &FileCtx<'_>, dst: &Path, result: io::Result<bool>) -> io::Result<Copied> {
    let result = result.and_then(|cloned| {
        if ctx.durable {
            flush(dst)?;
        }
        Ok(Copied {
            cloned,
            warnings: Vec::new(),
        })
    });
    if let Err(e) = &result
        && e.kind() != io::ErrorKind::AlreadyExists
    {
        let _ = remove_ours(dst);
    }
    result
}

/// `FlushFileBuffers` needs write access, which the read-only attribute (copied from the source) denies.
#[allow(clippy::permissions_set_readonly_false)]
fn flush(path: &Path) -> io::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    let read_only = perms.readonly();
    if read_only {
        perms.set_readonly(false);
        fs::set_permissions(path, perms.clone())?;
    }
    let flushed = OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|f| f.sync_all());
    if read_only {
        perms.set_readonly(true);
        fs::set_permissions(path, perms)?;
    }
    flushed
}

/// Removes a partial copy, clearing the read-only attribute first.
#[allow(clippy::permissions_set_readonly_false)]
fn remove_ours(path: &Path) -> io::Result<()> {
    if let Ok(md) = fs::symlink_metadata(path)
        && md.permissions().readonly()
    {
        let mut perms = md.permissions();
        perms.set_readonly(false);
        let _ = fs::set_permissions(path, perms);
    }
    fs::remove_file(path)
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
    let unsupported = |e: io::Error| io::Error::new(io::ErrorKind::Unsupported, e);
    ioctl::<(), ()>(raw(&out), FSCTL_SET_SPARSE, None, None).map_err(unsupported)?;
    let size = src.meta.size;
    let ranges = allocated_ranges(&input, size).map_err(unsupported)?;
    out.set_len(size)?;
    let mut meter = ctx.meter();
    let result = (|| {
        let mut buf = vec![0u8; CHUNK.min(ctx.plan.buf_size.max(64 << 10))];
        for (start, end) in ranges {
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

/// Experimental (issue #22): one read and one write for a small file, plus times and attributes.
/// Unlike `CopyFile2` it does not carry alternate data streams or security descriptors.
fn raw_small(ctx: &FileCtx<'_>, src: &Src, dst: &Path) -> io::Result<bool> {
    ctx.shared.checkpoint()?;
    let size = src.meta.size as usize;
    let (input, out) = create_like(src, dst)?;
    let mut meter = ctx.meter();
    let result = (|| {
        let mut buf = vec![0u8; size];
        let mut read = 0;
        while read < size {
            match input.seek_read(&mut buf[read..], read as u64)? {
                0 => {
                    return Err(io::Error::other(
                        "the file changed while it was being copied",
                    ));
                }
                n => read += n,
            }
        }
        let mut written = 0;
        while written < size {
            written += out.seek_write(&buf[written..], written as u64)?;
        }
        meter.add(size as u64);
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

// ---- Hardlinks -----------------------------------------------------------------------------------

/// Like the portable `open_file`, plus the volume, file ID and link count that let the engine keep
/// hardlink groups. Windows reports them only through a handle, so each file costs one attribute-only
/// open; the file ID query that guards against ReFS runs only for files with more than one name.
pub(crate) fn open_file(dir: &Dir, name: &OsStr) -> io::Result<Src> {
    let mut src = super::std_copy::open_file(dir, name)?;
    if src.meta.kind == EntryKind::File
        && let Some((volume, index, links)) = file_identity(&src.path)
    {
        src.meta.dev = volume;
        src.meta.ino = index;
        src.meta.nlink = links;
    }
    Ok(src)
}

/// `(volume serial, file index, link count)`. A link count above 1 is only reported together with an ID
/// that is unique on its volume: ReFS IDs need 128 bits, and two files must never be taken for one.
fn file_identity(path: &Path) -> Option<(u64, u64, u64)> {
    let f = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .ok()?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `info` is a valid out-buffer and the handle is open.
    if unsafe { GetFileInformationByHandle(raw(&f), &mut info) } == 0 {
        return None;
    }
    let volume = u64::from(info.dwVolumeSerialNumber);
    if info.nNumberOfLinks <= 1 {
        return Some((volume, 0, 1));
    }
    let mut id = FILE_ID_INFO::default();
    // SAFETY: `id` is a valid out-buffer of the size passed and the handle is open.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            raw(&f),
            FileIdInfo,
            ptr::from_mut(&mut id).cast(),
            mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    let (low, high) = id.FileId.Identifier.split_at(8);
    if ok == 0 || high.iter().any(|&b| b != 0) {
        return Some((volume, 0, 1));
    }
    let index = u64::from_le_bytes(low.try_into().ok()?);
    Some((id.VolumeSerialNumber, index, u64::from(info.nNumberOfLinks)))
}

// ---- Junctions -----------------------------------------------------------------------------------

/// The target of a junction (`IO_REPARSE_TAG_MOUNT_POINT`) without the `\??\` prefix; `None` for
/// anything else, volume mount points (`\??\Volume{...}`) included, which take the portable path.
fn junction_target(path: &Path) -> Option<PathBuf> {
    let f = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let mut buf = [0u8; MAX_REPARSE_SIZE];
    ioctl::<(), _>(raw(&f), FSCTL_GET_REPARSE_POINT, None, Some(&mut buf)).ok()?;
    parse_junction(&buf)
}

fn parse_junction(buf: &[u8]) -> Option<PathBuf> {
    let u16_at = |at: usize| -> Option<usize> {
        Some(usize::from(u16::from_le_bytes(
            buf.get(at..at + 2)?.try_into().ok()?,
        )))
    };
    if u32::from_le_bytes(buf.get(..4)?.try_into().ok()?) != IO_REPARSE_TAG_MOUNT_POINT {
        return None;
    }
    let (offset, len) = (u16_at(REPARSE_HEADER)?, u16_at(REPARSE_HEADER + 2)?);
    let start = REPARSE_HEADER + MOUNT_POINT_HEADER + offset;
    let (pairs, _) = buf.get(start..start + len)?.as_chunks::<2>();
    let wide: Vec<u16> = pairs.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let name = OsString::from_wide(&wide).into_string().ok()?;
    let target = name.strip_prefix(NT_PREFIX)?;
    // Only drive paths are real targets here.
    (target.as_bytes().get(1) == Some(&b':')).then(|| PathBuf::from(target))
}

fn junction_buffer(target: &Path) -> Vec<u8> {
    let print: Vec<u16> = target.as_os_str().encode_wide().collect();
    let substitute: Vec<u16> = NT_PREFIX
        .encode_utf16()
        .chain(print.iter().copied())
        .collect();
    let (sub_bytes, print_bytes) = (substitute.len() * 2, print.len() * 2);
    // Two names, each followed by a NUL character.
    let data_len = MOUNT_POINT_HEADER + sub_bytes + print_bytes + 4;
    let mut buf = Vec::with_capacity(REPARSE_HEADER + data_len);
    buf.extend_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    buf.extend_from_slice(&(data_len as u16).to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    for field in [0, sub_bytes, sub_bytes + 2, print_bytes] {
        buf.extend_from_slice(&(field as u16).to_le_bytes());
    }
    for unit in substitute.iter().chain(&[0]).chain(&print).chain(&[0]) {
        buf.extend_from_slice(&unit.to_le_bytes());
    }
    buf
}

/// Creates `link` (which must not exist) as a junction to `target`.
fn create_junction(link: &Path, target: &Path) -> io::Result<()> {
    let buf = junction_buffer(target);
    fs::create_dir(link)?;
    let result = (|| {
        let f = OpenOptions::new()
            .access_mode(GENERIC_WRITE)
            .share_mode(SHARE_ALL)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(link)?;
        let mut returned = 0u32;
        // SAFETY: `buf` is valid for its length, the handle is open, and there is no overlapped I/O.
        let ok = unsafe {
            DeviceIoControl(
                raw(&f),
                FSCTL_SET_REPARSE_POINT,
                buf.as_ptr().cast(),
                buf.len() as u32,
                ptr::null_mut(),
                0,
                &mut returned,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir(link);
    }
    result
}

/// Path components compared the way Windows compares names.
fn folded(p: &Path) -> Vec<String> {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

/// Where a junction in `src_dir` points in the copy at `dst_dir`: a target inside the copied tree
/// (below the one of the `roots` that holds `src_dir`) follows the tree to its new place; any other
/// target is kept. The copy's root is derived from `dst_dir`, so a renamed top-level folder
/// (`name (2)`) is followed too.
fn rebased_target(roots: &[PathBuf], src_dir: &Path, dst_dir: &Path, target: &Path) -> PathBuf {
    let absolute = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let (src_dir, dst_dir) = (absolute(src_dir), absolute(dst_dir));
    let (dir_names, target_names) = (folded(&src_dir), folded(target));
    for root in roots {
        let root_names = folded(&absolute(root));
        if !dir_names.starts_with(&root_names) || !target_names.starts_with(&root_names) {
            continue;
        }
        let depth = dir_names.len() - root_names.len();
        let dst: Vec<Component<'_>> = dst_dir.components().collect();
        let Some(keep) = dst.len().checked_sub(depth) else {
            continue;
        };
        let mut moved: PathBuf = dst[..keep].iter().collect();
        moved.extend(target.components().skip(root_names.len()));
        return moved;
    }
    target.to_path_buf()
}

/// Junctions are recreated as junctions, with targets inside the copied tree rebased onto the copy.
/// Symlinks and volume mount points take the portable path.
pub(crate) fn copy_symlink(
    src_dir: &Dir,
    name: &OsStr,
    m: &Meta,
    dst: &Dir,
    dst_name: &OsStr,
    roots: &[PathBuf],
) -> io::Result<()> {
    let Some(target) = junction_target(&src_dir.path().join(name)) else {
        return super::std_copy::copy_symlink(src_dir, name, m, dst, dst_name, roots);
    };
    let target = rebased_target(roots, src_dir.path(), dst.path(), &target);
    create_junction(&dst.path().join(dst_name), &target)
}

/// Whether the junction at `dst` is the (rebased) copy of the junction at `src`.
pub(crate) fn rewritten_link_matches(roots: &[PathBuf], src: &Path, dst: &Path) -> bool {
    let (Some(from), Some(to), Some(src_dir), Some(dst_dir)) = (
        junction_target(src),
        junction_target(dst),
        src.parent(),
        dst.parent(),
    ) else {
        return false;
    };
    folded(&rebased_target(roots, src_dir, dst_dir, &from)) == folded(&to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junction_buffer_round_trips() {
        let target = Path::new(r"C:\data\tree\sub");
        assert_eq!(
            parse_junction(&junction_buffer(target)).as_deref(),
            Some(target)
        );
    }

    #[test]
    fn volume_mount_points_are_not_junction_targets() {
        let mut buf = junction_buffer(Path::new(r"C:\x"));
        // Turn the drive letter's colon into another character: not a drive path any more.
        let at = REPARSE_HEADER + MOUNT_POINT_HEADER + NT_PREFIX.len() * 2;
        buf[at + 2] = b'_';
        assert_eq!(parse_junction(&buf), None);
    }

    #[test]
    fn targets_inside_the_tree_follow_the_copy() {
        let roots = [PathBuf::from(r"C:\src\tree")];
        let rebased = |dir: &str, dst: &str, target: &str| {
            rebased_target(&roots, Path::new(dir), Path::new(dst), Path::new(target))
        };
        // Junction at tree\a\b, copy at D:\out\tree (2)\a\b: the root is read off the copy.
        assert_eq!(
            rebased(
                r"C:\src\tree\a\b",
                r"D:\out\tree (2)\a\b",
                r"C:\SRC\tree\x\y"
            ),
            Path::new(r"D:\out\tree (2)\x\y")
        );
        // Outside the tree, and from a junction outside the tree: unchanged.
        assert_eq!(
            rebased(r"C:\src\tree\a", r"D:\out\tree\a", r"C:\elsewhere"),
            Path::new(r"C:\elsewhere")
        );
        assert_eq!(
            rebased(r"C:\other", r"D:\out", r"C:\src\tree\x"),
            Path::new(r"C:\src\tree\x")
        );
    }
}
