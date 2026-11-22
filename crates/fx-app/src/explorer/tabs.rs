//! Tabs of the current window: opening, closing, switching, and history navigation.

use super::{Explorer, absolute};
use crate::tab::Tab;
use std::path::{Path, PathBuf};

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
}
