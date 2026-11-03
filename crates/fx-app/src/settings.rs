//! User settings as plain data, persisted as TOML in the platform config directory.

use crate::places::home_dir;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub show_hidden: bool,
    pub confirm_delete: bool,
    pub use_trash: bool,
    /// `None` starts in the home directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_folder: Option<PathBuf>,
    pub theme: ThemeChoice,
    pub density: Density,
    pub show_age_column: bool,
    pub show_ext_column: bool,
    /// Open folders in the tree with a single click instead of a double click.
    pub tree_single_click: bool,
    /// Open folders in the list with a single click instead of a double click.
    pub list_single_click: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_hidden: false,
            confirm_delete: true,
            use_trash: true,
            start_folder: None,
            theme: ThemeChoice::default(),
            density: Density::default(),
            show_age_column: true,
            show_ext_column: true,
            tree_single_click: true,
            list_single_click: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    #[default]
    Compact,
    Comfortable,
}

impl Settings {
    /// Loads from [`Settings::path`]. A missing or unreadable file yields defaults; a corrupt one also logs to stderr.
    pub fn load() -> Self {
        match Self::path() {
            Some(path) => Self::load_from(&path),
            None => Self::default(),
        }
    }

    pub fn load_from(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
                eprintln!("fx: ignoring corrupt settings {}: {err}", path.display());
                Self::default()
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                eprintln!("fx: cannot read settings {}: {err}", path.display());
                Self::default()
            }
        }
    }

    /// Saves to [`Settings::path`]. Fails if there is no config directory.
    pub fn save(&self) -> io::Result<()> {
        let path = Self::path().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no config directory available")
        })?;
        self.save_to(&path)
    }

    /// Writes a temporary file next to `path`, syncs it, then renames it over `path`, so readers never see a partial file.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
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

    /// `<config dir>/fx/settings.toml`, or `None` when the platform has no config directory.
    pub fn path() -> Option<PathBuf> {
        config_dir().map(|dir| dir.join("fx").join("settings.toml"))
    }
}

#[cfg(windows)]
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn config_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join("Library").join("Application Support"))
}

/// XDG: `$XDG_CONFIG_HOME` only when absolute, as the spec requires; otherwise `~/.config`.
#[cfg(all(unix, not(target_os = "macos")))]
fn config_dir() -> Option<PathBuf> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn roundtrips_every_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fx").join("settings.toml");
        let settings = Settings {
            show_hidden: true,
            confirm_delete: false,
            use_trash: false,
            start_folder: Some(dir.path().to_path_buf()),
            theme: ThemeChoice::Light,
            density: Density::Comfortable,
            show_age_column: false,
            show_ext_column: false,
            tree_single_click: false,
            list_single_click: true,
        };
        settings.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), settings);
        assert!(!path.with_extension("toml.tmp").exists());
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Settings::load_from(&dir.path().join("nope.toml")),
            Settings::default()
        );
    }

    #[test]
    fn corrupt_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        write(&path, "show_hidden = [not toml");
        assert_eq!(Settings::load_from(&path), Settings::default());
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        write(
            &path,
            "show_hidden = true\nfuture_option = 42\ntheme = \"light\"\n",
        );
        let loaded = Settings::load_from(&path);
        assert!(loaded.show_hidden);
        assert_eq!(loaded.theme, ThemeChoice::Light);
        assert!(loaded.confirm_delete, "missing fields keep their defaults");
        assert!(loaded.use_trash);
    }

    #[test]
    fn save_creates_missing_directories_and_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("settings.toml");
        Settings::default().save_to(&path).unwrap();
        let changed = Settings {
            show_hidden: true,
            ..Settings::default()
        };
        changed.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), changed);
    }

    #[test]
    fn start_dir_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_path_buf();
        let cli = PathBuf::from("/cli/wins");
        let with_folder = Settings {
            start_folder: Some(folder.clone()),
            ..Settings::default()
        };
        assert_eq!(start_dir(Some(cli.clone()), &with_folder), cli);
        assert_eq!(start_dir(None, &with_folder), folder);

        let gone = Settings {
            start_folder: Some(dir.path().join("missing")),
            ..Settings::default()
        };
        let fallback = home_dir().unwrap_or_else(|| PathBuf::from("/"));
        assert_eq!(start_dir(None, &gone), fallback);
        assert_eq!(start_dir(None, &Settings::default()), fallback);
    }
}
