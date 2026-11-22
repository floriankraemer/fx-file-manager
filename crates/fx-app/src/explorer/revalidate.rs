//! Picking up external changes: periodic mtime checks of each window's active folder and explicit revalidation.

use super::loading::dir_mtime;
use super::{Explorer, Window, WindowId};
use crate::tab::LoadState;
use crate::view::needs_relist;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

/// How often the open folder's mtime is checked for external changes (see [`Explorer::poll`]).
pub const REVALIDATE_INTERVAL: Duration = Duration::from_secs(2);

/// Outcome of the background mtime check of a tab's folder.
pub(super) struct StatChecked {
    window: WindowId,
    tab_id: u64,
    generation: u64,
    stale: bool,
}

impl Explorer {
    /// Re-lists the active tab's folder without clearing the view, so there is no loading flash and the selection stays.
    /// The result arrives through [`Explorer::poll`]; navigating meanwhile discards it. Does nothing while the tab is loading.
    /// Poll-based detection only sees directory mtime changes (files added/removed/renamed); call this on window focus
    /// to also pick up size/content changes of existing files.
    // ponytail: mtime polling, not a watcher; a `notify` hint calling this method is the upgrade path
    pub fn revalidate(&mut self) {
        let tab = self.active();
        if matches!(tab.state(), LoadState::Loading) {
            return;
        }
        self.spawn_load(tab.id(), tab.generation(), tab.path().to_path_buf());
    }

    /// Every `REVALIDATE_INTERVAL`, stats window `w`'s active folder on a worker thread and re-lists it if it changed.
    pub(super) fn tick_revalidate(&mut self, w: usize) {
        let window = &mut self.windows[w];
        let now = Instant::now();
        if now < window.next_tick {
            return;
        }
        window.next_tick = now + REVALIDATE_INTERVAL;
        if window.stat_in_flight {
            return;
        }
        let Some(check) = StatCheck::of(window) else {
            return;
        };
        window.stat_in_flight = true;
        let tx = self.stat_tx.clone();
        let wake = Arc::clone(&self.wake);
        thread::spawn(move || {
            let checked = check.run();
            let stale = checked.stale;
            let _ = tx.send(checked);
            // Only a stale folder needs a repaint; otherwise the next idle tick clears the in-flight flag.
            if stale {
                wake();
            }
        });
    }

    /// Re-lists folders whose mtime check found them stale.
    pub(super) fn apply_stat_checks(&mut self) {
        while let Ok(checked) = self.stat_rx.try_recv() {
            if let Some(window) = self.windows.iter_mut().find(|w| w.id == checked.window) {
                window.stat_in_flight = false;
            }
            if checked.stale {
                self.relist_if_current(&checked);
            }
        }
    }

    /// A navigation or reload since the check bumped the generation and made it moot.
    fn relist_if_current(&self, checked: &StatChecked) {
        let Some(tab) = self
            .windows
            .iter()
            .flat_map(|w| &w.tabs)
            .find(|tab| tab.id() == checked.tab_id)
        else {
            return;
        };
        if tab.generation() == checked.generation && !matches!(tab.state(), LoadState::Loading) {
            self.spawn_load(tab.id(), tab.generation(), tab.path().to_path_buf());
        }
    }
}

/// What the worker thread needs to decide whether a window's active folder changed.
struct StatCheck {
    window: WindowId,
    tab_id: u64,
    generation: u64,
    path: PathBuf,
    mtime: Option<SystemTime>,
    listed_at: SystemTime,
}

impl StatCheck {
    fn of(window: &Window) -> Option<Self> {
        let tab = &window.tabs[window.active];
        let LoadState::Loaded(view) = tab.state() else {
            return None;
        };
        Some(Self {
            window: window.id,
            tab_id: tab.id(),
            generation: tab.generation(),
            path: tab.path().to_path_buf(),
            mtime: view.mtime,
            listed_at: view.listed_at,
        })
    }

    fn run(self) -> StatChecked {
        StatChecked {
            window: self.window,
            tab_id: self.tab_id,
            generation: self.generation,
            stale: needs_relist(self.mtime, self.listed_at, dir_mtime(&self.path)),
        }
    }
}
