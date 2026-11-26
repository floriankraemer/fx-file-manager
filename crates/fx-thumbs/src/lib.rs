//! Thumbnails for the Icons view, taken from the operating system's thumbnail cache where there is one.

use std::path::Path;
use std::time::SystemTime;
use std::{fmt, io};

mod decode;
#[cfg(target_os = "linux")]
mod freedesktop;

/// The edge lengths thumbnails are produced at: the freedesktop `normal`, `large` and `x-large` sizes,
/// so a cache another application filled is hit without rescaling.
pub const BUCKETS: [u32; 3] = [128, 256, 512];

/// The smallest bucket at least `px` wide, or the largest one.
pub fn bucket(px: u32) -> u32 {
    BUCKETS
        .into_iter()
        .find(|b| *b >= px)
        .unwrap_or(BUCKETS[BUCKETS.len() - 1])
}

/// RGBA8 pixels, straight alpha, fitting into a `bucket` square with the source's aspect ratio.
pub struct Thumb {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// What is known about the file from its listing; a cached thumbnail is valid only for the same values.
pub struct Request<'a> {
    pub path: &'a Path,
    pub modified: Option<SystemTime>,
    pub size: u64,
    /// The wanted edge length in physical pixels, snapped with [`bucket`].
    pub px: u32,
}

#[derive(Debug)]
pub enum ThumbError {
    /// No thumbnailer handles this kind of file.
    Unsupported,
    /// The file or its pixel count exceeds the decode limits.
    TooLarge,
    Io(io::Error),
    Decode(String),
}

impl fmt::Display for ThumbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThumbError::Unsupported => f.write_str("no thumbnailer for this file type"),
            ThumbError::TooLarge => f.write_str("file exceeds the thumbnail decode limits"),
            ThumbError::Io(e) => write!(f, "{e}"),
            ThumbError::Decode(e) => write!(f, "cannot decode: {e}"),
        }
    }
}

impl std::error::Error for ThumbError {}

impl From<io::Error> for ThumbError {
    fn from(e: io::Error) -> Self {
        ThumbError::Io(e)
    }
}

/// Produces thumbnails on the calling thread. Holds per-thread OS state, so each worker creates its own.
pub struct Thumbnailer {
    #[cfg(target_os = "linux")]
    cache: Option<freedesktop::Cache>,
}

impl Default for Thumbnailer {
    fn default() -> Self {
        Self::new()
    }
}

impl Thumbnailer {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "linux")]
            cache: freedesktop::Cache::user(),
        }
    }

    pub fn thumbnail(&mut self, request: &Request) -> Result<Thumb, ThumbError> {
        let px = bucket(request.px);
        #[cfg(target_os = "linux")]
        if let Some(cache) = &self.cache {
            return cache.thumbnail(request, px);
        }
        decode::thumbnail(request.path, px)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_snaps_up_and_caps_at_the_largest() {
        assert_eq!(bucket(1), 128);
        assert_eq!(bucket(128), 128);
        assert_eq!(bucket(129), 256);
        assert_eq!(bucket(400), 512);
        assert_eq!(bucket(4000), 512);
    }
}
