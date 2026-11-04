//! macOS backend: `getattrlistbulk(2)` returns name, type, mtime, flags, inode and file size
//! for a whole directory block in one syscall.

use crate::{EntryKind, Listing, StorageKind, Strategy};
use libc::{c_int, c_void};
use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// <sys/attr.h>; not exported by libc.
const ATTR_CMN_ERROR: u32 = 0x2000_0000;
// <sys/vnode.h> enum vtype.
const VREG: u32 = 1;
const VDIR: u32 = 2;
const VLNK: u32 = 5;

/// Closes the directory fd on every exit path.
struct Fd(c_int);

impl Drop for Fd {
    fn drop(&mut self) {
        // SAFETY: we own the fd and close it exactly once.
        unsafe { libc::close(self.0) };
    }
}

fn cstring(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))
}

pub(crate) fn enumerate(
    dir: &Path,
    strategy: &Strategy,
    metadata: bool,
    out: &mut Listing,
) -> io::Result<()> {
    let cdir = cstring(dir)?;
    // SAFETY: `cdir` is a valid C string.
    let raw = unsafe {
        libc::open(
            cdir.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = Fd(raw);

    let mut list = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS
            | libc::ATTR_CMN_NAME
            | ATTR_CMN_ERROR
            | libc::ATTR_CMN_OBJTYPE
            | libc::ATTR_CMN_MODTIME
            | libc::ATTR_CMN_FILEID
            | libc::ATTR_CMN_FLAGS,
        volattr: 0,
        dirattr: 0,
        fileattr: libc::ATTR_FILE_DATALENGTH,
        forkattr: 0,
    };
    // u64 elements give the 8-byte alignment the kernel expects; records themselves are
    // only 4-byte aligned, so they are parsed from byte slices, never by pointer cast.
    let mut buf = vec![0u64; strategy.buf_size.max(4096).div_ceil(8)];
    let buf_bytes = buf.len() * 8;
    let mut first = true;

    loop {
        // SAFETY: `list` and `buf` are valid for the sizes passed.
        let n = unsafe {
            libc::getattrlistbulk(
                fd.0,
                std::ptr::from_mut(&mut list).cast::<c_void>(),
                buf.as_mut_ptr().cast::<c_void>(),
                buf_bytes,
                u64::from(libc::FSOPT_PACK_INVAL_ATTRS),
            )
        };
        match n {
            0 => return Ok(()),
            n if n < 0 => {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                // Filesystem without getattrlistbulk: nothing pushed yet, let std do it all.
                if first && matches!(err.raw_os_error(), Some(libc::ENOTSUP | libc::EINVAL)) {
                    return super::std_fs::enumerate(dir, strategy, metadata, out);
                }
                return Err(err);
            }
            n => {
                first = false;
                // SAFETY: the kernel initialised the first `buf_bytes` bytes at most; we
                // view the same allocation as bytes and only read within record lengths.
                let bytes =
                    unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), buf_bytes) };
                let mut off = 0;
                for _ in 0..n {
                    let Some(len) = read_u32(bytes, off).map(|l| l as usize) else {
                        return Err(corrupt());
                    };
                    let rec = off
                        .checked_add(len)
                        .and_then(|end| bytes.get(off..end))
                        .ok_or_else(corrupt)?;
                    if let Some(p) = parse_record(rec) {
                        push(&fd, &p, metadata, out);
                    }
                    off += len;
                }
            }
        }
    }
}

fn corrupt() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "malformed getattrlistbulk record",
    )
}

/// One decoded record. `name` includes its trailing NUL; `None` fields were not valid.
struct Parsed<'a> {
    name: &'a [u8],
    kind: EntryKind,
    modified: Option<SystemTime>,
    flags: u32,
    inode: u64,
    data_len: Option<u64>,
}

fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn read_i64(b: &[u8], at: usize) -> Option<i64> {
    Some(i64::from_ne_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Layout (getattrlistbulk(2); FSOPT_PACK_INVAL_ATTRS makes it fixed for the requested set,
/// the returned set says which values are real). Byte offsets from the record start:
///   0 u32 length | 4 attribute_set_t (5 x u32) | 24 u32 ERROR (right after RETURNED_ATTRS)
///   28 attrreference_t NAME (offset is relative to its own position) | 36 u32 OBJTYPE
///   40 timespec MODTIME (i64 sec, i64 nsec) | 56 u32 FLAGS | 60 u64 FILEID | 68 off_t DATALENGTH
fn parse_record(rec: &[u8]) -> Option<Parsed<'_>> {
    const RETURNED_COMMON: usize = 4;
    const RETURNED_FILE: usize = 16;
    const NAME_REF: usize = 28;
    const OBJTYPE: usize = 36;
    const MODTIME: usize = 40;
    const FLAGS: usize = 56;
    const FILEID: usize = 60;
    const DATALENGTH: usize = 68;

    let common = read_u32(rec, RETURNED_COMMON)?;
    let file = read_u32(rec, RETURNED_FILE)?;
    let has = |bit: u32| common & bit != 0;

    if !has(libc::ATTR_CMN_NAME) {
        return None;
    }
    let name_start = NAME_REF.checked_add_signed(read_u32(rec, NAME_REF)? as i32 as isize)?;
    let name_len = read_u32(rec, NAME_REF + 4)? as usize;
    let name = rec.get(name_start..name_start.checked_add(name_len)?)?;
    let name = if name.last() == Some(&0) {
        name
    } else {
        return None;
    };

    let kind = if has(libc::ATTR_CMN_OBJTYPE) {
        match read_u32(rec, OBJTYPE)? {
            VREG => EntryKind::File,
            VDIR => EntryKind::Dir,
            VLNK => EntryKind::Symlink,
            _ => EntryKind::Other,
        }
    } else {
        EntryKind::Other
    };
    let modified = if has(libc::ATTR_CMN_MODTIME) {
        let (sec, nsec) = (read_i64(rec, MODTIME)?, read_i64(rec, MODTIME + 8)?);
        let nsec = Duration::from_nanos(u64::try_from(nsec).ok()?);
        Some(if sec >= 0 {
            UNIX_EPOCH + Duration::from_secs(sec.unsigned_abs()) + nsec
        } else {
            UNIX_EPOCH - Duration::from_secs(sec.unsigned_abs()) + nsec
        })
    } else {
        None
    };
    Some(Parsed {
        name,
        kind,
        modified,
        flags: if has(libc::ATTR_CMN_FLAGS) {
            read_u32(rec, FLAGS)?
        } else {
            0
        },
        inode: if has(libc::ATTR_CMN_FILEID) {
            u64::try_from(read_i64(rec, FILEID)?).unwrap_or(0)
        } else {
            0
        },
        data_len: if file & libc::ATTR_FILE_DATALENGTH != 0 {
            u64::try_from(read_i64(rec, DATALENGTH)?).ok()
        } else {
            None
        },
    })
}

fn push(dir: &Fd, p: &Parsed<'_>, metadata: bool, out: &mut Listing) {
    let bytes = &p.name[..p.name.len() - 1];
    let hidden = bytes.first() == Some(&b'.') || p.flags & libc::UF_HIDDEN != 0;
    out.push_encoded(bytes, p.kind, hidden, p.inode);
    if !metadata {
        return;
    }
    let e = out.entries.last_mut().expect("just pushed");
    e.modified = p.modified;
    // The bulk call has DATALENGTH only for regular files. std reports lstat's st_size for
    // dirs and symlinks (non-zero), so those cost one fstatat each to stay identical.
    e.size = match (p.kind, p.data_len) {
        (EntryKind::File, Some(len)) => Some(len),
        _ => lstat_size(dir, p.name),
    };
}

/// `name` must be NUL-terminated.
fn lstat_size(dir: &Fd, name: &[u8]) -> Option<u64> {
    // SAFETY: `name` ends in NUL; `st` is a valid out-pointer, read only on success.
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        let rc = libc::fstatat(
            dir.0,
            name.as_ptr().cast(),
            &mut st,
            libc::AT_SYMLINK_NOFOLLOW,
        );
        (rc == 0).then(|| u64::try_from(st.st_size).unwrap_or(0))
    }
}

const NETWORK_FS: [&[u8]; 6] = [
    b"smbfs", b"nfs", b"afpfs", b"webdav", b"macfuse", b"osxfuse",
];

fn fs_type_name(path: &Path) -> Option<std::ffi::CString> {
    let cpath = cstring(path).ok()?;
    // SAFETY: valid C string and out-pointer; the struct is read only on success.
    unsafe {
        let mut sfs: libc::statfs = std::mem::zeroed();
        if libc::statfs(cpath.as_ptr(), &mut sfs) != 0 {
            return None;
        }
        Some(std::ffi::CStr::from_ptr(sfs.f_fstypename.as_ptr()).to_owned())
    }
}

fn storage_of(fstype: &std::ffi::CStr) -> StorageKind {
    if NETWORK_FS.contains(&fstype.to_bytes()) {
        StorageKind::Network
    } else {
        // ponytail: macs ship SSD; IOKit rotational probe if external HDD speed matters.
        StorageKind::Ssd
    }
}

pub(crate) fn detect_storage(path: &Path) -> StorageKind {
    fs_type_name(path).map_or(StorageKind::Unknown, |t| storage_of(&t))
}

/// Clone support comes from the filesystem type: every APFS volume supports `clonefile`.
// ponytail: no getattrlist(VOL_CAP_INT_CLONE) probe; the per-pair method cache already
// falls back on the first ENOTSUP, so a probe would only save one failed call per job.
// Removable media stay `Ssd` until a DiskArbitration probe is justified.
pub(crate) fn volume_info(path: &Path) -> crate::VolumeInfo {
    let id = std::fs::symlink_metadata(path).map_or(0, |m| {
        use std::os::unix::fs::MetadataExt;
        m.dev()
    });
    let Some(fstype) = fs_type_name(path) else {
        return crate::VolumeInfo::unknown(id);
    };
    let fs = crate::FsKind::from_name(&fstype.to_string_lossy());
    crate::VolumeInfo {
        id,
        fs,
        storage: storage_of(&fstype),
        caps: crate::FsCaps::for_fs(fs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synthetic_record() {
        let mut r = vec![0u8; 76];
        let put = |r: &mut Vec<u8>, at: usize, v: &[u8]| r[at..at + v.len()].copy_from_slice(v);
        let common = libc::ATTR_CMN_NAME
            | libc::ATTR_CMN_OBJTYPE
            | libc::ATTR_CMN_MODTIME
            | libc::ATTR_CMN_FILEID
            | libc::ATTR_CMN_FLAGS;
        put(&mut r, 4, &common.to_ne_bytes());
        put(&mut r, 16, &libc::ATTR_FILE_DATALENGTH.to_ne_bytes());
        put(&mut r, 28, &48i32.to_ne_bytes()); // 28 + 48 = 76
        put(&mut r, 32, &3u32.to_ne_bytes());
        put(&mut r, 36, &VREG.to_ne_bytes());
        put(&mut r, 40, &7i64.to_ne_bytes());
        put(&mut r, 48, &500i64.to_ne_bytes());
        put(&mut r, 56, &libc::UF_HIDDEN.to_ne_bytes());
        put(&mut r, 60, &42i64.to_ne_bytes());
        put(&mut r, 68, &1234i64.to_ne_bytes());
        r.extend_from_slice(b"ab\0");
        let len = r.len() as u32;
        put(&mut r, 0, &len.to_ne_bytes());

        let p = parse_record(&r).unwrap();
        assert_eq!(p.name, b"ab\0");
        assert_eq!(p.kind, EntryKind::File);
        assert_eq!(p.modified, Some(UNIX_EPOCH + Duration::new(7, 500)));
        assert_eq!(
            (p.flags, p.inode, p.data_len),
            (libc::UF_HIDDEN, 42, Some(1234))
        );
    }
}
