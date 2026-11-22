//! Restoring and persisting the session (every window's tabs) and bookmarks to `state.toml`.

use super::Explorer;
use crate::ops::NoticeKind;
use crate::state::WindowState;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Session writes are at most this far apart (see [`Explorer::poll`]).
pub(super) const SESSION_SAVE_INTERVAL: Duration = Duration::from_secs(1);

impl Explorer {
    /// Opens one window per saved window that still has a directory among its tabs, loading only its active tab.
    /// The first restored window reuses the initial empty one. Returns how many windows were restored.
    pub(super) fn restore_windows(&mut self, saved: Vec<WindowState>) -> usize {
        let mut restored = 0;
        for window in saved {
            let saved_tabs = window.tabs.len();
            let (tabs, active) = surviving_tabs(window);
            if tabs.len() < saved_tabs {
                log::info!(
                    "dropped {} saved tab(s) whose folder is gone",
                    saved_tabs - tabs.len()
                );
            }
            if tabs.is_empty() {
                continue;
            }
            if restored > 0 {
                self.push_window();
            }
            let w = restored;
            restored += 1;
            for path in tabs {
                self.add_tab(w, path);
            }
            let window = &mut self.windows[w];
            window.active = active.min(window.tabs.len() - 1);
            self.ensure_loaded(w);
        }
        restored
    }

    /// Writes the session immediately instead of waiting for the next [`Explorer::poll`]. Call on exit.
    pub fn save_session(&mut self) {
        self.save_state();
    }

    /// Writes bookmarks and, if enabled, every window's tabs to `state.toml`. A failure becomes an error notice.
    pub(super) fn save_state(&mut self) {
        self.session_dirty = false;
        self.next_session_save = Instant::now() + SESSION_SAVE_INTERVAL;
        self.state.windows = if self.settings.restore_session {
            self.windows
                .iter()
                .map(|w| WindowState {
                    tabs: w.tabs.iter().map(|t| t.path().to_path_buf()).collect(),
                    active_tab: w.active,
                })
                .collect()
        } else {
            Vec::new()
        };
        if let Some(path) = &self.state_path {
            let saved = self.state.save_to(path);
            self.report_save("state", saved);
        }
    }

    /// Turns the first failed save of `what` into an error notice; repeats are only logged at debug until a save succeeds.
    pub(super) fn report_save(&mut self, what: &'static str, saved: io::Result<()>) {
        let Err(err) = saved else {
            if self.failing_saves.remove(what) {
                log::info!("saving {what} works again");
            } else {
                log::debug!("saved {what}");
            }
            return;
        };
        if !self.failing_saves.insert(what) {
            log::debug!("saving {what} still fails: {err}");
            return;
        }
        log::warn!("cannot save {what}: {err}");
        self.notify(
            NoticeKind::Error,
            crate::tr("notice.save_failed", &[("error", &err.to_string())]),
        );
    }
}

/// The saved tabs that are still directories and the index of the active one among them.
/// The saved active tab may be gone; the nearest surviving one at or after it takes over.
fn surviving_tabs(window: WindowState) -> (Vec<PathBuf>, usize) {
    let before = window.tabs.len().min(window.active_tab);
    let active = window.tabs[..before].iter().filter(|p| p.is_dir()).count();
    let tabs = window.tabs.into_iter().filter(|p| p.is_dir()).collect();
    (tabs, active)
}
