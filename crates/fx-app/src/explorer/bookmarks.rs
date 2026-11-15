//! Bookmarked folders and their background availability checks.

use super::{Explorer, absolute};
use crate::state::Bookmark;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

/// Result of the background `is_dir` check of one bookmark.
pub(super) struct BookmarkChecked {
    path: PathBuf,
    available: bool,
}

impl Explorer {
    pub fn bookmarks(&self) -> &[Bookmark] {
        &self.state.bookmarks
    }

    pub fn is_bookmarked(&self, path: &Path) -> bool {
        self.state.bookmarks.iter().any(|b| b.path == path)
    }

    /// Bookmarks `path`, labelled with its folder name (the full path for a root). Already bookmarked is a no-op.
    pub fn add_bookmark(&mut self, path: PathBuf) {
        let path = absolute(path);
        if self.is_bookmarked(&path) {
            return;
        }
        let label = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        self.state.bookmarks.push(Bookmark {
            label,
            path: path.clone(),
        });
        self.save_state();
        self.spawn_bookmark_checks(vec![path]);
    }

    /// Out-of-range indices are ignored.
    pub fn remove_bookmark(&mut self, index: usize) {
        if index >= self.state.bookmarks.len() {
            return;
        }
        let removed = self.state.bookmarks.remove(index);
        self.availability.remove(&removed.path);
        self.save_state();
    }

    /// The label is trimmed; an empty one is ignored.
    pub fn rename_bookmark(&mut self, index: usize, label: &str) {
        let label = label.trim();
        if label.is_empty() {
            return;
        }
        if let Some(bookmark) = self.state.bookmarks.get_mut(index) {
            bookmark.label = label.to_owned();
            self.save_state();
        }
    }

    /// Moves the bookmark at `from` so it ends up at index `to`. Out-of-range indices are ignored.
    pub fn move_bookmark(&mut self, from: usize, to: usize) {
        let len = self.state.bookmarks.len();
        if from >= len || to >= len || from == to {
            return;
        }
        let bookmark = self.state.bookmarks.remove(from);
        self.state.bookmarks.insert(to, bookmark);
        self.save_state();
    }

    /// Bookmarks the active tab's folder, or removes that bookmark if it exists.
    pub fn toggle_bookmark_active(&mut self) {
        self.toggle_bookmark(self.active().path().to_path_buf());
    }

    /// Bookmarks `path`, or removes that bookmark if it exists.
    pub fn toggle_bookmark(&mut self, path: PathBuf) {
        let path = absolute(path);
        match self.state.bookmarks.iter().position(|b| b.path == path) {
            Some(index) => self.remove_bookmark(index),
            None => self.add_bookmark(path),
        }
    }

    /// `None` until the background check of that bookmark's folder has finished.
    pub fn bookmark_available(&self, index: usize) -> Option<bool> {
        let bookmark = self.state.bookmarks.get(index)?;
        self.availability.get(&bookmark.path).copied()
    }

    /// Re-checks every bookmarked folder on a worker thread (a dead network mount can block `stat` for seconds).
    /// Results arrive through [`Explorer::poll`]. Call on window focus so a remounted folder becomes usable.
    pub fn check_bookmarks(&mut self) {
        let paths = self
            .state
            .bookmarks
            .iter()
            .map(|b| b.path.clone())
            .collect();
        self.spawn_bookmark_checks(paths);
    }

    fn spawn_bookmark_checks(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let tx = self.bookmark_tx.clone();
        let wake = Arc::clone(&self.wake);
        thread::spawn(move || {
            for path in paths {
                let available = path.is_dir();
                let _ = tx.send(BookmarkChecked { path, available });
                wake();
            }
        });
    }

    /// Records finished bookmark checks. Returns true if any availability changed.
    pub(super) fn apply_bookmark_checks(&mut self) -> bool {
        let mut changed = false;
        while let Ok(checked) = self.bookmark_rx.try_recv() {
            // Results for bookmarks removed meanwhile are dropped.
            if self.is_bookmarked(&checked.path) {
                changed |= self.availability.insert(checked.path, checked.available)
                    != Some(checked.available);
            }
        }
        changed
    }
}
