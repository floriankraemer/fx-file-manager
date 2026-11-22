use super::Settings;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Writes a temporary file next to `path`, syncs it, then renames it over `path`, so readers never see a partial file.
pub(crate) fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    let mut file = File::create(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)
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

/// Chooses the folder to open at startup: the CLI argument, then the settings' start folder if it still exists, then home, then `/`.
pub fn start_dir(cli_arg: Option<PathBuf>, settings: &Settings) -> PathBuf {
    cli_arg
        .or_else(|| settings.start_folder.clone().filter(|dir| dir.is_dir()))
        .or_else(home_dir)
        .unwrap_or_else(|| PathBuf::from("/"))
}
