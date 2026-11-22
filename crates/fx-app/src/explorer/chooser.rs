//! The operating system's own "Open with" application chooser.

use std::path::Path;
use std::thread;

/// Lets the user pick the application for `path` in the system's chooser and opens the file with it.
/// The chooser is modal and slow, so it runs on its own thread and the window stays live.
/// Cancelling is not an error. `on_unavailable` is called, from the chooser's thread, only when the system offers no chooser.
pub fn open_with_chooser(path: &Path, on_unavailable: impl FnOnce() + Send + 'static) {
    let path = path.to_path_buf();
    thread::spawn(move || choose_and_open(&path, on_unavailable));
}

#[cfg(windows)]
fn choose_and_open(path: &Path, _on_unavailable: impl FnOnce()) {
    use std::os::windows::ffi::OsStrExt;
    let file: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    show_open_with_dialog(&file);
}

/// Runs the modal "Open with" dialog for the NUL-terminated wide path `file` on the calling thread.
#[cfg(windows)]
fn show_open_with_dialog(file: &[u16]) {
    use windows_sys::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
    };
    use windows_sys::Win32::UI::Shell::{
        OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OPENASINFO, SHOpenWithDialog,
    };
    let info = OPENASINFO {
        pcszFile: file.as_ptr(),
        pcszClass: std::ptr::null(),
        oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_EXEC,
    };
    // SAFETY: shell dialogs need COM on the calling thread; `file` is NUL-terminated and outlives the call.
    unsafe {
        let com = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let hr = SHOpenWithDialog(std::ptr::null_mut(), &info);
        if hr < 0 {
            log::debug!("SHOpenWithDialog failed: HRESULT {hr:#x}");
        }
        if com >= 0 {
            CoUninitialize();
        }
    }
}

/// The desktop portal's `OpenFile` with `ask` shows the desktop's native chooser, in or out of a sandbox.
#[cfg(target_os = "linux")]
fn choose_and_open(path: &Path, on_unavailable: impl FnOnce()) {
    if let Err(err) = ask_portal(path) {
        log::warn!("open-with portal failed for {}: {err}", path.display());
        on_unavailable();
    }
}

#[cfg(target_os = "linux")]
fn ask_portal(path: &Path) -> zbus::Result<()> {
    use std::collections::HashMap;
    use zbus::zvariant::{Fd, Value};
    let file = std::fs::File::open(path)?;
    let options = HashMap::from([("ask", Value::from(true))]);
    zbus::blocking::Connection::session()?.call_method(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        Some("org.freedesktop.portal.OpenURI"),
        "OpenFile",
        &("", Fd::from(&file), options),
    )?;
    Ok(())
}

/// macOS has no chooser command, so AppleScript's `choose application` picks the app and `open -a` runs it.
#[cfg(target_os = "macos")]
fn choose_and_open(path: &Path, _on_unavailable: impl FnOnce()) {
    use std::process::Command;
    let out = match Command::new("osascript")
        .args(["-e", "POSIX path of (choose application)"])
        .output()
    {
        Ok(out) if out.status.success() => out,
        Ok(out) => return log::debug!("no application chosen: {}", out.status),
        Err(err) => return log::warn!("cannot run osascript: {err}"),
    };
    let app = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if let Err(err) = Command::new("open").arg("-a").arg(&app).arg(path).status() {
        log::warn!("open -a {app} {} failed: {err}", path.display());
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn choose_and_open(_path: &Path, on_unavailable: impl FnOnce()) {
    on_unavailable();
}
