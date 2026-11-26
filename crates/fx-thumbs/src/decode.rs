//! In-process thumbnails for common image formats, used where the OS offers no thumbnailer.

use crate::{Thumb, ThumbError};
use fast_image_resize::images::Image;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader, Limits, RgbaImage};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Larger files are not decoded: reading them costs more than a preview is worth.
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// Decoders allocate width x height buffers, so arbitrary files are capped by pixel count too.
const MAX_PIXELS: u64 = 100_000_000;
const MAX_ALLOC: u64 = 1024 * 1024 * 1024;

const IMAGE_EXTENSIONS: [&str; 9] = [
    "jpg", "jpeg", "jpe", "png", "gif", "webp", "bmp", "tif", "tiff",
];

/// True if this module can decode the file, judged by its extension.
pub(crate) fn supports(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Decodes the image at `path` and fits it into a `px` square.
pub(crate) fn thumbnail(path: &Path, px: u32) -> Result<Thumb, ThumbError> {
    if !supports(path) {
        return Err(ThumbError::Unsupported);
    }
    if std::fs::metadata(path)?.len() > MAX_FILE_BYTES {
        return Err(ThumbError::TooLarge);
    }
    let image = if is_jpeg(path) {
        decode_jpeg(path, px)?
    } else {
        decode_any(path)?
    };
    fit(image, px)
}

fn is_jpeg(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["jpg", "jpeg", "jpe"]
            .iter()
            .any(|x| e.eq_ignore_ascii_case(x))
    })
}

fn check_pixels(width: u32, height: u32) -> Result<(), ThumbError> {
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(ThumbError::TooLarge);
    }
    Ok(())
}

fn decode_error(e: impl std::fmt::Display) -> ThumbError {
    ThumbError::Decode(e.to_string())
}

/// JPEG through the scaled IDCT: a 24 MP photo decodes at 1/8 size, without ever building the full image.
fn decode_jpeg(path: &Path, px: u32) -> Result<DynamicImage, ThumbError> {
    let mut decoder = jpeg_decoder::Decoder::new(BufReader::new(File::open(path)?));
    decoder.read_info().map_err(decode_error)?;
    let info = decoder.info().ok_or(ThumbError::Unsupported)?;
    check_pixels(info.width.into(), info.height.into())?;
    let target = u16::try_from(px).unwrap_or(u16::MAX);
    decoder.scale(target, target).map_err(decode_error)?;
    let pixels = decoder.decode().map_err(decode_error)?;
    let info = decoder.info().ok_or(ThumbError::Unsupported)?;
    let mut image = jpeg_image(info, pixels)?;
    if let Some(orientation) = decoder.exif_data().and_then(Orientation::from_exif_chunk) {
        image.apply_orientation(orientation);
    }
    Ok(image)
}

fn jpeg_image(info: jpeg_decoder::ImageInfo, pixels: Vec<u8>) -> Result<DynamicImage, ThumbError> {
    use jpeg_decoder::PixelFormat;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    let invalid = || ThumbError::Decode("pixel buffer does not match the image size".into());
    let image = match info.pixel_format {
        PixelFormat::RGB24 => {
            DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, pixels).ok_or_else(invalid)?)
        }
        PixelFormat::L8 => {
            DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, pixels).ok_or_else(invalid)?)
        }
        PixelFormat::L16 => {
            let wide = pixels
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_be_bytes(*b))
                .collect();
            DynamicImage::ImageLuma16(image::ImageBuffer::from_raw(w, h, wide).ok_or_else(invalid)?)
        }
        PixelFormat::CMYK32 => {
            DynamicImage::ImageRgb8(cmyk_to_rgb(w, h, &pixels).ok_or_else(invalid)?)
        }
    };
    Ok(image)
}

/// jpeg-decoder returns Adobe-inverted CMYK; this is the usual naive conversion, good enough for a preview.
fn cmyk_to_rgb(w: u32, h: u32, cmyk: &[u8]) -> Option<image::RgbImage> {
    let rgb = cmyk
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            let k = u16::from(p[3]);
            [0, 1, 2].map(|i| (u16::from(p[i]) * k / 255) as u8)
        })
        .collect();
    image::RgbImage::from_raw(w, h, rgb)
}

fn decode_any(path: &Path) -> Result<DynamicImage, ThumbError> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let (w, h) = decoder.dimensions();
    check_pixels(w, h)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// Scales `image` down to fit into a `px` square, keeping its aspect ratio. Smaller images stay as they are.
pub(crate) fn fit(image: DynamicImage, px: u32) -> Result<Thumb, ThumbError> {
    let rgba = image.into_rgba8();
    let (w, h) = rgba.dimensions();
    if w <= px && h <= px {
        return Ok(into_thumb(rgba));
    }
    let (nw, nh) = fitted(w, h, px);
    let source = DynamicImage::ImageRgba8(rgba);
    let mut target = Image::new(nw, nh, PixelType::U8x4);
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear));
    Resizer::new()
        .resize(&source, &mut target, &options)
        .map_err(decode_error)?;
    Ok(Thumb {
        width: nw,
        height: nh,
        rgba: target.into_vec(),
    })
}

/// Turns one premultiplied RGBA pixel into straight alpha.
#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn unpremultiply(p: &mut [u8; 4]) {
    let a = u16::from(p[3]);
    if a == 0 || a == 255 {
        return;
    }
    for c in &mut p[..3] {
        *c = (u16::from(*c) * 255 / a).min(255) as u8;
    }
}

fn into_thumb(rgba: RgbaImage) -> Thumb {
    Thumb {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    }
}

/// The size of a `w` x `h` image scaled to fit into a `px` square; never zero.
fn fitted(w: u32, h: u32, px: u32) -> (u32, u32) {
    let scale = f64::from(px) / f64::from(w.max(h));
    let side = |v: u32| ((f64::from(v) * scale).round() as u32).max(1);
    (side(w), side(h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgb, RgbImage};

    fn write(dir: &Path, name: &str, w: u32, h: u32, format: ImageFormat) -> std::path::PathBuf {
        let path = dir.join(name);
        RgbImage::from_fn(w, h, |x, _| Rgb([(x % 256) as u8, 40, 200]))
            .save_with_format(&path, format)
            .unwrap();
        path
    }

    #[test]
    fn fitted_keeps_the_aspect_ratio() {
        assert_eq!(fitted(4000, 3000, 128), (128, 96));
        assert_eq!(fitted(1000, 4000, 256), (64, 256));
        assert_eq!(fitted(10_000, 1, 128), (128, 1));
    }

    #[test]
    fn jpeg_is_decoded_scaled_and_fitted() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "photo.JPG", 2000, 1000, ImageFormat::Jpeg);
        let thumb = thumbnail(&path, 128).unwrap();
        assert_eq!((thumb.width, thumb.height), (128, 64));
        assert_eq!(thumb.rgba.len(), 128 * 64 * 4);
    }

    #[test]
    fn png_is_fitted_and_small_images_are_not_upscaled() {
        let dir = tempfile::tempdir().unwrap();
        let big = write(dir.path(), "big.png", 600, 300, ImageFormat::Png);
        let thumb = thumbnail(&big, 256).unwrap();
        assert_eq!((thumb.width, thumb.height), (256, 128));
        let small = write(dir.path(), "small.png", 40, 20, ImageFormat::Png);
        let thumb = thumbnail(&small, 256).unwrap();
        assert_eq!((thumb.width, thumb.height), (40, 20));
    }

    #[test]
    fn other_files_are_unsupported_and_garbage_fails_to_decode() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("notes.txt");
        std::fs::write(&text, "hello").unwrap();
        assert!(matches!(
            thumbnail(&text, 128),
            Err(ThumbError::Unsupported)
        ));
        let fake = dir.path().join("fake.png");
        std::fs::write(&fake, "not a png").unwrap();
        assert!(matches!(thumbnail(&fake, 128), Err(ThumbError::Decode(_))));
    }

    #[test]
    fn oversized_pixel_counts_are_rejected_before_decoding() {
        assert!(matches!(
            check_pixels(20_000, 20_000),
            Err(ThumbError::TooLarge)
        ));
        assert!(check_pixels(10_000, 10_000).is_ok());
    }
}
