//! Bookmarks and the last session as plain data, persisted as `state.toml` next to `settings.toml`.
//! A separate file keeps tab churn out of the settings file the user edits by hand.

use crate::settings::{config_dir, quarantine_corrupt, warn_no_config_dir, write_atomic};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A pinned folder. It stays in the list when the folder is missing or unmounted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    pub label: String,
    pub path: PathBuf,
}

/// The tabs of one window.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub tabs: Vec<PathBuf>,
    pub active_tab: usize,
}

/// Everything persisted besides the settings. Windows are in creation order; the first is the root window.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub bookmarks: Vec<Bookmark>,
    pub windows: Vec<WindowState>,
}

impl State {
    /// Loads from [`State::path`]. A missing or unreadable file yields the default; a corrupt one is moved to `state.toml.corrupt` first.
    pub fn load() -> Self {
        match Self::path() {
            Some(path) => Self::load_from(&path),
            None => {
                warn_no_config_dir();
                Self::default()
            }
        }
    }

    pub fn load_from(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
                quarantine_corrupt(path, &err);
                Self::default()
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                log::warn!("cannot read state {}: {err}", path.display());
                Self::default()
            }
        }
    }

    /// Atomic like [`Settings::save_to`](crate::Settings::save_to).
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
        write_atomic(path, &text)
    }

    /// `<config dir>/fx/state.toml`, or `None` when the platform has no config directory.
    pub fn path() -> Option<PathBuf> {
        config_dir().map(|dir| dir.join("fx").join("state.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn sample() -> State {
        State {
            bookmarks: vec![Bookmark {
                label: "Project".to_owned(),
                path: PathBuf::from("/home/me/src/project"),
            }],
            windows: vec![
                WindowState {
                    tabs: vec![PathBuf::from("/home/me/src"), PathBuf::from("/mnt/data")],
                    active_tab: 1,
                },
                WindowState {
                    tabs: vec![PathBuf::from("/tmp")],
                    active_tab: 0,
                },
            ],
        }
    }

    #[test]
    fn roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fx").join("state.toml");
        sample().save_to(&path).unwrap();
        assert_eq!(State::load_from(&path), sample());
    }

    #[test]
    fn missing_file_gives_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            State::load_from(&dir.path().join("state.toml")),
            State::default()
        );
    }

    #[test]
    fn corrupt_file_gives_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        fs::write(&path, "[[bookmarks]\nlabel = ").unwrap();
        assert_eq!(State::load_from(&path), State::default());
    }

    #[test]
    fn unknown_and_missing_fields_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        fs::write(
            &path,
            "future = 1\n[[windows]]\ntabs = [\"/a\"]\nzoom = 2\n[[bookmarks]]\nlabel = \"x\"\npath = \"/x\"\ncolor = \"red\"\n",
        )
        .unwrap();
        let state = State::load_from(&path);
        assert_eq!(state.bookmarks.len(), 1);
        assert_eq!(state.windows[0].tabs, [PathBuf::from("/a")]);
        assert_eq!(state.windows[0].active_tab, 0);
    }

    #[test]
    fn a_thousand_bookmarks_parse_fast() {
        let state = State {
            bookmarks: (0..1000)
                .map(|i| Bookmark {
                    label: format!("Folder {i}"),
                    path: PathBuf::from(format!("/mnt/data/projects/folder-{i}")),
                })
                .collect(),
            windows: Vec::new(),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        state.save_to(&path).unwrap();
        let start = Instant::now();
        let loaded = State::load_from(&path);
        assert!(start.elapsed() < Duration::from_millis(200));
        assert_eq!(loaded.bookmarks.len(), 1000);
    }
}
