//! Error classes that pick the retry, fallback or pause path, and the step an error belongs to.

use crate::copy::progress::{ErrorOp, is_cancelled};
use std::io;

/// `ENOTSUP`/`EXDEV`/`EINVAL`-style failures mean "try the next method" and disable this one for the pair.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn is_unsupported(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::CrossesDevices | io::ErrorKind::InvalidInput
    ) || is_unsupported_errno(e)
}

#[cfg(unix)]
fn is_unsupported_errno(e: &io::Error) -> bool {
    use rustix::io::Errno;
    const UNSUPPORTED: [Errno; 7] = [
        Errno::NOSYS,
        Errno::OPNOTSUPP,
        Errno::NOTSUP,
        Errno::OVERFLOW,
        Errno::NOTTY,
        Errno::XDEV,
        Errno::INVAL,
    ];
    e.raw_os_error()
        .is_some_and(|code| UNSUPPORTED.contains(&Errno::from_raw_os_error(code)))
}

#[cfg(not(unix))]
fn is_unsupported_errno(_e: &io::Error) -> bool {
    false
}

pub(super) fn is_transient(e: &io::Error) -> bool {
    use io::ErrorKind::*;
    // ERROR_UNEXP_NET_ERR, ERROR_NETNAME_DELETED: an SMB session dropped mid-file.
    let windows_net = cfg!(windows) && matches!(e.raw_os_error(), Some(59 | 64));
    // EMFILE/ENFILE: other workers close their descriptors within one file.
    let out_of_fds = cfg!(unix) && matches!(e.raw_os_error(), Some(23 | 24));
    let transient_kind = matches!(
        e.kind(),
        TimedOut
            | WouldBlock
            | ConnectionReset
            | ConnectionAborted
            | NetworkDown
            | NetworkUnreachable
            | HostUnreachable
            | Interrupted
    );
    windows_net || out_of_fds || transient_kind && !is_cancelled(e)
}

pub(super) fn is_disk_full(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded
    )
}

pub(super) fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

/// An error and the step it belongs to.
pub(super) struct Failure {
    pub(super) op: ErrorOp,
    pub(super) err: io::Error,
}

pub(super) trait At<T> {
    fn at(self, op: ErrorOp) -> Result<T, Failure>;
}

impl<T> At<T> for io::Result<T> {
    fn at(self, op: ErrorOp) -> Result<T, Failure> {
        self.map_err(|err| Failure { op, err })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::copy::progress::cancelled;

    #[test]
    fn error_classes() {
        assert!(is_transient(&io::ErrorKind::TimedOut.into()));
        assert!(is_transient(&io::ErrorKind::ConnectionReset.into()));
        assert!(!is_transient(&cancelled()));
        assert!(!is_transient(&io::ErrorKind::PermissionDenied.into()));
        assert!(is_disk_full(&io::ErrorKind::StorageFull.into()));
        assert!(is_disk_full(&io::ErrorKind::QuotaExceeded.into()));
        assert!(is_unsupported(&io::ErrorKind::Unsupported.into()));
        assert!(is_unsupported(&io::ErrorKind::CrossesDevices.into()));
        assert!(!is_unsupported(&io::ErrorKind::PermissionDenied.into()));
        #[cfg(target_os = "linux")]
        for errno in [
            rustix::io::Errno::XDEV,
            rustix::io::Errno::OPNOTSUPP,
            rustix::io::Errno::NOSYS,
            rustix::io::Errno::INVAL,
        ] {
            assert!(is_unsupported(&errno.into()), "{errno:?}");
        }
    }
}
