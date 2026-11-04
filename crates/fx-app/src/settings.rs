//! User settings as plain data, persisted as TOML in the platform config directory.

use crate::places::home_dir;
use fx_core::ConflictPolicy;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const MIN_FONT_SIZE: f32 = 8.0;
pub const MAX_FONT_SIZE: f32 = 32.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub show_hidden: bool,
    pub confirm_delete: bool,
    pub use_trash: bool,
    /// `None` starts in the home directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_folder: Option<PathBuf>,
    /// Theme id, e.g. `nord-dark`. Legacy `dark` and `light` are migrated on load.
    #[serde(deserialize_with = "theme_id")]
    pub theme: String,
    pub density: Density,
    pub show_age_column: bool,
    pub show_ext_column: bool,
    /// Open folders in the tree with a single click instead of a double click.
    pub tree_single_click: bool,
    /// Open folders in the list with a single click instead of a double click.
    pub list_single_click: bool,
    /// List the immediate subfolders in the background so entering one paints instantly.
    pub preload_subfolders: bool,
    /// `None` keeps the built-in font.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    /// Font size of list rows, clamped to `MIN_FONT_SIZE..=MAX_FONT_SIZE` on load.
    pub list_font_size: f32,
    /// Font size of tree rows, clamped to `MIN_FONT_SIZE..=MAX_FONT_SIZE` on load.
    pub tree_font_size: f32,
    /// Where folders sort relative to files.
    pub folder_order: FolderOrder,
    /// What a paste does when an item with the same name exists.
    pub conflict_policy: ConflictChoice,
    /// Copy workers per transfer; `0` lets the copy planner decide per storage type.
    pub copy_workers: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_hidden: false,
            confirm_delete: true,
            use_trash: true,
            start_folder: None,
            theme: DEFAULT_THEME.to_owned(),
            density: Density::default(),
            show_age_column: true,
            show_ext_column: true,
            tree_single_click: true,
            list_single_click: false,
            preload_subfolders: true,
            font_family: None,
            list_font_size: 13.5,
            tree_font_size: 13.5,
            folder_order: FolderOrder::default(),
            conflict_policy: ConflictChoice::default(),
            copy_workers: 0,
        }
    }
}

pub const DEFAULT_THEME: &str = "nord-dark";

/// Maps the pre-theming values `dark` and `light` to their Nord themes.
fn theme_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let id = String::deserialize(d)?;
    Ok(match id.as_str() {
        "dark" => "nord-dark".to_owned(),
        "light" => "nord-light".to_owned(),
        _ => id,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    #[default]
    Compact,
    Comfortable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderOrder {
    #[default]
    Top,
    Mixed,
    Bottom,
}

/// Persisted form of [`ConflictPolicy`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictChoice {
    #[default]
    Ask,
    Skip,
    Overwrite,
    KeepBoth,
    OverwriteIfNewer,
    SkipIdentical,
}

impl ConflictChoice {
    pub const ALL: [ConflictChoice; 6] = [
        ConflictChoice::Ask,
        ConflictChoice::Skip,
        ConflictChoice::Overwrite,
        ConflictChoice::KeepBoth,
        ConflictChoice::OverwriteIfNewer,
        ConflictChoice::SkipIdentical,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ConflictChoice::Ask => "Ask every time",
            ConflictChoice::Skip => "Skip",
            ConflictChoice::Overwrite => "Overwrite",
            ConflictChoice::KeepBoth => "Keep both, rename the copy",
            ConflictChoice::OverwriteIfNewer => "Overwrite if newer",
            ConflictChoice::SkipIdentical => "Skip identical files (resume)",
        }
    }
}

impl From<ConflictChoice> for ConflictPolicy {
    fn from(c: ConflictChoice) -> Self {
        match c {
            ConflictChoice::Ask => ConflictPolicy::Ask,
            ConflictChoice::Skip => ConflictPolicy::Skip,
            ConflictChoice::Overwrite => ConflictPolicy::Overwrite,
            ConflictChoice::KeepBoth => ConflictPolicy::KeepBoth,
            ConflictChoice::OverwriteIfNewer => ConflictPolicy::OverwriteIfNewer,
            ConflictChoice::SkipIdentical => ConflictPolicy::SkipIdentical,
        }
    }
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
        let mut settings = match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
                eprintln!("fx: ignoring corrupt settings {}: {err}", path.display());
                Self::default()
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                eprintln!("fx: cannot read settings {}: {err}", path.display());
                Self::default()
            }
        };
        settings.clamp_font_sizes();
        settings
    }

    /// Keeps font sizes within `MIN_FONT_SIZE..=MAX_FONT_SIZE`; NaN falls back to the default.
    fn clamp_font_sizes(&mut self) {
        let defaults = Self::default();
        self.list_font_size = clamp_font_size(self.list_font_size, defaults.list_font_size);
        self.tree_font_size = clamp_font_size(self.tree_font_size, defaults.tree_font_size);
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

    /// `<config dir>/fx/themes`, where user theme files live.
    pub fn themes_dir() -> Option<PathBuf> {
        config_dir().map(|dir| dir.join("fx").join("themes"))
    }

    /// `<config dir>/fx/settings.toml`, or `None` when the platform has no config directory.
    pub fn path() -> Option<PathBuf> {
        config_dir().map(|dir| dir.join("fx").join("settings.toml"))
    }
}

fn clamp_font_size(size: f32, default: f32) -> f32 {
    if size.is_nan() {
        default
    } else {
        size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
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
            theme: "solarized-dark".to_owned(),
            density: Density::Comfortable,
            show_age_column: false,
            show_ext_column: false,
            tree_single_click: false,
            list_single_click: true,
            preload_subfolders: false,
            font_family: Some("Inter".to_string()),
            list_font_size: 16.0,
            tree_font_size: 10.5,
            folder_order: FolderOrder::Bottom,
            conflict_policy: ConflictChoice::OverwriteIfNewer,
            copy_workers: 6,
        };
        settings.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), settings);
        assert!(!path.with_extension("toml.tmp").exists());
    }

    #[test]
    fn legacy_theme_values_are_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        for (old, new) in [
            ("dark", "nord-dark"),
            ("light", "nord-light"),
            ("dracula", "dracula"),
        ] {
            write(&path, &format!("theme = \"{old}\"\n"));
            assert_eq!(Settings::load_from(&path).theme, new);
        }
        assert_eq!(Settings::default().theme, DEFAULT_THEME);
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
        assert_eq!(loaded.theme, "nord-light");
        assert!(loaded.confirm_delete, "missing fields keep their defaults");
        assert!(loaded.use_trash);
        assert!(loaded.preload_subfolders);
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
    fn file_without_new_keys_gets_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        write(&path, "show_hidden = true\ntheme = \"light\"\n");
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.font_family, None);
        assert_eq!(loaded.list_font_size, 13.5);
        assert_eq!(loaded.tree_font_size, 13.5);
        assert_eq!(loaded.folder_order, FolderOrder::Top);
    }

    #[test]
    fn font_sizes_are_clamped_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        write(&path, "list_font_size = 2.0\ntree_font_size = 99.0\n");
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.list_font_size, MIN_FONT_SIZE);
        assert_eq!(loaded.tree_font_size, MAX_FONT_SIZE);

        write(&path, "list_font_size = nan\ntree_font_size = 20.0\n");
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.list_font_size, Settings::default().list_font_size);
        assert_eq!(loaded.tree_font_size, 20.0);
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
