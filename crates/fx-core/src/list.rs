//! Use case: detect storage, enumerate, enrich, sort.

use crate::listing::Listing;
use crate::storage::{StorageKind, Strategy};
use crate::{sort, sys};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::{fmt, io, thread};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// The OS-specific bulk API (compile-time chosen).
    Native,
    /// Portable `std::fs`; baseline and test oracle.
    Std,
}

#[derive(Clone, Copy, Debug)]
pub struct ListOptions {
    pub metadata: bool,
    pub sort: bool,
    pub backend: Backend,
    /// Overrides storage detection.
    pub storage: Option<StorageKind>,
}

impl Default for ListOptions {
    fn default() -> Self {
        Self {
            metadata: true,
            sort: true,
            backend: Backend::Native,
            storage: None,
        }
    }
}

#[derive(Debug)]
pub struct Listed {
    pub listing: Listing,
    pub storage: StorageKind,
    pub strategy: Strategy,
}

#[derive(Debug)]
pub enum ListError {
    NotFound(PathBuf),
    PermissionDenied(PathBuf),
    NotADirectory(PathBuf),
    Io(PathBuf, io::Error),
}

impl ListError {
    pub(crate) fn new(path: &Path, e: io::Error) -> Self {
        let path = path.to_path_buf();
        match e.kind() {
            io::ErrorKind::NotFound => Self::NotFound(path),
            io::ErrorKind::PermissionDenied => Self::PermissionDenied(path),
            io::ErrorKind::NotADirectory => Self::NotADirectory(path),
            _ => Self::Io(path, e),
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::NotFound(p)
            | Self::PermissionDenied(p)
            | Self::NotADirectory(p)
            | Self::Io(p, _) => p,
        }
    }
}

/// Formats as `<path>: <reason>`.
impl fmt::Display for ListError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path().display();
        match self {
            Self::NotFound(_) => write!(f, "{path}: no such file or directory"),
            Self::PermissionDenied(_) => write!(f, "{path}: permission denied"),
            Self::NotADirectory(_) => write!(f, "{path}: not a directory"),
            Self::Io(_, e) => write!(f, "{path}: {e}"),
        }
    }
}

impl Error for ListError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(_, e) => Some(e),
            _ => None,
        }
    }
}

pub fn list_dir(path: &Path, opts: &ListOptions) -> Result<Listed, ListError> {
    let storage = opts
        .storage
        .unwrap_or_else(|| sys::native::detect_storage(path));
    let cores = thread::available_parallelism().map_or(1, |n| n.get());
    let strategy = Strategy::for_kind(storage, cores);

    let mut listing = Listing::default();
    let enumerate = match opts.backend {
        Backend::Native => sys::native::enumerate,
        Backend::Std => sys::std_fs::enumerate,
    };
    enumerate(path, &strategy, opts.metadata, &mut listing).map_err(|e| ListError::new(path, e))?;

    if opts.sort {
        sort::sort(&mut listing);
    }
    Ok(Listed {
        listing,
        storage,
        strategy,
    })
}
