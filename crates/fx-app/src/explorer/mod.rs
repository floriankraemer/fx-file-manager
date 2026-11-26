//! Owns the tabs and the background loader. Navigation goes through here because only Explorer can start loads.

mod bookmarks;
mod chooser;
mod details;
mod entries;
mod file_ops;
mod loading;
mod notices;
mod open;
mod opener;
mod places;
mod preload;
mod previews;
mod restore;
mod revalidate;
mod search;
mod session;
mod settings;
mod tabs;
mod transfers;
mod tree;
mod watch;
mod windows;

#[cfg(test)]
mod search_tests;
#[cfg(test)]
mod tests;

pub use details::DetailsState;
pub use opener::open_with_system;
pub use revalidate::REVALIDATE_INTERVAL;
pub use search::SearchStart;

use crate::ops::{Notice, Outcome};
use crate::places::Place;
use crate::search::SearchRunner;
use crate::settings::Settings;
use crate::state::State;
use crate::thumbnails::Thumbnails;
use crate::transfers::Transfers;
use bookmarks::BookmarkChecked;
use details::DetailsLoaded;
use loading::Completed;
use open::OpenChecked;
use preload::PreloadDone;
use restore::TabChecked;
use revalidate::StatChecked;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;
use tree::TreeLoaded;
use watch::Watches;
use windows::Window;
pub use windows::WindowId;

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
    /// Errors that background helpers, such as the Open with chooser, report back.
    error_tx: Sender<String>,
    error_rx: Receiver<String>,
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
    /// Files whose last save failed. A repeated failure stays silent so a read-only config folder does not flood the UI and log.
    failing_saves: HashSet<&'static str>,
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
    /// Bookmarks whose check has not answered yet; a path stuck on a dead mount is not re-checked, so threads cannot pile up.
    bookmark_checking: HashSet<PathBuf>,
    bookmark_tx: Sender<BookmarkChecked>,
    bookmark_rx: Receiver<BookmarkChecked>,
    searches: SearchRunner,
    /// The root notice was acknowledged; it is shown once per app session.
    root_notice_seen: bool,
    /// Bumped per background folder check; older answers are dropped.
    open_generation: u64,
    open_tx: Sender<OpenChecked>,
    open_rx: Receiver<OpenChecked>,
    /// Where a startup tab whose folder is gone goes when it is its window's only tab.
    restore_fallback: PathBuf,
    pending_tab_checks: usize,
    tab_check_tx: Sender<TabChecked>,
    tab_check_rx: Receiver<TabChecked>,
    /// Bumped per properties request; older answers are dropped.
    details_generation: u64,
    details: Option<DetailsState>,
    details_tx: Sender<DetailsLoaded>,
    details_rx: Receiver<DetailsLoaded>,
    places_probing: bool,
    /// The latest probed locations that the UI has not taken yet.
    fresh_places: Option<Vec<Place>>,
    places_tx: Sender<Vec<Place>>,
    places_rx: Receiver<Vec<Place>>,
    watches: Watches,
    thumbnails: Thumbnails,
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
    /// window is opened at once, loading only each window's active tab; a background check then drops
    /// tabs whose folder is gone, or sends a window's only tab to `fallback`.
    /// Without `settings.restore_session`, or with no saved tab, one tab opens at `fallback`.
    /// `state_path` is where bookmark and session changes are written; `None` disables that.
    pub fn with_state(
        start: Option<PathBuf>,
        fallback: PathBuf,
        settings: Settings,
        state: State,
        state_path: Option<PathBuf>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let unverified_start = start.is_none();
        let restore = unverified_start && settings.restore_session;
        let saved = if restore {
            state.windows.clone()
        } else {
            Vec::new()
        };
        let mut explorer = Self::build(settings, state, state_path, wake);
        explorer.restore_fallback = absolute(fallback.clone());
        if explorer.restore_windows(saved) == 0 {
            explorer.new_tab(start.unwrap_or(fallback));
        }
        if unverified_start {
            explorer.check_restored_tabs();
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
        let (error_tx, error_rx) = mpsc::channel();
        let (tree_tx, tree_rx) = mpsc::channel();
        let (preload_tx, preload_rx) = mpsc::channel();
        let (stat_tx, stat_rx) = mpsc::channel();
        let (bookmark_tx, bookmark_rx) = mpsc::channel();
        let (open_tx, open_rx) = mpsc::channel();
        let (tab_check_tx, tab_check_rx) = mpsc::channel();
        let (details_tx, details_rx) = mpsc::channel();
        let (places_tx, places_rx) = mpsc::channel();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        Self {
            windows: vec![Window::new(0)],
            current: 0,
            next_window_id: 1,
            next_tab_id: 0,
            tx,
            rx,
            job_tx,
            job_rx,
            error_tx,
            error_rx,
            job_in_flight: false,
            transfers: Transfers::default(),
            tree_tx,
            tree_rx,
            tree_generation: 0,
            wake: Arc::clone(&wake),
            settings,
            settings_path: Settings::path(),
            notices: VecDeque::new(),
            failing_saves: HashSet::new(),
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
            bookmark_checking: HashSet::new(),
            bookmark_tx,
            bookmark_rx,
            searches: SearchRunner::new(Arc::clone(&wake)),
            root_notice_seen: false,
            open_generation: 0,
            open_tx,
            open_rx,
            restore_fallback: PathBuf::new(),
            pending_tab_checks: 0,
            tab_check_tx,
            tab_check_rx,
            details_generation: 0,
            details: None,
            details_tx,
            details_rx,
            places_probing: false,
            fresh_places: None,
            places_tx,
            places_rx,
            thumbnails: Thumbnails::new(Arc::clone(&wake)),
            watches: Watches::new(wake),
        }
    }

    /// Applies finished loads and file operations. Results for closed tabs or superseded requests are dropped.
    /// Returns true if any tab or notice changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = self.apply_bookmark_checks();
        if self.session_dirty && Instant::now() >= self.next_session_save {
            self.save_state();
        }
        changed |= self.apply_loads();
        changed |= self.apply_searches();
        changed |= self.apply_open_checks();
        changed |= self.apply_tab_checks();
        changed |= self.apply_details();
        changed |= self.apply_places();
        self.apply_preloads();
        self.sync_watches();
        self.apply_dir_changes();
        self.apply_stat_checks();
        for w in 0..self.windows.len() {
            self.tick_revalidate(w);
        }
        changed |= self.apply_tree_loads();
        changed |= self.apply_job_outcomes();
        changed |= self.apply_helper_errors();
        changed |= self.poll_transfers();
        changed
    }
}

/// Falls back to the path as given when the working directory is unavailable; the load then reports the real error.
fn absolute(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}
