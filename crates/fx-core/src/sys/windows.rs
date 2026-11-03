//! Windows backend: bulk directory enumeration via `GetFileInformationByHandleEx`.
use crate::listing::push_wtf8_from_utf16;
use crate::{EntryKind, Listing, StorageKind, Strategy};

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{io, mem, ptr};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_LEVEL, ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES,
    ERROR_NOT_SUPPORTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_BOTH_DIR_INFO, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo,
    GetDriveTypeW, GetFileInformationByHandleEx, GetVolumeNameForVolumeMountPointW,
    GetVolumePathNameW, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    DEVICE_SEEK_PENALTY_DESCRIPTOR, IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery,
    STORAGE_PROPERTY_QUERY, StorageDeviceSeekPenaltyProperty,
};

const MAX_PATH: usize = 260;
const DRIVE_REMOTE: u32 = 4;
/// Reparse tags with this bit are name surrogates (symlinks, junctions): what std calls a symlink.
const REPARSE_TAG_NAME_SURROGATE: u32 = 0x2000_0000;
/// 100 ns ticks between 1601-01-01 and 1970-01-01.
const TICKS_TO_UNIX_EPOCH: i64 = 116_444_736_000_000_000;

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by CreateFileW and is closed exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

fn last_error() -> io::Error {
    // SAFETY: plain thread-local error read.
    io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
}

/// NUL-terminated UTF-16. Paths of MAX_PATH or more get the `\\?\` prefix, which switches off
/// Win32 normalization, so the path is made absolute and normalized (`/`, `.`, `..`) first.
fn wide_path(path: &Path) -> Vec<u16> {
    let normalized = std::path::absolute(path);
    let mut w: Vec<u16> = normalized
        .as_deref()
        .unwrap_or(path)
        .as_os_str()
        .encode_wide()
        .collect();
    let verbatim = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    if w.len() >= MAX_PATH && normalized.is_ok() && !w.starts_with(&verbatim) {
        if w.starts_with(&[b'\\' as u16, b'\\' as u16]) {
            // \\server\share -> \\?\UNC\server\share
            w.splice(0..2, r"\\?\UNC\".encode_utf16());
        } else {
            w.splice(0..0, r"\\?\".encode_utf16());
        }
    }
    w.push(0);
    w
}

fn open(path: &[u16], access: u32) -> io::Result<Handle> {
    // SAFETY: `path` is NUL-terminated; other pointers are null as documented.
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        Err(last_error())
    } else {
        Ok(Handle(h))
    }
}

/// FILETIME ticks (since 1601) as `SystemTime`, equal to what std reports.
fn system_time(ticks: i64) -> Option<SystemTime> {
    let since_unix = ticks.checked_sub(TICKS_TO_UNIX_EPOCH)?;
    let d = Duration::new(
        since_unix.unsigned_abs() / 10_000_000,
        (since_unix.unsigned_abs() % 10_000_000) as u32 * 100,
    );
    if since_unix >= 0 {
        UNIX_EPOCH.checked_add(d)
    } else {
        UNIX_EPOCH.checked_sub(d)
    }
}

pub(crate) fn enumerate(
    dir: &Path,
    strategy: &Strategy,
    metadata: bool,
    out: &mut Listing,
) -> io::Result<()> {
    let handle = open(&wide_path(dir), FILE_LIST_DIRECTORY)?;
    // u64 elements: 8-byte alignment, which entries keep via NextEntryOffset.
    let mut buf = vec![0u64; strategy.buf_size.max(4096) / 8];
    let buf_bytes = (buf.len() * 8) as u32;
    let name_off = mem::offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    let mut class = FileIdBothDirectoryRestartInfo;
    let mut first = true;
    let mut name = Vec::new();
    loop {
        // SAFETY: `buf` is valid for `buf_bytes` writable bytes.
        let ok = unsafe {
            GetFileInformationByHandleEx(handle.0, class, buf.as_mut_ptr().cast(), buf_bytes)
        };
        class = FileIdBothDirectoryInfo;
        if ok == 0 {
            // SAFETY: plain thread-local error read.
            let e = unsafe { GetLastError() };
            return match e {
                ERROR_NO_MORE_FILES => Ok(()),
                // Not a directory, or a filesystem without this info class: nothing has been
                // pushed yet, so std can redo (and correctly classify) the whole directory.
                ERROR_INVALID_PARAMETER | ERROR_INVALID_LEVEL | ERROR_NOT_SUPPORTED if first => {
                    super::std_fs::enumerate(dir, strategy, metadata, out)
                }
                e => Err(io::Error::from_raw_os_error(e as i32)),
            };
        }
        first = false;
        let base = buf.as_ptr().cast::<u8>();
        let mut off = 0usize;
        loop {
            // SAFETY: the kernel filled a chain of 8-aligned records inside `buf`;
            // `off` follows NextEntryOffset and the name lies within the record.
            let (info, units) = unsafe {
                let info = &*base.add(off).cast::<FILE_ID_BOTH_DIR_INFO>();
                let n = info.FileNameLength as usize / 2;
                (
                    info,
                    std::slice::from_raw_parts(base.add(off + name_off).cast::<u16>(), n),
                )
            };
            if units != [b'.' as u16] && units != [b'.' as u16, b'.' as u16] {
                name.clear();
                push_wtf8_from_utf16(&mut name, units);
                let attrs = info.FileAttributes;
                // EaSize carries the reparse tag when the reparse-point attribute is set.
                let kind = if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0
                    && info.EaSize & REPARSE_TAG_NAME_SURROGATE != 0
                {
                    EntryKind::Symlink
                } else if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    EntryKind::Dir
                } else {
                    EntryKind::File
                };
                out.push_encoded(
                    &name,
                    kind,
                    attrs & FILE_ATTRIBUTE_HIDDEN != 0,
                    info.FileId as u64,
                );
                if metadata {
                    let e = out.entries.last_mut().expect("just pushed");
                    e.size = u64::try_from(info.EndOfFile).ok();
                    e.modified = system_time(info.LastWriteTime);
                }
            }
            match info.NextEntryOffset {
                0 => break,
                next => off += next as usize,
            }
        }
    }
}

pub(crate) fn detect_storage(path: &Path) -> StorageKind {
    let w = wide_path(path);
    let unc = [b'\\' as u16, b'\\' as u16];
    let is_local_prefix = |s: &[u16]| {
        // \\?\C:\ or \\.\C:\ are local; \\?\UNC\ is not.
        s.len() > 4 && matches!(s[2], 0x3F | 0x2E) && s[3] == b'\\' as u16 && {
            let rest = &s[4..];
            !(rest.len() >= 4 && String::from_utf16_lossy(&rest[..4]).eq_ignore_ascii_case("UNC\\"))
        }
    };
    if w.starts_with(&unc) && !is_local_prefix(&w) {
        return StorageKind::Network;
    }
    let mut root = [0u16; MAX_PATH + 1];
    // SAFETY: `w` is NUL-terminated and `root` has the stated capacity.
    if unsafe { GetVolumePathNameW(w.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return StorageKind::Unknown;
    }
    // SAFETY: GetVolumePathNameW NUL-terminates `root`.
    if unsafe { GetDriveTypeW(root.as_ptr()) } == DRIVE_REMOTE {
        return StorageKind::Network;
    }
    seek_penalty(&root).unwrap_or(StorageKind::Unknown)
}

/// `root` is a NUL-terminated volume mount point such as `C:\`.
fn seek_penalty(root: &[u16]) -> Option<StorageKind> {
    let mut volume = [0u16; 64];
    // SAFETY: `root` is NUL-terminated; `volume` has the stated capacity.
    if unsafe {
        GetVolumeNameForVolumeMountPointW(root.as_ptr(), volume.as_mut_ptr(), volume.len() as u32)
    } == 0
    {
        return None;
    }
    // `\\?\Volume{guid}\` -> `\\?\Volume{guid}` (a trailing slash would open the root dir).
    let len = volume.iter().position(|&c| c == 0)?;
    volume[len - 1] = 0;
    let handle = open(&volume[..len], 0).ok()?;
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceSeekPenaltyProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    // SAFETY: all-zero is a valid descriptor.
    let mut desc: DEVICE_SEEK_PENALTY_DESCRIPTOR = unsafe { mem::zeroed() };
    let mut returned = 0u32;
    // SAFETY: in/out buffers are valid for the sizes passed.
    let ok = unsafe {
        DeviceIoControl(
            handle.0,
            IOCTL_STORAGE_QUERY_PROPERTY,
            ptr::from_ref(&query).cast(),
            mem::size_of_val(&query) as u32,
            ptr::from_mut(&mut desc).cast(),
            mem::size_of_val(&desc) as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        None
    } else if desc.IncursSeekPenalty {
        Some(StorageKind::Hdd)
    } else {
        Some(StorageKind::Ssd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_matches_std() {
        // 2001-09-09T01:46:40Z = 1_000_000_000 s after the Unix epoch.
        let t = system_time(TICKS_TO_UNIX_EPOCH + 10_000_000_000_000_000).unwrap();
        assert_eq!(t, UNIX_EPOCH + Duration::from_secs(1_000_000_000));
        assert_eq!(
            system_time(TICKS_TO_UNIX_EPOCH - 10_000_000).unwrap(),
            UNIX_EPOCH - Duration::from_secs(1)
        );
    }

    #[test]
    fn long_paths_get_verbatim_prefix() {
        let long = format!(r"C:\{}", "a".repeat(300));
        let w = wide_path(Path::new(&long));
        assert!(w.starts_with(&r"\\?\C:\".encode_utf16().collect::<Vec<_>>()));
        let unc = format!(r"\\srv\share\{}", "a".repeat(300));
        let w = wide_path(Path::new(&unc));
        assert!(w.starts_with(&r"\\?\UNC\srv".encode_utf16().collect::<Vec<_>>()));
    }

    #[test]
    fn unc_is_network() {
        assert_eq!(
            detect_storage(Path::new(r"\\server\share\dir")),
            StorageKind::Network
        );
    }
}
