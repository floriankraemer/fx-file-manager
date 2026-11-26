//! The per-window folder tree: expanding, revealing and loading subfolders in the background.

use super::Explorer;
use crate::tree::{Children, FolderTree, TreeNode, list_children};
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

pub(super) struct TreeLoaded {
    generation: u64,
    path: PathBuf,
    result: Result<Vec<TreeNode>, String>,
}

impl TreeLoaded {
    fn children(&self) -> Children {
        match &self.result {
            Ok(nodes) => Children::Loaded(nodes.clone()),
            Err(err) => Children::Failed(err.clone()),
        }
    }
}

impl Explorer {
    pub fn tree(&self) -> &FolderTree {
        &self.win().tree
    }

    /// Expands a collapsed folder (loading its subfolders if needed) or collapses an expanded one. The cache is kept.
    pub fn tree_toggle(&mut self, path: PathBuf) {
        if !self.win_mut().tree.expanded.remove(&path) {
            self.tree_expand(path);
        }
    }

    /// Expands `path`, loading its subfolders unless they are cached. A failed load is retried.
    pub fn tree_expand(&mut self, path: PathBuf) {
        let tree = &mut self.win_mut().tree;
        let needs_load = !matches!(
            tree.children.get(&path),
            Some(Children::Loading | Children::Loaded(_))
        );
        if needs_load {
            tree.children.insert(path.clone(), Children::Loading);
        }
        tree.expanded.insert(path.clone());
        if needs_load {
            self.load_tree(path);
        }
    }

    /// Expands `root` and every folder between it and `path` (not `path` itself), where `root` is the deepest of `roots` containing `path`.
    /// The GUI calls this when the active tab's path changes.
    pub fn tree_reveal(&mut self, path: &Path, roots: &[PathBuf]) {
        let Some(root) = roots
            .iter()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
        else {
            return;
        };
        for dir in path.ancestors().skip(1) {
            if !dir.starts_with(root) {
                break;
            }
            self.tree_expand(dir.to_path_buf());
        }
    }

    /// Navigates to the visible tree node (under `roots`) whose name best matches the typed `text`. Returns true on a match.
    pub fn tree_type_ahead(&mut self, text: &str, roots: &[TreeNode], now: Instant) -> bool {
        let visible = self.win().tree.visible(roots);
        let active = self.active().path();
        let current = visible.iter().position(|node| node.path == active);
        let names = |i: usize| Some(Cow::Borrowed(visible.get(i)?.name.as_str()));
        let found = self
            .win_mut()
            .tree
            .type_ahead
            .find(text, now, current, visible.len(), names);
        let Some(node) = found.and_then(|i| visible.get(i)) else {
            return false;
        };
        self.navigate(node.path.clone());
        true
    }

    /// Invalidates every cached tree listing in all windows and reloads it.
    pub(super) fn reload_tree(&mut self) {
        self.tree_generation += 1;
        let cached: Vec<PathBuf> = self
            .windows
            .iter()
            .flat_map(|w| w.tree.children.keys().cloned())
            .collect();
        self.load_trees(cached);
    }

    pub(super) fn load_tree(&self, path: PathBuf) {
        self.load_trees(vec![path]);
    }

    /// Lists all `paths` on a single worker thread, so a long session cannot fan out into hundreds of threads.
    pub(super) fn load_trees(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let generation = self.tree_generation;
        let show_hidden = self.settings.show_hidden;
        let tx = self.tree_tx.clone();
        let wake = Arc::clone(&self.wake);
        thread::spawn(move || {
            for path in paths {
                let result = list_children(&path, show_hidden);
                let _ = tx.send(TreeLoaded {
                    generation,
                    path,
                    result,
                });
                wake();
            }
        });
    }

    /// Fills every window's tree slot for finished loads; stale generations are dropped. Returns true if any slot changed.
    pub(super) fn apply_tree_loads(&mut self) -> bool {
        let mut changed = false;
        while let Ok(done) = self.tree_rx.try_recv() {
            if done.generation != self.tree_generation {
                continue;
            }
            changed |= self.fill_tree_slots(&done);
        }
        changed
    }

    fn fill_tree_slots(&mut self, done: &TreeLoaded) -> bool {
        let mut changed = false;
        for window in &mut self.windows {
            if let Some(slot) = window.tree.children.get_mut(&done.path) {
                *slot = done.children();
                changed = true;
            }
        }
        changed
    }
}
