//! State of one tab's recursive search: streamed hits, progress and how the run ended.

use fx_core::{Hit, SearchQuery, UnreadableFolder, WalkControl};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchStatus {
    Running,
    Complete,
    /// The user stopped the walk; the hits found so far stay.
    Stopped,
    /// More matches exist than the configured cap; delivery stopped at the cap.
    StoppedAtCap,
    /// The root could not be read.
    Failed(String),
}

/// What a search worker reports back.
pub(crate) enum SearchEvent {
    Hits(Vec<Hit>),
    Unreadable(UnreadableFolder),
    /// Always the last event of a search.
    Done(SearchStatus),
}

/// Unreadable folders kept for listing; the count itself stays exact.
const MAX_LISTED_UNREADABLE: usize = 1_000;

#[derive(Debug)]
pub struct SearchView {
    root: PathBuf,
    text: String,
    query: SearchQuery,
    on_network: bool,
    hits: Vec<Hit>,
    unreadable: Vec<UnreadableFolder>,
    status: SearchStatus,
    started: Instant,
    elapsed: Option<Duration>,
    control: Arc<WalkControl>,
    selected: Option<usize>,
}

impl SearchView {
    pub(crate) fn new(root: PathBuf, text: String, query: SearchQuery, on_network: bool) -> Self {
        Self {
            root,
            text,
            query,
            on_network,
            hits: Vec::new(),
            unreadable: Vec::new(),
            status: SearchStatus::Running,
            started: Instant::now(),
            elapsed: None,
            control: Arc::default(),
            selected: None,
        }
    }

    pub(crate) fn control(&self) -> Arc<WalkControl> {
        Arc::clone(&self.control)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn query(&self) -> &str {
        &self.text
    }

    /// The query contains `*` or `?` and is matched as a glob.
    pub fn is_glob(&self) -> bool {
        self.query.is_glob()
    }

    /// Byte range of the matched substring in `name`, for highlighting. `None` for globs.
    pub fn match_range(&self, name: &str) -> Option<Range<usize>> {
        self.query.match_range(name)
    }

    /// The searched folder was on a network share when the search started.
    pub fn on_network(&self) -> bool {
        self.on_network
    }

    /// Matches in arrival order, unsorted.
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    /// Folder of `hit` relative to the search folder, `/`-separated with a trailing `/`; empty for the search folder itself.
    pub fn relative_folder(&self, hit: &Hit) -> String {
        let parent = hit.path.parent().unwrap_or(&self.root);
        let rel = parent.strip_prefix(&self.root).unwrap_or(parent);
        let text = rel
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        match text.is_empty() {
            true => text,
            false => text + "/",
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Selects a hit; out-of-range rows clear the selection.
    pub fn select(&mut self, row: Option<usize>) {
        self.selected = row.filter(|&r| r < self.hits.len());
    }

    /// Moves the selection by `delta` rows, starting at the first or last hit when nothing is selected.
    pub fn move_selection(&mut self, delta: isize) {
        let Some(last) = self.hits.len().checked_sub(1) else {
            return;
        };
        let start = self
            .selected
            .map_or(if delta < 0 { last as isize + 1 } else { -1 }, |r| {
                r as isize
            });
        self.selected = Some((start + delta).clamp(0, last as isize) as usize);
    }

    pub fn matches(&self) -> usize {
        self.hits.len()
    }

    pub fn status(&self) -> &SearchStatus {
        &self.status
    }

    pub fn is_running(&self) -> bool {
        self.status == SearchStatus::Running
    }

    /// Folders walked: live while running, exact once the walk has ended, also when it was stopped.
    pub fn dirs_walked(&self) -> u64 {
        self.control.dirs_read()
    }

    /// Number of folders that could not be read.
    pub fn unreadable(&self) -> u64 {
        self.control.unreadable()
    }

    /// The first unreadable folders, at most 1,000; `unreadable()` counts them all.
    pub fn unreadable_folders(&self) -> &[UnreadableFolder] {
        &self.unreadable
    }

    /// Live while running, frozen when the search ends.
    pub fn elapsed(&self) -> Duration {
        self.elapsed.unwrap_or_else(|| self.started.elapsed())
    }

    /// Stops the walk and keeps the hits. No-op once the search has ended.
    pub(crate) fn cancel(&mut self) {
        self.control.cancel();
        self.end(SearchStatus::Stopped);
    }

    /// Returns true if the view changed.
    pub(crate) fn apply(&mut self, event: SearchEvent) -> bool {
        match event {
            SearchEvent::Hits(hits) => self.append(hits),
            SearchEvent::Unreadable(folder) => self.add_unreadable(folder),
            SearchEvent::Done(status) => self.end(status),
        }
    }

    fn append(&mut self, hits: Vec<Hit>) -> bool {
        if !self.is_running() || hits.is_empty() {
            return false;
        }
        self.hits.extend(hits);
        true
    }

    fn add_unreadable(&mut self, folder: UnreadableFolder) -> bool {
        if !self.is_running() || self.unreadable.len() >= MAX_LISTED_UNREADABLE {
            return false;
        }
        self.unreadable.push(folder);
        true
    }

    /// The first ending wins: a stopped search keeps `Stopped` when the worker's `Done` arrives later.
    fn end(&mut self, status: SearchStatus) -> bool {
        if !self.is_running() {
            return false;
        }
        self.status = status;
        self.elapsed = Some(self.started.elapsed());
        true
    }
}

impl Drop for SearchView {
    fn drop(&mut self) {
        self.control.cancel();
    }
}
