use super::Settings;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Once;

/// Writes a uniquely named temporary file next to `path`, syncs it, then renames it over `path`, so readers never see a partial file.
pub(crate) fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("toml.{}.tmp", std::process::id()));
    let result = write_synced(&tmp, text).and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
        return result;
    }
    sync_parent_dir(path);
    Ok(())
}

fn write_synced(tmp: &Path, text: &str) -> io::Result<()> {
    let mut file = File::create(tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

#[cfg(unix)]
fn sync_parent_dir(path: &Path) {
    let Some(dir) = path.parent() else { return };
    if let Err(err) = File::open(dir).and_then(|dir| dir.sync_all()) {
        log::debug!("cannot sync {}: {err}", dir.display());
    }
}

#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) {}

/// Moves an unparsable config file to `<name>.corrupt` (replacing an older one) so the next save cannot destroy the user's edits.
pub(crate) fn quarantine_corrupt(path: &Path, err: &dyn std::fmt::Display) {
    let mut backup = path.as_os_str().to_owned();
    backup.push(".corrupt");
    match fs::rename(path, &backup) {
        Ok(()) => log::error!(
            "corrupt config {} ({err}); kept as {}, using defaults",
            path.display(),
            Path::new(&backup).display()
        ),
        Err(rename_err) => log::error!(
            "corrupt config {} ({err}); backup failed ({rename_err}), using defaults",
            path.display()
        ),
    }
}

pub(crate) fn warn_no_config_dir() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| log::warn!("no config directory; settings and state are not persisted"));
}

/// `$HOME`, else `%USERPROFILE%`.
pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(windows)]
pub(crate) fn config_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
pub(crate) fn config_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join("Library").join("Application Support"))
}

/// XDG: `$XDG_CONFIG_HOME` only when absolute, as the spec requires; otherwise `~/.config`.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| home_dir().map(|home| home.join(".config")))
}

/// Chooses the folder to open at startup: the CLI argument, then the settings' start folder, then home, then `/`.
/// The start folder is not probed, which could block on a dead mount; `Explorer::with_state` replaces it with home once a background check finds it gone.
pub fn start_dir(cli_arg: Option<PathBuf>, settings: &Settings) -> PathBuf {
    cli_arg
        .or_else(|| settings.start_folder.clone())
        .or_else(home_dir)
        .unwrap_or_else(|| PathBuf::from("/"))
}
