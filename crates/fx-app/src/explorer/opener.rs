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
    log::debug!("opening {} with {OPENER}", path.display());
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
