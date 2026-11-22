//! Recursive search of the active tab: the worker thread, its cancellation and the root-folder notice.

use super::Explorer;
use crate::search::{SearchEvent, SearchView};
use fx_core::{ListError, SearchQuery, WalkOptions, WalkProgress, Walked, search};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time::Instant;

pub(super) struct SearchMsg {
    tab_id: u64,
    generation: u64,
    event: SearchEvent,
}

fn log_search_end(root: &Path, walked: &Result<Walked, ListError>, started: Instant) {
    match walked {
        Ok(w) => log::info!(
            "search in {} walked {} dirs, {} unreadable, in {:?}",
            root.display(),
            w.stats.dirs,
            w.stats.errors,
            started.elapsed()
        ),
        Err(err) => log::warn!("search root {} failed: {err}", root.display()),
    }
}

impl Explorer {
    /// Searches below the active tab's folder, replacing any earlier search of that tab.
    /// Blank text clears the search instead, because an empty query would match every entry.
    pub fn start_search(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return self.clear_search();
        }
        let search = &self.settings.search;
        let query = SearchQuery::new(text, search.glob_scope.into());
        let cap = search.result_cap;
        let tab = self.active_mut();
        let tab_id = tab.id();
        let (generation, view) = tab.begin_search(text.to_owned(), cap);
        let (root, cancel, progress) = (
            view.root().to_path_buf(),
            view.cancel_flag(),
            view.progress(),
        );
        self.spawn_search(tab_id, generation, root, query, (cancel, progress));
    }

    /// Stops the active tab's walk. The hits found so far stay.
    pub fn cancel_search(&mut self) {
        self.active_mut().cancel_search();
    }

    /// Stops the active tab's walk and drops its results.
    pub fn clear_search(&mut self) {
        self.active_mut().clear_search();
    }

    /// The active tab's search, if it has one.
    pub fn search(&self) -> Option<&SearchView> {
        self.active().search()
    }

    /// Selects result `row` of the active tab's search.
    pub fn select_search_hit(&mut self, row: Option<usize>) {
        if let Some(search) = self.active_mut().search_mut() {
            search.select(row);
        }
    }

    /// Moves the selected result up (`-1`) or down (`1`).
    pub fn move_search_selection(&mut self, delta: isize) {
        if let Some(search) = self.active_mut().search_mut() {
            search.move_selection(delta);
        }
    }

    /// Enters the folder or opens the file of result `row`. Entering a folder ends the search.
    pub fn open_search_hit(&mut self, row: usize) {
        if let Some(path) = self.search_hit_path(row) {
            self.open_path(path);
        }
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
        tab.clear_search();
        tab.navigate_to(parent);
        tab.select_after_load(name);
        self.start_load(w, index);
    }

    fn search_hit_path(&self, row: usize) -> Option<PathBuf> {
        Some(self.search()?.hits().get(row)?.path.clone())
    }

    /// True when the active tab sits at a filesystem root and the user has not turned the notice off.
    pub fn search_needs_root_notice(&self) -> bool {
        self.settings.search.warn_root && self.active().path().parent().is_none()
    }

    /// "Don't show again": turns `search.warn_root` off and persists it.
    pub fn disable_search_root_notice(&mut self) {
        let mut settings = self.settings.clone();
        settings.search.warn_root = false;
        self.set_settings(settings);
    }

    fn spawn_search(
        &self,
        tab_id: u64,
        generation: u64,
        root: PathBuf,
        query: SearchQuery,
        (cancel, progress): (Arc<AtomicBool>, Arc<WalkProgress>),
    ) {
        let tx = self.search_tx.clone();
        let wake = Arc::clone(&self.wake);
        thread::spawn(move || {
            let send = |event| {
                // A send only fails once the Explorer is gone, and then nobody needs the result.
                let _ = tx.send(SearchMsg {
                    tab_id,
                    generation,
                    event,
                });
                wake();
            };
            let opts = WalkOptions::default();
            let started = Instant::now();
            let walked = search(&root, &query, &opts, &cancel, &progress, |hits| {
                send(SearchEvent::Hits(hits))
            });
            log_search_end(&root, &walked, started);
            send(SearchEvent::Done(
                walked.map(|w| w.stats).map_err(|e| e.to_string()),
            ));
        });
    }

    /// Applies search batches to their tabs in any window; stale and orphaned ones are dropped.
    /// Returns true if any search changed.
    pub(super) fn apply_searches(&mut self) -> bool {
        let show_hidden = self.settings.show_hidden;
        let mut changed = false;
        while let Ok(msg) = self.search_rx.try_recv() {
            let tab = self
                .windows
                .iter_mut()
                .flat_map(|w| &mut w.tabs)
                .find(|tab| tab.id() == msg.tab_id);
            if let Some(tab) = tab {
                changed |= tab.apply_search(msg.generation, msg.event, show_hidden);
            }
        }
        changed
    }
}
