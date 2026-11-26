//! The freedesktop.org thumbnail cache (`~/.cache/thumbnails`), shared with GNOME, KDE and other file managers.
//!
//! A thumbnail is `<size dir>/<md5 of the file URI>.png` and is valid while its `Thumb::MTime` (and
//! `Thumb::Size`, when written) match the file. Misses for images are decoded here and written back.

use crate::{Request, Thumb, ThumbError, decode};
use md5::{Digest, Md5};
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, BufReader, BufWriter};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const MTIME: &str = "Thumb::MTime";
const SIZE: &str = "Thumb::Size";
const URI: &str = "Thumb::URI";

pub(crate) struct Cache {
    root: PathBuf,
}

/// The validity of a cached thumbnail, read from its text chunks.
#[derive(Default)]
struct Stamp {
    mtime: Option<u64>,
    size: Option<u64>,
}

impl Cache {
    /// `$XDG_CACHE_HOME/thumbnails`, falling back to `~/.cache/thumbnails`.
    pub(crate) fn user() -> Option<Self> {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
        Some(Self::at(base.join("thumbnails")))
    }

    pub(crate) fn at(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn thumbnail(&self, request: &Request, px: u32) -> Result<Thumb, ThumbError> {
        let mtime = request
            .modified
            .and_then(|m| m.duration_since(UNIX_EPOCH).ok());
        // The spec forbids thumbnails of the cache itself, and without an mtime no entry can be validated.
        let (Some(mtime), false) = (mtime, request.path.starts_with(&self.root)) else {
            return decode::thumbnail(request.path, px);
        };
        let stamp = Stamp {
            mtime: Some(mtime.as_secs()),
            size: Some(request.size),
        };
        let uri = file_uri(request.path);
        let name = format!("{}.png", hex_md5(&uri));
        if let Some(thumb) = self.lookup(&name, &stamp, px) {
            return Ok(thumb);
        }
        let thumb = decode::thumbnail(request.path, px)?;
        let path = self.root.join(dir_name(px)).join(&name);
        if let Err(e) = write(&path, &uri, &stamp, &thumb) {
            log::debug!("cannot write thumbnail {}: {e}", path.display());
        }
        Ok(thumb)
    }

    /// The first valid thumbnail at `px` or a larger size, fitted to `px`.
    fn lookup(&self, name: &str, stamp: &Stamp, px: u32) -> Option<Thumb> {
        crate::BUCKETS
            .into_iter()
            .filter(|b| *b >= px)
            .find_map(|b| read(&self.root.join(dir_name(b)).join(name), stamp))
            .and_then(|thumb| refit(thumb, px))
    }
}

fn refit(thumb: Thumb, px: u32) -> Option<Thumb> {
    if thumb.width <= px && thumb.height <= px {
        return Some(thumb);
    }
    let image = image::RgbaImage::from_raw(thumb.width, thumb.height, thumb.rgba)?;
    decode::fit(image::DynamicImage::ImageRgba8(image), px).ok()
}

fn dir_name(px: u32) -> &'static str {
    match px {
        0..=128 => "normal",
        129..=256 => "large",
        _ => "x-large",
    }
}

/// `file://` URI of `path`, escaped as GLib's `g_filename_to_uri` does, so the hash matches other applications'.
fn file_uri(path: &Path) -> String {
    const KEEP: &[u8] = b"-._~!$&'()*+,;=:@/";
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || KEEP.contains(&b) {
            uri.push(char::from(b));
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

fn hex_md5(text: &str) -> String {
    Md5::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The thumbnail at `path` if it exists, decodes and matches `stamp`.
fn read(path: &Path, stamp: &Stamp) -> Option<Thumb> {
    let file = File::open(path).ok()?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    if !stamp_of(reader.info()).matches(stamp) {
        return None;
    }
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut pixels).ok()?;
    pixels.truncate(frame.buffer_size());
    let rgba = to_rgba(frame.color_type, pixels)?;
    Some(Thumb {
        width: frame.width,
        height: frame.height,
        rgba,
    })
}

fn stamp_of(info: &png::Info) -> Stamp {
    let mut stamp = Stamp::default();
    let latin1 = info
        .uncompressed_latin1_text
        .iter()
        .map(|c| (c.keyword.as_str(), c.text.clone()));
    let utf8 = info
        .utf8_text
        .iter()
        .filter_map(|c| Some((c.keyword.as_str(), c.get_text().ok()?)));
    for (keyword, text) in latin1.chain(utf8) {
        // Some writers store fractional seconds; the spec's value is the integer part.
        let number = text.split('.').next().and_then(|n| n.trim().parse().ok());
        match keyword {
            MTIME => stamp.mtime = number,
            SIZE => stamp.size = number,
            _ => {}
        }
    }
    stamp
}

impl Stamp {
    /// The cached entry needs a matching mtime; its size is compared only when it recorded one.
    fn matches(&self, file: &Stamp) -> bool {
        self.mtime.is_some()
            && self.mtime == file.mtime
            && self.size.is_none_or(|s| Some(s) == file.size)
    }
}

fn to_rgba(color: png::ColorType, pixels: Vec<u8>) -> Option<Vec<u8>> {
    let expand = |n: usize, f: fn(&[u8]) -> [u8; 4]| pixels.chunks_exact(n).flat_map(f).collect();
    Some(match color {
        png::ColorType::Rgba => pixels,
        png::ColorType::Rgb => expand(3, |p| [p[0], p[1], p[2], 255]),
        png::ColorType::GrayscaleAlpha => expand(2, |p| [p[0], p[0], p[0], p[1]]),
        png::ColorType::Grayscale => expand(1, |p| [p[0], p[0], p[0], 255]),
        png::ColorType::Indexed => return None,
    })
}

/// Writes atomically through a temporary file; the spec asks for a private directory and file.
fn write(path: &Path, uri: &str, stamp: &Stamp, thumb: &Thumb) -> io::Result<()> {
    let dir = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let temp = dir.join(format!(".fx-{}-{}.png", std::process::id(), thread_id()));
    let result = encode(&temp, uri, stamp, thumb).and_then(|()| fs::rename(&temp, path));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn thread_id() -> String {
    format!("{:?}", std::thread::current().id())
        .chars()
        .filter(char::is_ascii_digit)
        .collect()
}

fn encode(path: &Path, uri: &str, stamp: &Stamp, thumb: &Thumb) -> io::Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), thumb.width, thumb.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let texts = [
        (URI, uri.to_owned()),
        (MTIME, stamp.mtime.unwrap_or_default().to_string()),
        (SIZE, stamp.size.unwrap_or_default().to_string()),
        ("Software", "fx".to_owned()),
    ];
    for (keyword, text) in texts {
        encoder
            .add_text_chunk(keyword.to_owned(), text)
            .map_err(io::Error::other)?;
    }
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer
        .write_image_data(&thumb.rgba)
        .map_err(io::Error::other)?;
    writer.finish().map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, SystemTime};

    fn request(path: &Path, modified: SystemTime, size: u64) -> Request<'_> {
        Request {
            path,
            modified: Some(modified),
            size,
            px: 128,
        }
    }

    fn image(dir: &Path) -> (PathBuf, SystemTime, u64) {
        let path = dir.join("pic.png");
        image::RgbImage::from_pixel(300, 150, image::Rgb([10, 20, 30]))
            .save(&path)
            .unwrap();
        let meta = fs::metadata(&path).unwrap();
        (path, meta.modified().unwrap(), meta.len())
    }

    #[test]
    fn uri_and_hash_match_glib() {
        let path = Path::new("/home/jens/photos/me.png");
        assert_eq!(file_uri(path), "file:///home/jens/photos/me.png");
        // The example from the thumbnail specification.
        assert_eq!(
            hex_md5("file:///home/jens/photos/me.png"),
            "c6ee772d9e49320e97ec29a7eb5b1697"
        );
        assert_eq!(
            file_uri(Path::new("/a b/ü#?.jpg")),
            "file:///a%20b/%C3%BC%23%3F.jpg"
        );
    }

    #[test]
    fn a_miss_is_decoded_written_and_then_hit() {
        let cache_dir = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let cache = Cache::at(cache_dir.path().to_path_buf());
        let (path, modified, size) = image(files.path());
        let thumb = cache
            .thumbnail(&request(&path, modified, size), 128)
            .unwrap();
        assert_eq!((thumb.width, thumb.height), (128, 64));

        let name = format!("{}.png", hex_md5(&file_uri(&path)));
        let written = cache_dir.path().join("normal").join(&name);
        let mode = fs::metadata(&written).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let stamp = Stamp {
            mtime: Some(modified.duration_since(UNIX_EPOCH).unwrap().as_secs()),
            size: Some(size),
        };
        let hit = cache.lookup(&name, &stamp, 128).unwrap();
        assert_eq!(
            (hit.width, hit.height, hit.rgba),
            (thumb.width, thumb.height, thumb.rgba)
        );
    }

    #[test]
    fn a_changed_file_invalidates_its_thumbnail() {
        let cache_dir = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let cache = Cache::at(cache_dir.path().to_path_buf());
        let (path, modified, size) = image(files.path());
        cache
            .thumbnail(&request(&path, modified, size), 128)
            .unwrap();
        let name = format!("{}.png", hex_md5(&file_uri(&path)));
        let later = modified + Duration::from_secs(5);
        let stale = Stamp {
            mtime: Some(later.duration_since(UNIX_EPOCH).unwrap().as_secs()),
            size: Some(size),
        };
        assert!(cache.lookup(&name, &stale, 128).is_none());
    }

    #[test]
    fn a_larger_cached_size_serves_a_smaller_request() {
        let cache_dir = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let cache = Cache::at(cache_dir.path().to_path_buf());
        let (path, modified, size) = image(files.path());
        cache
            .thumbnail(&request(&path, modified, size), 256)
            .unwrap();
        assert!(cache_dir.path().join("large").is_dir());
        fs::remove_file(&path).unwrap();
        // The source is gone, so only the 256 px entry can answer, fitted down to 128.
        let thumb = cache
            .thumbnail(&request(&path, modified, size), 128)
            .unwrap();
        assert_eq!((thumb.width, thumb.height), (128, 64));
    }

    #[test]
    fn a_size_less_entry_is_valid_on_mtime_alone() {
        let entry = Stamp {
            mtime: Some(7),
            size: None,
        };
        let file = Stamp {
            mtime: Some(7),
            size: Some(99),
        };
        assert!(entry.matches(&file));
        assert!(
            !Stamp {
                mtime: None,
                size: None
            }
            .matches(&file)
        );
    }
}
