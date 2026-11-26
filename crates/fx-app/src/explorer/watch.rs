//! Re-lists folders as soon as the OS reports a change in them. Every tab's folder and every expanded
//! tree folder is watched; the mtime polling in `revalidate` stays as the fallback for file systems
//! without change notifications, such as some network mounts.

use super::Explorer;
use crate::tab::LoadState;
use fx_core::DirWatcher;
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

pub(super) struct Watches {
    watcher: DirWatcher,
    /// What the watcher was last told to watch, sorted.
    dirs: Vec<PathBuf>,
    changes: Arc<Mutex<Changes>>,
    tabs: InFlight<u64>,
    trees: InFlight<PathBuf>,
}

/// Folders reported changed since the last poll.
#[derive(Default)]
struct Changes {
    dirs: HashSet<PathBuf>,
    /// The OS dropped events, so any watched folder may have changed.
    all: bool,
}

impl Changes {
    fn is_empty(&self) -> bool {
        self.dirs.is_empty() && !self.all
    }
}

impl Watches {
    pub(super) fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let changes = Arc::new(Mutex::new(Changes::default()));
        let sink = Arc::clone(&changes);
        let watcher = DirWatcher::new(move |dir| {
            let mut changes = sink.lock().unwrap_or_else(PoisonError::into_inner);
            // One wake per poll is enough: a copy of thousands of files must not flood the UI with repaints.
            let wake_ui = changes.is_empty();
            match dir {
                Some(dir) => changes.dirs.insert(dir.to_path_buf()),
                None => mem::replace(&mut changes.all, true),
            };
            drop(changes);
            if wake_ui {
                wake();
            }
        });
        Self {
            watcher,
            dirs: Vec::new(),
            changes,
            tabs: InFlight::default(),
            trees: InFlight::default(),
        }
    }
}

/// Re-lists running per key, so a burst of changes costs at most one listing in flight plus one after it.
struct InFlight<K>(HashMap<K, bool>);

impl<K> Default for InFlight<K> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

impl<K: Hash + Eq> InFlight<K> {
    /// True if a re-list of `key` should start now; otherwise one is queued behind the running one.
    fn start(&mut self, key: K) -> bool {
        if let Some(again) = self.0.get_mut(&key) {
            *again = true;
            return false;
        }
        self.0.insert(key, false);
        true
    }

    /// Re-lists `key` once its running load ends.
    fn defer(&mut self, key: K) {
        self.0.insert(key, true);
    }

    /// Ends the running load of `key`; true if a change arrived meanwhile and needs another re-list.
    fn finish<Q: Hash + Eq + ?Sized>(&mut self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        self.0.remove(key).unwrap_or(false)
    }
}

impl Explorer {
    /// Points the watcher at the folders currently shown: all tabs and expanded tree folders.
    pub(super) fn sync_watches(&mut self) {
        let mut wanted: Vec<&Path> = self
            .windows
            .iter()
            .flat_map(|w| {
                w.tabs
                    .iter()
                    .map(|tab| tab.path())
                    .chain(w.tree.expanded.iter().map(PathBuf::as_path))
            })
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        if wanted.iter().eq(self.watches.dirs.iter()) {
            return;
        }
        self.watches.dirs = wanted.into_iter().map(Path::to_path_buf).collect();
        self.watches.watcher.watch(self.watches.dirs.clone());
    }

    /// Re-lists every tab and tree folder whose folder the OS reported changed.
    pub(super) fn apply_dir_changes(&mut self) {
        let changes = mem::take(
            &mut *self
                .watches
                .changes
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        if changes.all {
            log::debug!("folder change events were dropped, re-listing every watched folder");
            for dir in self.watches.dirs.clone() {
                self.relist_dir(&dir);
            }
            return;
        }
        for dir in changes.dirs {
            self.relist_dir(&dir);
        }
    }

    fn relist_dir(&mut self, dir: &Path) {
        let tabs: Vec<u64> = self
            .windows
            .iter()
            .flat_map(|w| &w.tabs)
            .filter(|tab| tab.path() == dir)
            .map(|tab| tab.id())
            .collect();
        for tab_id in tabs {
            self.relist_tab(tab_id);
        }
        if self
            .windows
            .iter()
            .any(|w| w.tree.children.contains_key(dir))
        {
            self.relist_tree(dir.to_path_buf());
        }
    }

    /// Re-lists tab `tab_id`'s folder in the background without clearing the view.
    /// A tab that is still loading is re-listed after that load, which may predate the change.
    pub(super) fn relist_tab(&mut self, tab_id: u64) {
        let Some(tab) = self
            .windows
            .iter()
            .flat_map(|w| &w.tabs)
            .find(|tab| tab.id() == tab_id)
        else {
            return;
        };
        match tab.state() {
            LoadState::Loading => self.watches.tabs.defer(tab_id),
            LoadState::Loaded(_) if self.watches.tabs.start(tab_id) => {
                self.spawn_load(tab_id, tab.generation(), tab.path().to_path_buf(), true);
            }
            LoadState::Loaded(_) | LoadState::Failed(_) => {}
        }
    }

    /// Called for every finished load of `tab_id`; true if it must be re-listed again.
    pub(super) fn tab_load_finished(&mut self, tab_id: u64) -> bool {
        self.watches.tabs.finish(&tab_id)
    }

    /// Re-lists a tree folder in the background; its shown subfolders stay until the result arrives.
    pub(super) fn relist_tree(&mut self, path: PathBuf) {
        if self.watches.trees.start(path.clone()) {
            self.load_tree(path);
        }
    }

    /// Called for every finished tree load of `path`; true if it must be re-listed again.
    pub(super) fn tree_load_finished(&mut self, path: &Path) -> bool {
        self.watches.trees.finish(path)
    }
}
