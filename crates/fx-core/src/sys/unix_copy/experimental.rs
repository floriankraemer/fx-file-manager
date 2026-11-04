//! Opt-in Linux copy experiments of issue #22. None is planned by default; `fx cp --experimental NAME`
//! tries one first and falls back to the planned chain when it does not apply or is unsupported.

use super::{Src, linux, write_at};
use crate::copy::pipeline::{FileCtx, Meter};
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use rustix::io::Errno;
use std::io;
use std::os::fd::BorrowedFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

/// The io_uring path handles files up to this size; larger ones are not latency bound.
pub(super) const URING_MAX_FILE: u64 = 1 << 20;
/// Range size and worker count of the parallel ranged copy.
pub(super) const RANGE: u64 = 64 << 20;
pub(super) const RANGE_WORKERS: usize = 4;
/// `O_DIRECT` block: a multiple of any logical block size.
const DIRECT_BLOCK: usize = 8 << 20;
const DIRECT_ALIGN: usize = 4096;

/// Linked `READ` -> `WRITE` in one submission: one syscall per small file instead of two.
/// ponytail: no linked OPENAT/CLOSE and no registered buffers (the open stays in `place` for conflict
/// handling); add them only if #21 shows the two-op version is worth extending.
#[cfg(feature = "uring")]
pub(super) fn uring_copy(
    ctx: &FileCtx<'_>,
    src: &Src,
    out: BorrowedFd<'_>,
    meter: &mut Meter<'_>,
) -> io::Result<()> {
    use io_uring::{IoUring, opcode, squeue::Flags, types::Fd};
    use std::cell::RefCell;
    use std::os::fd::AsRawFd;

    thread_local! {
        /// One ring per worker; `None` until first use.
        static RING: RefCell<Option<IoUring>> = const { RefCell::new(None) };
    }
    let size = src.meta.size as usize;
    if size == 0 {
        return Ok(());
    }
    ctx.shared.checkpoint()?;
    RING.with_borrow_mut(|slot| {
        if slot.is_none() {
            // EPERM (seccomp, sysctl), ENOSYS (old kernel): "unsupported", so the chain moves on.
            *slot = Some(IoUring::new(4).map_err(|e| {
                io::Error::new(io::ErrorKind::Unsupported, format!("io_uring: {e}"))
            })?);
        }
        let ring = slot.as_mut().expect("ring created above");
        let mut broken = false;
        let result = super::BUF.with_borrow_mut(|buf| {
            if buf.len() < size {
                buf.resize(size, 0);
            }
            let read = opcode::Read::new(Fd(src.fd.as_raw_fd()), buf.as_mut_ptr(), size as u32)
                .build()
                .flags(Flags::IO_LINK)
                .user_data(1);
            let write = opcode::Write::new(Fd(out.as_raw_fd()), buf.as_ptr(), size as u32)
                .build()
                .user_data(2);
            // SAFETY: `buf` and both descriptors stay alive until both completions are reaped below;
            // on a submit error the ring is dropped, which waits for in-flight requests.
            let pushed = unsafe { ring.submission().push_multiple(&[read, write]) };
            let submitted = pushed
                .map_err(|_| io::Error::other("io_uring submission queue full"))
                .and_then(|()| ring.submit_and_wait(2).map(drop));
            if let Err(e) = submitted {
                broken = true;
                return Err(e);
            }
            let mut result = [0i32; 2];
            for cqe in ring.completion() {
                result[(cqe.user_data() - 1) as usize] = cqe.result();
            }
            let [read, write] = result;
            if read < 0 {
                return Err(io::Error::from_raw_os_error(-read));
            }
            if read as usize != size {
                return Err(io::Error::other(
                    "the file changed while it was being copied",
                ));
            }
            let written = match write {
                w if w < 0 => return Err(io::Error::from_raw_os_error(-w)),
                w => w as usize,
            };
            // A short write is finished synchronously.
            write_at(out, &buf[written..size], written as u64)?;
            meter.add(size as u64);
            Ok(())
        });
        if broken {
            *slot = None;
        }
        result
    })
}

/// Copies `size` bytes in `range`-sized pieces by up to `workers` threads, each with positional
/// `copy_file_range`. Every worker meters on its own; their counts are merged so a failure rolls all back.
pub(super) fn parallel_copy(
    ctx: &FileCtx<'_>,
    src: &Src,
    out: BorrowedFd<'_>,
    size: u64,
    meter: &mut Meter<'_>,
    range: u64,
    workers: usize,
) -> io::Result<()> {
    let ranges: Vec<(u64, u64)> = (0..size)
        .step_by(range as usize)
        .map(|start| (start, (start + range).min(size)))
        .collect();
    let next = AtomicUsize::new(0);
    let done: Vec<(Meter<'_>, io::Result<()>)> = thread::scope(|s| {
        let handles: Vec<_> = (0..workers.min(ranges.len()))
            .map(|_| {
                s.spawn(|| {
                    let mut m = ctx.meter();
                    loop {
                        let Some(&r) = ranges.get(next.fetch_add(1, Ordering::Relaxed)) else {
                            return (m, Ok(()));
                        };
                        let kind = linux::Kind::CopyFileRange;
                        if let Err(e) = linux::copy_ranges(ctx, src, out, &[r], &mut m, kind, false)
                        {
                            // Stops the other workers at their next range.
                            next.store(ranges.len(), Ordering::Relaxed);
                            return (m, Err(e));
                        }
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("range worker panicked"))
            .collect()
    });
    let mut first_error = None;
    for (m, result) in done {
        meter.adopt(m);
        if let Err(e) = result {
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// Aligned read/write with `O_DIRECT` on both files: no page cache, so no write-back stall.
pub(super) fn direct_copy(
    ctx: &FileCtx<'_>,
    src: &Src,
    out: BorrowedFd<'_>,
    meter: &mut Meter<'_>,
) -> io::Result<()> {
    let size = src.meta.size;
    let src_flags = fcntl_getfl(&src.fd)?;
    let out_flags = fcntl_getfl(out)?;
    fcntl_setfl(&src.fd, src_flags | OFlags::DIRECT)?;
    let result = fcntl_setfl(out, out_flags | OFlags::DIRECT)
        .map_err(io::Error::from)
        .and_then(|()| {
            let mut storage = vec![0u8; DIRECT_BLOCK + DIRECT_ALIGN];
            let pad = storage.as_ptr().align_offset(DIRECT_ALIGN);
            let buf = &mut storage[pad..pad + DIRECT_BLOCK];
            let mut off = 0u64;
            while off < size {
                ctx.shared.checkpoint()?;
                let n = match rustix::io::pread(&src.fd, &mut *buf, off) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(Errno::INTR) => continue,
                    Err(e) => return Err(e.into()),
                };
                if n % DIRECT_ALIGN != 0 {
                    // Only the last block can be ragged: it needs a buffered write.
                    fcntl_setfl(out, out_flags)?;
                }
                write_at(out, &buf[..n], off)?;
                meter.add(n as u64);
                off += n as u64;
            }
            Ok(())
        });
    let _ = fcntl_setfl(&src.fd, src_flags);
    let _ = fcntl_setfl(out, out_flags);
    result
}
