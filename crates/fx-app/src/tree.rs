//! Folder-tree state for the sidebar: which folders are expanded and their cached subfolders.

use fx_core::{EntryKind, ListOptions, list_dir, natural_cmp};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeNode {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Children {
    Loading,
    Loaded(Vec<TreeNode>),
    Failed(String),
}

#[derive(Default)]
pub struct FolderTree {
    pub(crate) expanded: HashSet<PathBuf>,
    pub(crate) children: HashMap<PathBuf, Children>,
}

impl FolderTree {
    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    /// `None` until a load for `path` was requested.
    pub fn children(&self, path: &Path) -> Option<&Children> {
        self.children.get(path)
    }
}

/// Subfolders of `dir` (symlinks to folders included), natural-sorted. Runs on a worker thread.
pub(crate) fn list_children(dir: &Path, show_hidden: bool) -> Result<Vec<TreeNode>, String> {
    let opts = ListOptions {
        metadata: false,
        sort: false,
        ..ListOptions::default()
    };
    let listed = list_dir(dir, &opts).map_err(|e| e.to_string())?.listing;
    let mut nodes: Vec<TreeNode> = listed
        .entries
        .iter()
        .filter(|e| show_hidden || !e.hidden)
        .filter_map(|e| {
            let path = dir.join(listed.name(e));
            let is_dir = match e.kind {
                EntryKind::Dir => true,
                EntryKind::Symlink => path.is_dir(),
                _ => false,
            };
            is_dir.then(|| TreeNode {
                name: listed.name(e).to_string_lossy().into_owned(),
                path,
            })
        })
        .collect();
    nodes.sort_by(|a, b| natural_cmp(a.name.as_bytes(), b.name.as_bytes()));
    Ok(nodes)
}
