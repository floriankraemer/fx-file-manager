//! Recursive search of the active tab: the worker thread, its cancellation and the root-folder notice.

use super::Explorer;
use crate::search::{SearchEvent, SearchView};
use fx_core::{SearchQuery, WalkOptions, search};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread;

pub(super) struct SearchMsg {
    tab_id: u64,
    generation: u64,
    event: SearchEvent,
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
        let (root, cancel) = (view.root().to_path_buf(), view.cancel_flag());
        self.spawn_search(tab_id, generation, root, query, cancel);
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
        cancel: Arc<AtomicBool>,
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
            let walked = search(&root, &query, &WalkOptions::default(), &cancel, |hits| {
                send(SearchEvent::Hits(hits))
            });
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
