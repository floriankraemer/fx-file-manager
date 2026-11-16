//! Drags files out of fx into other programs through the operating system's own drag and drop.

use drag::{DragItem, Image, Options};
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use std::path::PathBuf;

const PREVIEW: &[u8] = include_bytes!("../../../../assets/drag-icon.png");

/// The window the drag leaves from: whichever fx window has focus, since a drag only starts under the pointer.
struct Source(RawWindowHandle);

impl HasWindowHandle for Source {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: the handle was just read from the focused window, which outlives this call.
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

/// Starts dragging `paths`; Windows keeps control until the drop, macOS returns at once.
pub fn start(paths: Vec<PathBuf>) {
    let Some(handle) = platform::focused_window() else {
        return;
    };
    let result = drag::start_drag(
        &Source(handle),
        DragItem::Files(paths),
        Image::Raw(PREVIEW.to_vec()),
        |_, _| {},
        Options::default(),
    );
    if let Err(err) = result {
        eprintln!("fx: starting the drag failed: {err}");
    }
}

#[cfg(windows)]
mod platform {
    use raw_window_handle::{RawWindowHandle, Win32WindowHandle};
    use std::num::NonZeroIsize;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    pub fn focused_window() -> Option<RawWindowHandle> {
        // SAFETY: GetForegroundWindow takes no arguments and only reads window manager state.
        let hwnd = unsafe { GetForegroundWindow() };
        let hwnd = NonZeroIsize::new(hwnd as isize)?;
        Some(RawWindowHandle::Win32(Win32WindowHandle::new(hwnd)))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::NSApplication;
    use raw_window_handle::{AppKitWindowHandle, RawWindowHandle};
    use std::ptr::NonNull;

    pub fn focused_window() -> Option<RawWindowHandle> {
        let app = NSApplication::sharedApplication(MainThreadMarker::new()?);
        let view = app.keyWindow()?.contentView()?;
        let view = NonNull::new(Retained::as_ptr(&view) as *mut _)?;
        Some(RawWindowHandle::AppKit(AppKitWindowHandle::new(view)))
    }
}
