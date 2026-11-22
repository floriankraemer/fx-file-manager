//! Tabs of the current window: opening, closing, switching, history navigation and row lookups.

use super::chooser::open_with_chooser;
use super::{Explorer, absolute, open_with_system};
use crate::ops::{EntryDetails, details};
use crate::tab::{LoadState, Tab};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

impl Explorer {
    pub fn tabs(&self) -> &[Tab] {
        &self.win().tabs
    }

    pub fn active_index(&self) -> usize {
        self.win().active
    }

    pub fn active(&self) -> &Tab {
        let w = self.win();
        &w.tabs[w.active]
    }

    pub fn active_mut(&mut self) -> &mut Tab {
        let w = self.win_mut();
        &mut w.tabs[w.active]
    }

    /// Out-of-range indices are ignored.
    pub fn set_active(&mut self, index: usize) {
        let w = self.win_mut();
        if index >= w.tabs.len() {
            return;
        }
        w.active = index;
        self.session_dirty = true;
        self.ensure_loaded(self.current);
    }

    /// Starts the first load of window `w`'s active tab if it was restored without one.
    pub(super) fn ensure_loaded(&mut self, w: usize) {
        let window = &self.windows[w];
        if window.tabs[window.active].generation() == 0 {
            self.start_load(w, window.active);
        }
    }

    /// Opens a tab at `path`, makes it active and starts loading it.
    pub fn new_tab(&mut self, path: PathBuf) {
        self.push_tab(self.current, path);
    }

    /// Appends an active tab to window `w` and starts loading it.
    pub(super) fn push_tab(&mut self, w: usize, path: PathBuf) {
        let index = self.add_tab(w, path);
        self.start_load(w, index);
    }

    /// Appends an active tab to window `w` without loading it. Its generation stays 0 until the first load.
    pub(super) fn add_tab(&mut self, w: usize, path: PathBuf) -> usize {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let tab = Tab::new(
            id,
            absolute(path),
            self.settings.show_hidden,
            self.settings.folder_order,
        );
        let window = &mut self.windows[w];
        window.tabs.push(tab);
        window.active = window.tabs.len() - 1;
        window.active
    }

    /// The last remaining tab is never closed. Out-of-range indices are ignored.
    pub fn close_tab(&mut self, index: usize) {
        let w = self.win_mut();
        if w.tabs.len() <= 1 || index >= w.tabs.len() {
            return;
        }
        w.tabs.remove(index);
        if index < w.active {
            w.active -= 1;
        } else {
            w.active = w.active.min(w.tabs.len() - 1);
        }
        self.session_dirty = true;
        self.ensure_loaded(self.current);
    }

    /// Navigates the active tab, recording history.
    pub fn navigate(&mut self, path: PathBuf) {
        let index = self.win().active;
        self.win_mut().tabs[index].navigate_to(absolute(path));
        self.start_load(self.current, index);
    }

    pub fn back(&mut self) {
        let index = self.win().active;
        if self.win_mut().tabs[index].go_back() {
            self.start_load(self.current, index);
        }
    }

    pub fn forward(&mut self) {
        let index = self.win().active;
        if self.win_mut().tabs[index].go_forward() {
            self.start_load(self.current, index);
        }
    }

    pub fn up(&mut self) {
        let Some(parent) = self.active().path().parent().map(Path::to_path_buf) else {
            return;
        };
        self.navigate(parent);
    }

    pub fn refresh(&mut self) {
        let index = self.win().active;
        self.start_load(self.current, index);
    }

    /// Enters a directory (or a symlink to one); opens anything else with the system handler.
    /// Does nothing while the tab is not loaded.
    pub fn activate(&mut self, row: usize) {
        let tab = self.active();
        let LoadState::Loaded(view) = tab.state() else {
            return;
        };
        let Some(&index) = tab.order().get(row) else {
            return;
        };
        let path = tab
            .path()
            .join(view.listing.name(&view.listing.entries[index as usize]));
        self.open_path(path);
    }

    /// Enters `path` when it is a directory (or a symlink to one); opens anything else with the system handler.
    pub(super) fn open_path(&mut self, path: PathBuf) {
        if fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            self.navigate(path);
        } else if let Err(err) = open_with_system(&path) {
            eprintln!("fx: cannot open {}: {err}", path.display());
        }
    }

    /// Lets the user pick the application for the file at `row`; shows a notice when the system has no chooser.
    pub fn open_with(&self, row: usize) {
        let Some(path) = self.entry_path(row) else {
            return;
        };
        let tx = self.error_tx.clone();
        let wake = Arc::clone(&self.wake);
        open_with_chooser(&path, move || {
            let _ = tx.send(crate::t("notice.open_with_unavailable").to_owned());
            wake();
        });
    }

    /// Shows the errors background helpers reported. Returns true if there were any.
    pub(super) fn apply_helper_errors(&mut self) -> bool {
        let mut changed = false;
        while let Ok(text) = self.error_rx.try_recv() {
            self.notify_error(text);
            changed = true;
        }
        changed
    }

    /// Opens the folder at `row` in a new tab. Non-folders are ignored.
    pub fn open_in_new_tab(&mut self, row: usize) {
        let Some(path) = self.entry_path(row) else {
            return;
        };
        if fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            self.new_tab(path);
        }
    }

    /// Full path of a visible row of the active tab.
    pub fn entry_path(&self, row: usize) -> Option<PathBuf> {
        let tab = self.active();
        Some(tab.path().join(tab.name_at(row)?))
    }

    /// Properties for a visible row of the active tab. Reads metadata now, on the calling thread; `None` if the row or entry is gone.
    pub fn entry_details(&self, row: usize) -> Option<EntryDetails> {
        details(&self.entry_path(row)?)
    }
}
