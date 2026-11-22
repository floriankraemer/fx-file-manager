use super::{Explorer, REVALIDATE_INTERVAL};
use crate::tab::Tab;
use crate::tree::FolderTree;
use crate::view::DirView;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

/// Identifies a window for the lifetime of the [`Explorer`]; never reused.
pub type WindowId = u64;

/// The per-window state: its own tabs, folder tree and preload cache.
/// Everything else (settings, clipboard, transfers, notices, job runner) is shared by all windows.
pub(super) struct Window {
    pub(super) id: WindowId,
    pub(super) tabs: Vec<Tab>,
    pub(super) active: usize,
    pub(super) tree: FolderTree,
    /// Listings of the active folder's subfolders; only an optimistic first paint, a fresh load always follows.
    pub(super) preloaded: HashMap<PathBuf, DirView>,
    /// Bumped to cancel the running preload thread and to drop its in-flight results.
    pub(super) preload_gen: Arc<AtomicU64>,
    /// Folder whose subfolders the current preload covers; `None` once it was cancelled or never started.
    pub(super) preload_parent: Option<PathBuf>,
    /// At most one mtime check runs at a time.
    pub(super) stat_in_flight: bool,
    pub(super) next_tick: Instant,
}

impl Window {
    pub(super) fn new(id: WindowId) -> Self {
        Self {
            id,
            tabs: Vec::new(),
            active: 0,
            tree: FolderTree::default(),
            preloaded: HashMap::new(),
            preload_gen: Arc::new(AtomicU64::new(0)),
            preload_parent: None,
            stat_in_flight: false,
            next_tick: Instant::now() + REVALIDATE_INTERVAL,
        }
    }
}

impl Explorer {
    pub(super) fn win(&self) -> &Window {
        &self.windows[self.current]
    }

    pub(super) fn win_mut(&mut self) -> &mut Window {
        &mut self.windows[self.current]
    }

    /// Appends an empty window with a fresh id and returns its index.
    pub(super) fn push_window(&mut self) -> usize {
        let id = self.next_window_id;
        self.next_window_id += 1;
        self.windows.push(Window::new(id));
        self.windows.len() - 1
    }

    /// Opens a window with one tab at `path` and starts loading it. The current window does not change.
    pub fn open_window(&mut self, path: PathBuf) -> WindowId {
        let index = self.push_window();
        self.push_tab(index, path);
        self.windows[index].id
    }

    /// The last remaining window is never closed. Unknown ids are ignored.
    pub fn close_window(&mut self, id: WindowId) {
        if self.windows.len() <= 1 {
            return;
        }
        let Some(index) = self.windows.iter().position(|w| w.id == id) else {
            return;
        };
        let current_id = self.win().id;
        self.windows[index].cancel_preload();
        self.windows.remove(index);
        self.session_dirty = true;
        self.current = self
            .windows
            .iter()
            .position(|w| w.id == current_id)
            .unwrap_or(0);
    }

    /// All windows in creation order.
    pub fn window_ids(&self) -> Vec<WindowId> {
        self.windows.iter().map(|w| w.id).collect()
    }

    /// The active folder of window `id`, if that window exists.
    pub fn window_folder(&self, id: WindowId) -> Option<PathBuf> {
        let w = self.windows.iter().find(|w| w.id == id)?;
        Some(w.tabs[w.active].path().to_path_buf())
    }

    /// The window the rest of the API acts on.
    pub fn window(&self) -> WindowId {
        self.win().id
    }

    /// Makes `id` the window the rest of the API acts on. Unknown ids are ignored.
    pub fn set_window(&mut self, id: WindowId) {
        if let Some(index) = self.windows.iter().position(|w| w.id == id) {
            self.current = index;
        }
    }
}
