//! Error classes that pick the retry, fallback or pause path, and the step an error belongs to.

use crate::copy::progress::{ErrorOp, is_cancelled};
use std::io;

/// What a failed copy method means for the rest of the job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fallback {
    /// The volume pair cannot do this at all: the method is disabled and the chain moves on.
    Pair,
    /// This one file is odd (a size or range the method rejects): the next method takes it and the
    /// method stays enabled for the others.
    File,
}

pub(crate) fn fallback_of(e: &io::Error) -> Option<Fallback> {
    if matches!(
        e.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::CrossesDevices
    ) || is_pair_errno(e)
    {
        return Some(Fallback::Pair);
    }
    (e.kind() == io::ErrorKind::InvalidInput || is_file_errno(e)).then_some(Fallback::File)
}

#[cfg(unix)]
fn is_pair_errno(e: &io::Error) -> bool {
    use rustix::io::Errno;
    const PAIR: [Errno; 5] = [
        Errno::NOSYS,
        Errno::OPNOTSUPP,
        Errno::NOTSUP,
        Errno::NOTTY,
        Errno::XDEV,
    ];
    e.raw_os_error()
        .is_some_and(|code| PAIR.contains(&Errno::from_raw_os_error(code)))
}

#[cfg(unix)]
fn is_file_errno(e: &io::Error) -> bool {
    use rustix::io::Errno;
    e.raw_os_error().is_some_and(|code| {
        matches!(
            Errno::from_raw_os_error(code),
            Errno::INVAL | Errno::OVERFLOW
        )
    })
}

/// ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED, ERROR_CALL_NOT_IMPLEMENTED.
#[cfg(not(unix))]
fn is_pair_errno(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(1 | 50 | 120))
}

#[cfg(not(unix))]
fn is_file_errno(_e: &io::Error) -> bool {
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
        let pair = |e: io::Error| fallback_of(&e) == Some(Fallback::Pair);
        assert!(pair(io::ErrorKind::Unsupported.into()));
        assert!(pair(io::ErrorKind::CrossesDevices.into()));
        assert_eq!(fallback_of(&io::ErrorKind::PermissionDenied.into()), None);
        assert_eq!(
            fallback_of(&io::ErrorKind::InvalidInput.into()),
            Some(Fallback::File)
        );
        #[cfg(target_os = "linux")]
        for errno in [
            rustix::io::Errno::XDEV,
            rustix::io::Errno::OPNOTSUPP,
            rustix::io::Errno::NOSYS,
            rustix::io::Errno::NOTTY,
        ] {
            assert!(pair(errno.into()), "{errno:?}");
        }
        #[cfg(target_os = "linux")]
        for errno in [rustix::io::Errno::INVAL, rustix::io::Errno::OVERFLOW] {
            assert_eq!(
                fallback_of(&errno.into()),
                Some(Fallback::File),
                "{errno:?}"
            );
        }
    }
}
