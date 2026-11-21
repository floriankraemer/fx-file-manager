//! Owns the tabs and the background loader. Navigation goes through here because only Explorer can start loads.

mod bookmarks;
mod chooser;
mod file_ops;
mod loading;
mod opener;
mod preload;
mod revalidate;
mod session;
mod settings;
mod tabs;
mod transfers;
mod tree;

#[cfg(test)]
mod tests;

pub use chooser::open_with_chooser;
pub use opener::open_with_system;
pub use revalidate::REVALIDATE_INTERVAL;

use crate::ops::{Notice, NoticeKind, Outcome};
use crate::settings::Settings;
use crate::state::State;
use crate::tab::Tab;
use crate::transfers::Transfers;
use crate::tree::FolderTree;
use crate::view::DirView;
use bookmarks::BookmarkChecked;
use loading::Completed;
use preload::PreloadDone;
use revalidate::StatChecked;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;
use tree::TreeLoaded;

/// Identifies a window for the lifetime of the [`Explorer`]; never reused.
pub type WindowId = u64;

/// The per-window state: its own tabs, folder tree and preload cache.
/// Everything else (settings, clipboard, transfers, notices, job runner) is shared by all windows.
struct Window {
    id: WindowId,
    tabs: Vec<Tab>,
    active: usize,
    tree: FolderTree,
    /// Listings of the active folder's subfolders; only an optimistic first paint, a fresh load always follows.
    preloaded: HashMap<PathBuf, DirView>,
    /// Bumped to cancel the running preload thread and to drop its in-flight results.
    preload_gen: Arc<AtomicU64>,
    /// Folder whose subfolders the current preload covers; `None` once it was cancelled or never started.
    preload_parent: Option<PathBuf>,
    /// At most one mtime check runs at a time.
    stat_in_flight: bool,
    next_tick: Instant,
}

impl Window {
    fn new(id: WindowId) -> Self {
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

pub struct Explorer {
    windows: Vec<Window>,
    /// Index into `windows` of the window the tab/tree/navigation API acts on.
    current: usize,
    next_window_id: WindowId,
    /// Global so tab ids stay unique across windows and results find their tab in any window.
    next_tab_id: u64,
    tx: Sender<Completed>,
    rx: Receiver<Completed>,
    job_tx: Sender<Outcome>,
    job_rx: Receiver<Outcome>,
    /// One delete/rename/create at a time: a second request is refused while this is set.
    // ponytail: serial jobs; add a queue if users need to chain operations while one runs
    job_in_flight: bool,
    /// Copies and moves: several at once, queued per HDD/removable volume.
    transfers: Transfers,
    tree_tx: Sender<TreeLoaded>,
    tree_rx: Receiver<TreeLoaded>,
    /// Bumped when cached children become invalid wholesale; older tree results are dropped.
    tree_generation: u64,
    wake: Arc<dyn Fn() + Send + Sync>,
    settings: Settings,
    /// Where `set_settings` persists. `None` when the platform has no config directory.
    settings_path: Option<PathBuf>,
    notices: VecDeque<Notice>,
    /// Internal clipboard set by copy/cut: absolute paths and whether paste should move them.
    /// Shared by all windows, so copy in one window pastes in another.
    // ponytail: app-internal only, not the OS file clipboard; add per-platform clipboard integration if cross-app paste is needed
    clipboard: Option<(Vec<PathBuf>, bool)>,
    preload_tx: Sender<PreloadDone>,
    preload_rx: Receiver<PreloadDone>,
    stat_tx: Sender<StatChecked>,
    stat_rx: Receiver<StatChecked>,
    /// Bookmarks and the last session, persisted to `state_path`.
    state: State,
    /// Where `state.toml` is written. `None` disables persisting, e.g. when the platform has no config directory.
    state_path: Option<PathBuf>,
    /// The tab list or a tab path changed since the last write.
    session_dirty: bool,
    next_session_save: Instant,
    /// Whether each bookmarked folder exists, as last checked off the UI thread. Missing means not checked yet.
    availability: HashMap<PathBuf, bool>,
    bookmark_tx: Sender<BookmarkChecked>,
    bookmark_rx: Receiver<BookmarkChecked>,
}

impl Explorer {
    /// `wake` is called from worker threads when a result is ready; a UI passes `ctx.request_repaint`.
    pub fn new(start: PathBuf, wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_settings(start, Settings::default(), wake)
    }

    pub fn with_settings(
        start: PathBuf,
        settings: Settings,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let mut explorer = Self::build(settings, State::default(), None, wake);
        explorer.new_tab(start);
        explorer
    }

    /// Starts with the saved bookmarks and, unless `start` is given, the saved session.
    /// `start` (a folder from the command line) wins over the session. Otherwise one window per saved
    /// window that still has a directory among its tabs is opened, loading only each window's active tab.
    /// Without `settings.restore_session`, or when no saved tab survives, one tab opens at `fallback`.
    /// `state_path` is where bookmark and session changes are written; `None` disables that.
    pub fn with_state(
        start: Option<PathBuf>,
        fallback: PathBuf,
        settings: Settings,
        state: State,
        state_path: Option<PathBuf>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let restore = start.is_none() && settings.restore_session;
        let saved = if restore {
            state.windows.clone()
        } else {
            Vec::new()
        };
        let mut explorer = Self::build(settings, state, state_path, wake);
        if explorer.restore_windows(saved) == 0 {
            explorer.new_tab(start.unwrap_or(fallback));
        }
        explorer.session_dirty = false;
        explorer.check_bookmarks();
        explorer
    }

    /// An explorer with one empty window; the caller must add a tab.
    fn build(
        settings: Settings,
        state: State,
        state_path: Option<PathBuf>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let (job_tx, job_rx) = mpsc::channel();
        let (tree_tx, tree_rx) = mpsc::channel();
        let (preload_tx, preload_rx) = mpsc::channel();
        let (stat_tx, stat_rx) = mpsc::channel();
        let (bookmark_tx, bookmark_rx) = mpsc::channel();
        Self {
            windows: vec![Window::new(0)],
            current: 0,
            next_window_id: 1,
            next_tab_id: 0,
            tx,
            rx,
            job_tx,
            job_rx,
            job_in_flight: false,
            transfers: Transfers::default(),
            tree_tx,
            tree_rx,
            tree_generation: 0,
            wake: Arc::new(wake),
            settings,
            settings_path: Settings::path(),
            notices: VecDeque::new(),
            clipboard: None,
            preload_tx,
            preload_rx,
            stat_tx,
            stat_rx,
            state,
            state_path,
            session_dirty: false,
            next_session_save: Instant::now(),
            availability: HashMap::new(),
            bookmark_tx,
            bookmark_rx,
        }
    }

    fn win(&self) -> &Window {
        &self.windows[self.current]
    }

    fn win_mut(&mut self) -> &mut Window {
        &mut self.windows[self.current]
    }

    /// Appends an empty window with a fresh id and returns its index.
    fn push_window(&mut self) -> usize {
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

    /// The oldest pending notice, if any.
    pub fn take_notice(&mut self) -> Option<Notice> {
        self.notices.pop_front()
    }

    /// Shows `text` as an error notice.
    pub fn notify_error(&mut self, text: impl Into<String>) {
        self.notify(NoticeKind::Error, text);
    }

    fn notify(&mut self, kind: NoticeKind, text: impl Into<String>) {
        self.notices.push_back(Notice {
            kind,
            text: text.into(),
        });
    }

    /// Applies finished loads and file operations. Results for closed tabs or superseded requests are dropped.
    /// Returns true if any tab or notice changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = self.apply_bookmark_checks();
        if self.session_dirty && Instant::now() >= self.next_session_save {
            self.save_state();
        }
        changed |= self.apply_loads();
        self.apply_preloads();
        self.apply_stat_checks();
        for w in 0..self.windows.len() {
            self.tick_revalidate(w);
        }
        changed |= self.apply_tree_loads();
        changed |= self.apply_job_outcomes();
        changed |= self.poll_transfers();
        changed
    }
}

/// Falls back to the path as given when the working directory is unavailable; the load then reports the real error.
fn absolute(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}
