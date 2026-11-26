//! The Windows shell thumbnailer: `IShellItemImageFactory` answers from `thumbcache_*.db` and runs the
//! registered thumbnail handlers on a miss, so videos, PDFs and Office files get previews as in Explorer.

use crate::{Request, Thumb, ThumbError, decode};
use std::ffi::c_void;
use std::marker::PhantomData;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_THUMBNAILONLY,
};
use windows::core::HSTRING;

/// COM is initialised for the thread that creates this, so it must stay on that thread.
pub(crate) struct Shell {
    com: bool,
    _thread_bound: PhantomData<*const ()>,
}

impl Shell {
    pub(crate) fn new() -> Self {
        // SAFETY: plain COM initialisation of the current thread; balanced in `Drop` when it succeeded.
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        Self {
            com,
            _thread_bound: PhantomData,
        }
    }

    pub(crate) fn thumbnail(&self, request: &Request, px: u32) -> Result<Thumb, ThumbError> {
        let edge = i32::try_from(px).unwrap_or(i32::MAX);
        let size = SIZE { cx: edge, cy: edge };
        let path = HSTRING::from(request.path.as_os_str());
        // SAFETY: `path` outlives the call; the factory is released when dropped.
        let factory: IShellItemImageFactory = unsafe { SHCreateItemFromParsingName(&path, None) }
            .map_err(|e| ThumbError::Decode(e.to_string()))?;
        // Thumbnail only: without it the shell falls back to the file type icon, which fx draws itself.
        // SAFETY: COM call on a valid interface; the returned bitmap is owned and deleted below.
        let bitmap = unsafe { factory.GetImage(size, SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK) }
            .map_err(|_| ThumbError::Unsupported)?;
        let pixels = read_bitmap(bitmap);
        // SAFETY: the bitmap came from GetImage and is not used afterwards.
        let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
        let image = pixels.ok_or_else(|| ThumbError::Decode("unreadable shell bitmap".into()))?;
        decode::fit(image::DynamicImage::ImageRgba8(image), px)
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        if self.com {
            // SAFETY: balances the successful CoInitializeEx of this thread.
            unsafe { CoUninitialize() };
        }
    }
}

/// The bitmap as top-down straight-alpha RGBA.
fn read_bitmap(bitmap: HBITMAP) -> Option<image::RgbaImage> {
    let mut info = BITMAP::default();
    let wanted = i32::try_from(size_of::<BITMAP>()).ok()?;
    // SAFETY: `info` is a BITMAP of the size passed.
    let got = unsafe {
        GetObjectW(
            HGDIOBJ(bitmap.0),
            wanted,
            Some((&raw mut info).cast::<c_void>()),
        )
    };
    let (w, h) = (
        u32::try_from(info.bmWidth).ok()?,
        u32::try_from(info.bmHeight).ok()?,
    );
    if got == 0 || w == 0 || h == 0 {
        return None;
    }
    let mut header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).ok()?,
            biWidth: info.bmWidth,
            // Negative height asks for top-down rows.
            biHeight: -info.bmHeight,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bgra = vec![0u8; w as usize * h as usize * 4];
    // SAFETY: a memory DC for GetDIBits; `bgra` holds exactly `h` rows of 32-bit pixels.
    let lines = unsafe {
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            h,
            Some(bgra.as_mut_ptr().cast()),
            &raw mut header,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        lines
    };
    if lines <= 0 {
        return None;
    }
    image::RgbaImage::from_raw(w, h, to_straight_rgba(bgra))
}

/// Shell bitmaps are premultiplied BGRA; opaque images often come with an all-zero alpha channel.
fn to_straight_rgba(mut bgra: Vec<u8>) -> Vec<u8> {
    let opaque = bgra.as_chunks::<4>().0.iter().all(|p| p[3] == 0);
    for p in bgra.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
        if opaque {
            p[3] = 255;
            continue;
        }
        decode::unpremultiply(p);
    }
    bgra
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn zero_alpha_means_opaque_and_premultiplied_is_undone() {
        assert_eq!(to_straight_rgba(vec![10, 20, 30, 0]), vec![30, 20, 10, 255]);
        assert_eq!(
            to_straight_rgba(vec![0, 0, 64, 128, 0, 0, 0, 0]),
            vec![127, 0, 0, 128, 0, 0, 0, 0]
        );
    }

    #[test]
    fn the_shell_thumbnails_a_png() {
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
        let thumb = Shell::new().thumbnail(&request, 128).unwrap();
        assert!(thumb.width <= 128 && thumb.height <= 128);
        assert!(thumb.width > thumb.height);
    }
}
