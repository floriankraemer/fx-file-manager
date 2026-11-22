//! State of one tab's recursive search: streamed hits, progress and how the run ended.

use fx_core::{Hit, WalkStats};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
    dirs_walked: u64,
    unreadable: u64,
    started: Instant,
    elapsed: Option<Duration>,
    cancel: Arc<AtomicBool>,
}

impl SearchView {
    pub(crate) fn new(root: PathBuf, query: String, cap: usize) -> Self {
        Self {
            root,
            query,
            cap,
            hits: Vec::new(),
            status: SearchStatus::Running,
            dirs_walked: 0,
            unreadable: 0,
            started: Instant::now(),
            elapsed: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
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

    pub fn matches(&self) -> usize {
        self.hits.len()
    }

    pub fn status(&self) -> &SearchStatus {
        &self.status
    }

    pub fn is_running(&self) -> bool {
        self.status == SearchStatus::Running
    }

    /// Folders walked; known once the walk has ended, `0` before.
    pub fn dirs_walked(&self) -> u64 {
        self.dirs_walked
    }

    /// Folders that could not be read; known once the walk has ended, `0` before.
    pub fn unreadable(&self) -> u64 {
        self.unreadable
    }

    /// Live while running, frozen when the search ends.
    pub fn elapsed(&self) -> Duration {
        self.elapsed.unwrap_or_else(|| self.started.elapsed())
    }

    /// Stops the walk and keeps the hits. No-op once the search has ended.
    pub(crate) fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
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
            if !show_hidden && is_hidden(&self.root, &hit.path) {
                continue;
            }
            if self.hits.len() >= self.cap {
                self.cancel.store(true, Ordering::Relaxed);
                self.end(SearchStatus::StoppedAtCap);
                return true;
            }
            self.hits.push(hit);
        }
        self.hits.len() != before
    }

    fn finish(&mut self, result: Result<WalkStats, String>) -> bool {
        match result {
            Ok(stats) => {
                self.dirs_walked = (stats.dirs + 1).saturating_sub(stats.errors);
                self.unreadable = stats.errors;
                self.end(SearchStatus::Complete);
            }
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
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Any component below `root` starts with a dot.
fn is_hidden(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|rel| {
        rel.components()
            .any(|c| c.as_os_str().as_encoded_bytes().first() == Some(&b'.'))
    })
}
