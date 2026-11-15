//! Handing files to the operating system's default or chosen application.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

#[cfg(target_os = "windows")]
const OPENER: &str = "explorer";
#[cfg(target_os = "macos")]
const OPENER: &str = "open";
#[cfg(all(unix, not(target_os = "macos")))]
const OPENER: &str = "xdg-open";

/// Spawns the OS opener and does not wait for it. The child is not reaped; a waiter thread would be needed if opens become frequent.
// ponytail: unreaped child per open; add a reaper thread if opens get frequent
pub fn open_with_system(path: &Path) -> io::Result<()> {
    Command::new(OPENER)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|mut child| {
            // Reap the child so it does not linger as a zombie.
            thread::spawn(move || child.wait());
        })
}

/// Shows the Windows "Open with" dialog for `path` and opens it with the chosen app.
/// The dialog is modal, so it runs on its own thread and the window stays live. Cancelling is not an error, so the result is not reported.
#[cfg(windows)]
pub fn open_with_chooser(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    let file: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    thread::spawn(move || show_open_with_dialog(&file));
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
        SHOpenWithDialog(std::ptr::null_mut(), &info);
        if com >= 0 {
            CoUninitialize();
        }
    }
}
