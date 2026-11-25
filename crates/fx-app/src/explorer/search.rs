//! Search use cases: starting, restarting and ending the search of a tab, opening its results, and the root-folder notice.

use super::Explorer;
use super::open::OpenTarget;
use crate::search::{SearchRequest, SearchView};
use crate::settings::Settings;
use fx_core::{SearchOptions, SearchQuery};
use std::path::PathBuf;

/// Whether `request_search` began the search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchStart {
    Started,
    /// The search is on hold until `acknowledge_root_notice` is called.
    NeedsRootNotice,
}

impl Explorer {
    /// Searches below the active tab's folder, replacing any earlier search of that tab.
    /// Blank text clears the search instead, because an empty query would match every entry.
    /// A first search from a filesystem root waits for `acknowledge_root_notice`, once per session.
    pub fn request_search(&mut self, text: &str) -> SearchStart {
        let text = text.trim();
        if text.is_empty() {
            self.clear_search();
            return SearchStart::Started;
        }
        if self.search().is_none() && self.root_notice_due() {
            self.active_mut().search_slot_mut().hold(text.to_owned());
            return SearchStart::NeedsRootNotice;
        }
        self.start_search(text);
        SearchStart::Started
    }

    /// Closes the root notice and runs the search it held back in the active tab.
    /// With `dont_show_again`, `search.warn_root` is turned off and persisted.
    pub fn acknowledge_root_notice(&mut self, dont_show_again: bool) {
        self.root_notice_seen = true;
        if dont_show_again {
            let mut settings = self.settings.clone();
            settings.search.warn_root = false;
            self.set_settings(settings);
        }
        if let Some(text) = self.active_mut().search_slot_mut().take_held() {
            self.start_search(&text);
        }
    }

    fn root_notice_due(&self) -> bool {
        self.settings.search.warn_root
            && !self.root_notice_seen
            && self.active().path().parent().is_none()
    }

    pub(super) fn start_search(&mut self, text: &str) {
        let (w, index) = (self.current, self.win().active);
        self.begin_search(w, index, text);
    }

    /// Replaces the search of tab `index` of window `w` and starts its worker.
    fn begin_search(&mut self, w: usize, index: usize, text: &str) {
        let search = &self.settings.search;
        let query = SearchQuery::new(
            text,
            SearchOptions {
                scope: search.glob_scope.into(),
                case_sensitive: search.case_sensitive,
            },
        );
        let (cap, show_hidden) = (search.result_cap, self.settings.show_hidden);
        let tab = &mut self.windows[w].tabs[index];
        let (root, tab_id) = (tab.path().to_path_buf(), tab.id());
        let view = SearchView::new(
            root.clone(),
            text.to_owned(),
            query.clone(),
            tab.on_network(),
        );
        let control = view.control();
        let generation = tab.search_slot_mut().begin(view);
        self.searches.spawn(SearchRequest {
            tab_id,
            generation,
            root,
            query,
            show_hidden,
            cap,
            control,
        });
    }

    /// Whether `new` changes what a search finds, so running ones must start over.
    pub(super) fn search_rules_changed(&self, new: &Settings) -> bool {
        let (old, new_search) = (&self.settings.search, &new.search);
        new.show_hidden != self.settings.show_hidden
            || new_search.case_sensitive != old.case_sensitive
            || new_search.glob_scope != old.glob_scope
            || new_search.result_cap != old.result_cap
    }

    /// Restarts every tab's search, in all windows, under the current settings.
    pub(super) fn restart_searches(&mut self) {
        let running: Vec<_> = (0..self.windows.len())
            .flat_map(|w| {
                self.windows[w]
                    .tabs
                    .iter()
                    .enumerate()
                    .map(move |(i, t)| (w, i, t))
            })
            .filter_map(|(w, i, tab)| Some((w, i, tab.search()?.query().to_owned())))
            .collect();
        for (w, index, text) in running {
            self.begin_search(w, index, &text);
        }
    }

    /// Flips `search.case_sensitive`, persists it and restarts the searches.
    pub fn toggle_search_case(&mut self) {
        let mut settings = self.settings.clone();
        settings.search.case_sensitive = !settings.search.case_sensitive;
        self.set_settings(settings);
    }

    /// Stops the active tab's walk. The hits found so far stay.
    pub fn cancel_search(&mut self) {
        self.active_mut().search_slot_mut().cancel();
    }

    /// Stops the active tab's walk and drops its results.
    pub fn clear_search(&mut self) {
        self.active_mut().search_slot_mut().clear();
    }

    /// Esc: a running search stops and keeps its results; otherwise the search is dropped.
    /// Returns true if a running search was stopped, false if the caller should close its search bar.
    pub fn escape_search(&mut self) -> bool {
        if self.search().is_some_and(SearchView::is_running) {
            self.cancel_search();
            return true;
        }
        self.clear_search();
        false
    }

    /// The active tab's search, if it has one.
    pub fn search(&self) -> Option<&SearchView> {
        self.active().search()
    }

    /// Selects result `row` of the active tab's search.
    pub fn select_search_hit(&mut self, row: Option<usize>) {
        if let Some(search) = self.active_mut().search_slot_mut().view_mut() {
            search.select(row);
        }
    }

    /// Moves the selected result up (`-1`) or down (`1`).
    pub fn move_search_selection(&mut self, delta: isize) {
        if let Some(search) = self.active_mut().search_slot_mut().view_mut() {
            search.move_selection(delta);
        }
    }

    /// Enters the folder or opens the file of result `row`. Entering a folder ends the search.
    pub fn open_search_hit(&mut self, row: usize) {
        let Some(hit) = self.search().and_then(|search| search.hits().get(row)) else {
            return;
        };
        let (path, kind) = (hit.path.clone(), hit.kind);
        self.open_path(path, kind, OpenTarget::ThisTab);
    }

    /// Goes to the folder that holds result `row`, selects the item there and ends the search.
    pub fn reveal_search_hit(&mut self, row: usize) {
        let Some(path) = self.search_hit_path(row) else {
            return;
        };
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return;
        };
        let (parent, name) = (parent.to_path_buf(), name.to_os_string());
        let (w, index) = (self.current, self.win().active);
        let tab = &mut self.win_mut().tabs[index];
        tab.search_slot_mut().clear();
        tab.navigate_to(parent);
        tab.select_after_load(name);
        self.start_load(w, index);
    }

    fn search_hit_path(&self, row: usize) -> Option<PathBuf> {
        Some(self.search()?.hits().get(row)?.path.clone())
    }

    /// Applies search events to their tabs in any window; stale and orphaned ones are dropped.
    /// Bounded per call, and wakes the UI again when events are left over. Returns true if any search changed.
    pub(super) fn apply_searches(&mut self) -> bool {
        self.searches.drain(|msg| {
            let tab = self
                .windows
                .iter_mut()
                .flat_map(|w| &mut w.tabs)
                .find(|tab| tab.id() == msg.tab_id);
            tab.is_some_and(|tab| tab.search_slot_mut().apply(msg.generation, msg.event))
        })
    }
}
