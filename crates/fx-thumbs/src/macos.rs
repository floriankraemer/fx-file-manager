//! QuickLook thumbnails: `QLThumbnailGenerator` answers from the system thumbnail cache and runs the
//! QuickLook extensions on a miss, so every type Finder previews gets one here too.

use crate::{Request, Thumb, ThumbError, decode};
use block2::RcBlock;
use objc2::AllocAnyThread;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo,
};
use objc2_foundation::{NSError, NSString, NSURL};
use objc2_quick_look_thumbnailing::{
    QLThumbnailGenerationRequest, QLThumbnailGenerationRequestRepresentationTypes,
    QLThumbnailGenerator, QLThumbnailRepresentation,
};
use std::sync::mpsc;
use std::time::Duration;

/// A generator that never answers must not hold a worker forever.
const TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn thumbnail(request: &Request, px: u32) -> Result<Thumb, ThumbError> {
    let path = request.path.to_str().ok_or(ThumbError::Unsupported)?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let edge = f64::from(px);
    // SAFETY: an initialiser on a fresh allocation, the documented way to build the request.
    let ql_request = unsafe {
        QLThumbnailGenerationRequest::initWithFileAtURL_size_scale_representationTypes(
            QLThumbnailGenerationRequest::alloc(),
            &url,
            CGSize::new(edge, edge),
            1.0,
            QLThumbnailGenerationRequestRepresentationTypes::Thumbnail,
        )
    };
    let (tx, rx) = mpsc::sync_channel(1);
    let handler = RcBlock::new(
        move |rep: *mut QLThumbnailRepresentation, _: *mut NSError| {
            // SAFETY: QuickLook passes a valid representation or null for the duration of the call.
            let image = unsafe { rep.as_ref() }.and_then(|rep| {
                let cg = unsafe { rep.CGImage() };
                rgba(&cg)
            });
            let _ = tx.send(image);
        },
    );
    // SAFETY: the shared generator is thread safe; the handler is retained by the call.
    let generator = unsafe { QLThumbnailGenerator::sharedGenerator() };
    unsafe {
        generator.generateBestRepresentationForRequest_completionHandler(&ql_request, &handler)
    };
    let Ok(answer) = rx.recv_timeout(TIMEOUT) else {
        // SAFETY: cancelling a request this generator owns.
        unsafe { generator.cancelRequest(&ql_request) };
        return Err(ThumbError::Unsupported);
    };
    let image = answer.ok_or(ThumbError::Unsupported)?;
    decode::fit(image::DynamicImage::ImageRgba8(image), px)
}

/// Draws `image` into a straight-alpha RGBA buffer.
fn rgba(image: &CGImage) -> Option<image::RgbaImage> {
    let (w, h) = (CGImage::width(Some(image)), CGImage::height(Some(image)));
    if w == 0 || h == 0 {
        return None;
    }
    let mut pixels = vec![0u8; w * h * 4];
    let space = CGColorSpace::new_device_rgb()?;
    let info = CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::Order32Big.0;
    // SAFETY: `pixels` holds `h` rows of `w * 4` bytes and outlives the context, dropped right after drawing.
    let context = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            w,
            h,
            8,
            w * 4,
            Some(&space),
            info,
        )
    }?;
    let rect = CGRect::new(CGPoint::ZERO, CGSize::new(w as f64, h as f64));
    CGContext::draw_image(Some(&context), rect, Some(image));
    drop(context);
    for p in pixels.as_chunks_mut::<4>().0 {
        decode::unpremultiply(p);
    }
    image::RgbaImage::from_raw(u32::try_from(w).ok()?, u32::try_from(h).ok()?, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn quicklook_thumbnails_a_png() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pic.png");
        image::RgbImage::from_pixel(300, 150, image::Rgb([200, 30, 30]))
            .save(&path)
            .unwrap();
        let request = Request {
            path: &path,
            modified: Some(SystemTime::now()),
            size: 0,
            px: 128,
        };
        let thumb = thumbnail(&request, 128).unwrap();
        assert!(thumb.width <= 128 && thumb.height <= 128);
        assert!(thumb.width > thumb.height);
    }
}
