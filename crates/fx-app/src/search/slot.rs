use super::{SearchEvent, SearchView};
use std::path::Path;

/// A tab's search and the generation its results must carry; older results are stale and dropped.
#[derive(Debug, Default)]
pub(crate) struct SearchSlot {
    generation: u64,
    view: Option<SearchView>,
}

impl SearchSlot {
    pub(crate) fn view(&self) -> Option<&SearchView> {
        self.view.as_ref()
    }

    pub(crate) fn view_mut(&mut self) -> Option<&mut SearchView> {
        self.view.as_mut()
    }

    /// Replaces any search with `view` and returns the generation its results must carry.
    /// Dropping the old view cancels its walk.
    pub(crate) fn begin(&mut self, view: SearchView) -> u64 {
        self.generation += 1;
        self.view = Some(view);
        self.generation
    }

    pub(crate) fn cancel(&mut self) {
        if let Some(view) = &mut self.view {
            view.cancel();
        }
    }

    /// Drops the search and its results, and makes any in-flight event stale.
    pub(crate) fn clear(&mut self) {
        self.generation += 1;
        self.view = None;
    }

    /// A search is rooted where the tab was when it started; once the tab is elsewhere it goes away.
    pub(crate) fn clear_if_moved(&mut self, tab_path: &Path) {
        if self.view.as_ref().is_some_and(|v| v.root() != tab_path) {
            self.clear();
        }
    }

    /// Returns true if the event belonged to the current search and changed it.
    pub(crate) fn apply(&mut self, generation: u64, event: SearchEvent) -> bool {
        if generation != self.generation {
            return false;
        }
        self.view.as_mut().is_some_and(|view| view.apply(event))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::SearchStatus;
    use fx_core::{SearchOptions, SearchQuery};
    use std::path::PathBuf;

    fn view(root: &str) -> SearchView {
        let query = SearchQuery::new("x", SearchOptions::default());
        SearchView::new(PathBuf::from(root), "x".to_owned(), query, false)
    }

    #[test]
    fn events_of_an_older_generation_are_dropped() {
        let mut slot = SearchSlot::default();
        let old = slot.begin(view("/r"));
        let new = slot.begin(view("/r"));
        let done = || SearchEvent::Done(SearchStatus::Complete);
        assert!(!slot.apply(old, done()));
        assert!(slot.view().unwrap().is_running());
        assert!(slot.apply(new, done()));
    }

    #[test]
    fn clearing_makes_in_flight_events_stale_and_cancels_the_walk() {
        let mut slot = SearchSlot::default();
        let generation = slot.begin(view("/r"));
        let control = slot.view().unwrap().control();
        slot.clear();
        assert!(control.is_cancelled());
        assert!(!slot.apply(generation, SearchEvent::Hits(Vec::new())));
    }

    #[test]
    fn a_search_survives_only_while_the_tab_stays_at_its_root() {
        let mut slot = SearchSlot::default();
        slot.begin(view("/r"));
        slot.clear_if_moved(Path::new("/r"));
        assert!(slot.view().is_some());
        slot.clear_if_moved(Path::new("/r/sub"));
        assert!(slot.view().is_none());
    }
}
