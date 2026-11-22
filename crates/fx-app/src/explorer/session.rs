//! Restoring and persisting the session (every window's tabs) and bookmarks to `state.toml`.

use super::Explorer;
use crate::ops::NoticeKind;
use crate::state::WindowState;
use std::io;
use std::time::{Duration, Instant};

/// Session writes are at most this far apart (see [`Explorer::poll`]).
pub(super) const SESSION_SAVE_INTERVAL: Duration = Duration::from_secs(1);

impl Explorer {
    /// Opens one window per saved window that has tabs, loading only its active tab. Nothing is probed here; see `check_restored_tabs`.
    /// The first restored window reuses the initial empty one. Returns how many windows were restored.
    pub(super) fn restore_windows(&mut self, saved: Vec<WindowState>) -> usize {
        let mut restored = 0;
        for window in saved {
            if window.tabs.is_empty() {
                continue;
            }
            if restored > 0 {
                self.push_window();
            }
            let w = restored;
            restored += 1;
            for path in window.tabs {
                self.add_tab(w, path);
            }
            let tabs = &mut self.windows[w];
            tabs.active = window.active_tab.min(tabs.tabs.len() - 1);
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
