//! User settings as plain data, persisted as TOML in the platform config directory.

use crate::columns::ListColumn;
use crate::i18n::Language;
use crate::place_group::PlaceGroup;
use fx_core::{ConflictPolicy, GlobScope};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const MIN_FONT_SIZE: f32 = 8.0;
pub const MAX_FONT_SIZE: f32 = 32.0;
/// Smallest accepted `search.result_cap`.
pub const MIN_RESULT_CAP: usize = 100;

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
    /// Shown columns of the file list, left to right. Always contains `Name`.
    pub list_columns: Vec<ListColumn>,
    /// Open folders in the tree with a single click instead of a double click.
    pub tree_single_click: bool,
    /// Open folders in the list with a single click instead of a double click.
    pub list_single_click: bool,
    /// List the immediate subfolders in the background so entering one paints instantly.
    pub preload_subfolders: bool,
    /// Show the network share next to a mapped drive letter in the sidebar (Windows).
    pub show_share_names: bool,
    /// Top-to-bottom order of the sidebar groups; see [`Settings::sidebar_groups`].
    pub sidebar_order: Vec<PlaceGroup>,
    pub hidden_sidebar_groups: Vec<PlaceGroup>,
    /// `None` keeps the built-in font.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    /// Font size of list rows, clamped to `MIN_FONT_SIZE..=MAX_FONT_SIZE` on load.
    pub list_font_size: f32,
    /// Font size of tree rows, clamped to `MIN_FONT_SIZE..=MAX_FONT_SIZE` on load.
    pub tree_font_size: f32,
    /// Where folders sort relative to files.
    pub folder_order: FolderOrder,
    /// UI language; `system` follows the OS.
    pub language: Language,
    /// Date order of the Modified column: the language's own or ISO.
    pub date_format: DateFormat,
    /// What a paste does when an item with the same name exists.
    pub conflict_policy: ConflictChoice,
    /// Copy workers per transfer; `0` lets the copy planner decide per storage type.
    pub copy_workers: u32,
    /// Read every copied file back and compare checksums.
    pub verify_copies: bool,
    /// Reopen the windows and tabs of the last session on launch.
    pub restore_session: bool,
    /// Held while dropping dragged items, copies them instead of moving them.
    pub drag_copy_modifier: DragModifier,
    /// Shortcut overrides: command id to shortcut strings such as `"ctrl+shift+n"`. An empty list unbinds.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub keymap: BTreeMap<String, Vec<String>>,
    /// Widths the user dragged list columns to. The last column fills the rest and has none.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub list_column_widths: BTreeMap<ListColumn, f32>,
    /// Recursive search below the current folder.
    pub search: SearchSettings,
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
            list_columns: ListColumn::ALL.to_vec(),
            tree_single_click: true,
            list_single_click: false,
            preload_subfolders: true,
            show_share_names: true,
            sidebar_order: PlaceGroup::ALL.to_vec(),
            hidden_sidebar_groups: Vec::new(),
            font_family: None,
            list_font_size: 13.5,
            tree_font_size: 13.5,
            folder_order: FolderOrder::default(),
            language: Language::default(),
            date_format: DateFormat::default(),
            conflict_policy: ConflictChoice::default(),
            copy_workers: 0,
            verify_copies: false,
            restore_session: true,
            drag_copy_modifier: DragModifier::default(),
            keymap: BTreeMap::new(),
            list_column_widths: BTreeMap::new(),
            search: SearchSettings::default(),
        }
    }
}

impl Settings {
    /// The visible sidebar groups in the saved order.
    pub fn sidebar_groups(&self) -> Vec<PlaceGroup> {
        let mut out = self.sidebar_order_full();
        out.retain(|g| self.sidebar_group_shown(*g));
        out
    }

    pub fn sidebar_group_shown(&self, group: PlaceGroup) -> bool {
        !self.hidden_sidebar_groups.contains(&group)
    }

    pub fn set_sidebar_group_shown(&mut self, group: PlaceGroup, shown: bool) {
        self.hidden_sidebar_groups.retain(|g| *g != group);
        if !shown {
            self.hidden_sidebar_groups.push(group);
        }
    }

    /// Moves `group` to the slot `target` occupies, shifting the others. Hidden groups keep their slots.
    pub fn move_sidebar_group(&mut self, group: PlaceGroup, target: PlaceGroup) {
        let mut order = self.sidebar_order_full();
        move_to_slot_of(&mut order, group, target);
        self.sidebar_order = order;
    }

    /// The saved order completed: groups missing from a hand-edited file go last, repeats are dropped.
    fn sidebar_order_full(&self) -> Vec<PlaceGroup> {
        let mut out: Vec<PlaceGroup> = Vec::new();
        for g in self.sidebar_order.iter().chain(&PlaceGroup::ALL) {
            if !out.contains(g) {
                out.push(*g);
            }
        }
        out
    }
}

/// Moves `item` to the slot `target` occupies, shifting the others.
fn move_to_slot_of<T: PartialEq + Copy>(items: &mut Vec<T>, item: T, target: T) {
    let at = items.iter().position(|i| *i == target).unwrap_or(0);
    items.retain(|i| *i != item);
    items.insert(at.min(items.len()), item);
}

impl Settings {
    pub fn column_shown(&self, column: ListColumn) -> bool {
        self.list_columns.contains(&column)
    }

    /// Hiding `Name` is ignored. A column shown again goes last.
    pub fn set_column_shown(&mut self, column: ListColumn, shown: bool) {
        if shown == self.column_shown(column) || (!shown && !column.hideable()) {
            return;
        }
        if !shown {
            self.list_columns.retain(|c| *c != column);
            return;
        }
        self.list_columns.push(column);
    }

    /// Moves `column` to the slot `target` occupies. Both must be shown.
    pub fn move_column(&mut self, column: ListColumn, target: ListColumn) {
        if self.column_shown(column) && self.column_shown(target) {
            move_to_slot_of(&mut self.list_columns, column, target);
        }
    }

    /// The saved width of `column`, never below its minimum.
    pub fn column_width(&self, column: ListColumn) -> f32 {
        let saved = self.list_column_widths.get(&column).copied();
        saved
            .filter(|w| w.is_finite())
            .unwrap_or(column.default_width())
            .max(column.min_width())
    }

    /// Drops repeats and makes sure `Name` is present, as a hand-edited file may break either.
    fn normalize_columns(&mut self) {
        let mut seen = Vec::new();
        for c in &self.list_columns {
            if !seen.contains(c) {
                seen.push(*c);
            }
        }
        if !seen.contains(&ListColumn::Name) {
            seen.insert(0, ListColumn::Name);
        }
        self.list_columns = seen;
    }
}

/// The two flags that preceded `list_columns`.
#[derive(Deserialize)]
struct LegacyColumns {
    list_columns: Option<serde::de::IgnoredAny>,
    show_ext_column: Option<bool>,
    show_age_column: Option<bool>,
}

impl LegacyColumns {
    /// The column list the old flags describe; `None` when the file already has `list_columns` or sets no flag.
    fn migrate(self) -> Option<Vec<ListColumn>> {
        if self.list_columns.is_some()
            || (self.show_ext_column, self.show_age_column) == (None, None)
        {
            return None;
        }
        let mut columns = vec![ListColumn::Name];
        let optional = [
            (ListColumn::Ext, self.show_ext_column),
            (ListColumn::Size, Some(true)),
            (ListColumn::Modified, Some(true)),
            (ListColumn::Age, self.show_age_column),
        ];
        columns.extend(
            optional
                .into_iter()
                .filter_map(|(c, shown)| shown.unwrap_or(true).then_some(c)),
        );
        Some(columns)
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DateFormat {
    #[default]
    Locale,
    Iso,
}

/// A modifier key that changes what a drop does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DragModifier {
    Ctrl,
    Shift,
    /// Option on macOS.
    Alt,
}

impl DragModifier {
    pub const ALL: [DragModifier; 3] = [DragModifier::Ctrl, DragModifier::Shift, DragModifier::Alt];

    /// In the current UI language.
    pub fn label(self) -> &'static str {
        crate::t(match self {
            DragModifier::Ctrl => "drag_modifier.ctrl",
            DragModifier::Shift => "drag_modifier.shift",
            DragModifier::Alt => "drag_modifier.alt",
        })
    }
}

/// The platform's file manager convention: Option in Finder, Ctrl elsewhere.
impl Default for DragModifier {
    fn default() -> Self {
        if cfg!(target_os = "macos") {
            DragModifier::Alt
        } else {
            DragModifier::Ctrl
        }
    }
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

    /// In the current UI language.
    pub fn label(self) -> &'static str {
        crate::t(match self {
            ConflictChoice::Ask => "conflict_policy.ask",
            ConflictChoice::Skip => "conflict_policy.skip",
            ConflictChoice::Overwrite => "conflict_policy.overwrite",
            ConflictChoice::KeepBoth => "conflict_policy.keep_both",
            ConflictChoice::OverwriteIfNewer => "conflict_policy.overwrite_if_newer",
            ConflictChoice::SkipIdentical => "conflict_policy.skip_identical",
        })
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

/// The `[search]` table of `settings.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchSettings {
    /// Most matches a search delivers before it stops; at least `MIN_RESULT_CAP` after loading.
    pub result_cap: usize,
    /// What a glob query is matched against.
    pub glob_scope: GlobScopeChoice,
    /// Show a notice before searching from a filesystem root.
    pub warn_root: bool,
}

impl Default for SearchSettings {
    fn default() -> Self {
        Self {
            result_cap: 100_000,
            glob_scope: GlobScopeChoice::default(),
            warn_root: true,
        }
    }
}

impl SearchSettings {
    fn normalize(&mut self) {
        self.result_cap = self.result_cap.max(MIN_RESULT_CAP);
    }
}

/// Persisted form of [`GlobScope`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlobScopeChoice {
    #[default]
    Name,
    RelativePath,
}

impl GlobScopeChoice {
    pub const ALL: [GlobScopeChoice; 2] = [GlobScopeChoice::Name, GlobScopeChoice::RelativePath];

    /// In the current UI language.
    pub fn label(self) -> &'static str {
        crate::t(match self {
            GlobScopeChoice::Name => "search.scope.name",
            GlobScopeChoice::RelativePath => "search.scope.relative_path",
        })
    }
}

impl From<GlobScopeChoice> for GlobScope {
    fn from(c: GlobScopeChoice) -> Self {
        match c {
            GlobScopeChoice::Name => GlobScope::Name,
            GlobScopeChoice::RelativePath => GlobScope::RelativePath,
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
        let mut settings = Self::read(path);
        settings.clamp_font_sizes();
        settings.normalize_columns();
        settings.search.normalize();
        settings
    }

    /// Parses `path` as is; any failure yields defaults.
    fn read(path: &Path) -> Self {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                eprintln!("fx: cannot read settings {}: {err}", path.display());
                return Self::default();
            }
        };
        let mut settings: Self = toml::from_str(&text).unwrap_or_else(|err| {
            eprintln!("fx: ignoring corrupt settings {}: {err}", path.display());
            Self::default()
        });
        let legacy = toml::from_str::<LegacyColumns>(&text).ok();
        if let Some(columns) = legacy.and_then(LegacyColumns::migrate) {
            settings.list_columns = columns;
        }
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
        write_atomic(path, &text)
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

fn clamp_font_size(size: f32, default: f32) -> f32 {
    if size.is_nan() {
        default
    } else {
        size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn sidebar_groups_follow_order_and_hide() {
        let mut s = Settings::default();
        s.move_sidebar_group(PlaceGroup::Places, PlaceGroup::Drives);
        assert_eq!(
            s.sidebar_groups(),
            [
                PlaceGroup::Bookmarks,
                PlaceGroup::Drives,
                PlaceGroup::Places,
                PlaceGroup::Wsl
            ]
        );
        s.move_sidebar_group(PlaceGroup::Wsl, PlaceGroup::Bookmarks);
        s.set_sidebar_group_shown(PlaceGroup::Drives, false);
        assert_eq!(
            s.sidebar_groups(),
            [PlaceGroup::Wsl, PlaceGroup::Bookmarks, PlaceGroup::Places]
        );
        s.sidebar_order = vec![PlaceGroup::Wsl];
        assert_eq!(s.sidebar_groups().len(), 3);
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
            list_columns: vec![ListColumn::Size, ListColumn::Name, ListColumn::Age],
            tree_single_click: false,
            list_single_click: true,
            preload_subfolders: false,
            show_share_names: false,
            sidebar_order: vec![PlaceGroup::Wsl, PlaceGroup::Places],
            hidden_sidebar_groups: vec![PlaceGroup::Bookmarks],
            font_family: Some("Inter".to_string()),
            list_font_size: 16.0,
            tree_font_size: 10.5,
            folder_order: FolderOrder::Bottom,
            language: Language::Uk,
            date_format: DateFormat::Iso,
            conflict_policy: ConflictChoice::OverwriteIfNewer,
            copy_workers: 6,
            verify_copies: true,
            restore_session: false,
            drag_copy_modifier: DragModifier::Shift,
            search: SearchSettings {
                result_cap: 500,
                glob_scope: GlobScopeChoice::RelativePath,
                warn_root: false,
            },
            keymap: BTreeMap::from([
                (
                    "rename".to_owned(),
                    vec!["f2".to_owned(), "ctrl+r".to_owned()],
                ),
                ("open_settings".to_owned(), vec![]),
            ]),
            list_column_widths: BTreeMap::from([(ListColumn::Size, 120.0)]),
        };
        settings.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), settings);
        assert!(!path.with_extension("toml.tmp").exists());
    }

    #[test]
    fn columns_hide_show_and_move_but_name_stays() {
        let mut s = Settings::default();
        s.set_column_shown(ListColumn::Name, false);
        s.set_column_shown(ListColumn::Ext, false);
        s.move_column(ListColumn::Age, ListColumn::Name);
        assert_eq!(
            s.list_columns,
            [
                ListColumn::Age,
                ListColumn::Name,
                ListColumn::Size,
                ListColumn::Modified
            ]
        );
        s.set_column_shown(ListColumn::Ext, true);
        assert_eq!(s.list_columns.last(), Some(&ListColumn::Ext));
        assert_eq!(s.column_width(ListColumn::Size), 80.0);
        s.list_column_widths.insert(ListColumn::Size, 1.0);
        assert_eq!(s.column_width(ListColumn::Size), 50.0);
    }

    #[test]
    fn legacy_column_flags_become_a_column_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        write(&path, "show_ext_column = false\n");
        assert_eq!(
            Settings::load_from(&path).list_columns,
            [
                ListColumn::Name,
                ListColumn::Size,
                ListColumn::Modified,
                ListColumn::Age
            ]
        );
        write(
            &path,
            "show_age_column = false\nlist_columns = [\"size\", \"size\"]\n",
        );
        assert_eq!(
            Settings::load_from(&path).list_columns,
            [ListColumn::Name, ListColumn::Size]
        );
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
    fn search_defaults() {
        let search = Settings::default().search;
        assert_eq!(search.result_cap, 100_000);
        assert_eq!(search.glob_scope, GlobScopeChoice::Name);
        assert!(search.warn_root);
    }

    #[test]
    fn search_table_is_read_with_snake_case_scope_and_partial_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        write(&path, "[search]\nglob_scope = \"relative_path\"\n");
        let search = Settings::load_from(&path).search;
        assert_eq!(search.glob_scope, GlobScopeChoice::RelativePath);
        assert_eq!(search.result_cap, 100_000);
    }

    #[test]
    fn search_result_cap_has_a_floor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        write(&path, "[search]\nresult_cap = 3\n");
        assert_eq!(Settings::load_from(&path).search.result_cap, MIN_RESULT_CAP);
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
