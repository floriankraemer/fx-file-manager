//! State of one tab's recursive search: streamed hits, progress and how the run ended.

use fx_core::{Hit, WalkControl, WalkStats};
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

/// What a search worker reports back, tagged with the generation of the search that produced it.
pub(crate) enum SearchEvent {
    Hits(Vec<Hit>),
    Done(Result<WalkStats, String>),
}

#[derive(Debug)]
pub struct SearchView {
    root: PathBuf,
    query: String,
    cap: usize,
    hits: Vec<Hit>,
    status: SearchStatus,
    started: Instant,
    elapsed: Option<Duration>,
    control: Arc<WalkControl>,
    selected: Option<usize>,
}

impl SearchView {
    pub(crate) fn new(root: PathBuf, query: String, cap: usize) -> Self {
        Self {
            root,
            query,
            cap,
            hits: Vec::new(),
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
        &self.query
    }

    /// Matches in arrival order, unsorted. Never longer than the cap.
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

    /// Folders walked: live while running, exact once the walk has ended.
    pub fn dirs_walked(&self) -> u64 {
        self.control.dirs_read()
    }

    /// Folders that could not be read: live while running, exact once the walk has ended.
    pub fn unreadable(&self) -> u64 {
        self.control.unreadable()
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
    pub(crate) fn apply(&mut self, event: SearchEvent, show_hidden: bool) -> bool {
        match event {
            SearchEvent::Hits(hits) => self.append(hits, show_hidden),
            SearchEvent::Done(result) => self.finish(result),
        }
    }

    fn append(&mut self, hits: Vec<Hit>, show_hidden: bool) -> bool {
        if !self.is_running() {
            return false;
        }
        let before = self.hits.len();
        for hit in hits {
            if !show_hidden && hit.hidden {
                continue;
            }
            if self.hits.len() >= self.cap {
                self.control.cancel();
                self.end(SearchStatus::StoppedAtCap);
                return true;
            }
            self.hits.push(hit);
        }
        self.hits.len() != before
    }

    fn finish(&mut self, result: Result<WalkStats, String>) -> bool {
        match result {
            Ok(_) => self.end(SearchStatus::Complete),
            Err(message) => self.end(SearchStatus::Failed(message)),
        }
        true
    }

    fn end(&mut self, status: SearchStatus) {
        if !self.is_running() {
            return;
        }
        self.status = status;
        self.elapsed = Some(self.started.elapsed());
    }
}

impl Drop for SearchView {
    fn drop(&mut self) {
        self.control.cancel();
    }
}
