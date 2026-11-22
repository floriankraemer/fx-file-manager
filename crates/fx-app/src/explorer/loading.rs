//! Background listing of tab folders. Results carry the tab's generation so stale ones are dropped.

use super::Explorer;
use crate::tab::{LoadState, Tab};
use crate::view::DirView;
use fx_core::{ListError, ListOptions, list_dir};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Instant, SystemTime};

pub(super) struct Completed {
    tab_id: u64,
    generation: u64,
    /// A background re-list of an already shown folder, as opposed to a navigation.
    revalidation: bool,
    result: Result<DirView, ListError>,
}

impl Explorer {
    pub(super) fn start_load(&mut self, w: usize, index: usize) {
        self.session_dirty = true;
        let window = &mut self.windows[w];
        let tab = &mut window.tabs[index];
        tab.clear_search_if_moved();
        let generation = tab.begin_load();
        let tab_id = tab.id();
        let path = tab.path().to_path_buf();
        // Navigating cancels a running preload; the next completed load starts a new one.
        window.cancel_preload();
        if self.settings.preload_subfolders
            && let Some(view) = window.preloaded.remove(&path)
        {
            // Same generation as the fresh load below, which then replaces this possibly stale view.
            window.tabs[index].apply_load(generation, Ok(view));
        }
        self.spawn_load(tab_id, generation, path, false);
    }

    pub(super) fn spawn_load(
        &self,
        tab_id: u64,
        generation: u64,
        path: PathBuf,
        revalidation: bool,
    ) {
        let tx = self.tx.clone();
        let wake = Arc::clone(&self.wake);
        thread::spawn(move || {
            let started = Instant::now();
            let result = list_view(path.clone());
            log_listing(&path, &result, started);
            // A send only fails once the Explorer is gone, and then nobody needs the result.
            let _ = tx.send(Completed {
                tab_id,
                generation,
                revalidation,
                result,
            });
            wake();
        });
    }

    /// Applies finished loads to their tabs in any window, then preloads for windows whose active tab just loaded.
    /// Returns true if any tab changed.
    pub(super) fn apply_loads(&mut self) -> bool {
        let mut changed = false;
        let mut loaded = Vec::new();
        while let Ok(done) = self.rx.try_recv() {
            let Some(applied) = self.apply_completed(done) else {
                continue;
            };
            changed |= applied.changed;
            loaded.extend(applied.active_loaded);
        }
        for w in loaded {
            self.preload_unless_current(w);
        }
        changed
    }

    fn apply_completed(&mut self, done: Completed) -> Option<Applied> {
        let (w, window) = self
            .windows
            .iter_mut()
            .enumerate()
            .find(|(_, window)| window.tabs.iter().any(|tab| tab.id() == done.tab_id))?;
        let active_id = window.tabs[window.active].id();
        let tab = window.tabs.iter_mut().find(|tab| tab.id() == done.tab_id)?;
        if keeps_loaded_view(tab, &done) {
            return None;
        }
        let ok = done.result.is_ok();
        let changed = tab.apply_load(done.generation, done.result.map_err(|e| e.to_string()));
        let active_loaded = (changed && ok && tab.id() == active_id).then_some(w);
        Some(Applied {
            changed,
            active_loaded,
        })
    }

    /// A revalidation of the folder already being preloaded keeps that preload: cached views are
    /// only a first paint that a fresh load always replaces, so redoing up to PRELOAD_MAX listings
    /// on every focus change or mtime tick would buy nothing.
    fn preload_unless_current(&mut self, w: usize) {
        let window = &self.windows[w];
        if window.preload_parent.as_deref() != Some(window.tabs[window.active].path()) {
            self.maybe_preload(w);
        }
    }
}

fn log_listing(path: &Path, result: &Result<DirView, ListError>, started: Instant) {
    match result {
        Ok(view) => log::debug!(
            "listed {} ({} entries) in {:?}",
            path.display(),
            view.listing.entries.len(),
            started.elapsed()
        ),
        Err(err) => log::warn!("cannot list {}: {err}", path.display()),
    }
}

/// A transient error in a background revalidation must not replace a good listing with a failure page.
fn keeps_loaded_view(tab: &Tab, done: &Completed) -> bool {
    let Err(err) = &done.result else {
        return false;
    };
    let keep = done.revalidation
        && done.generation == tab.generation()
        && matches!(tab.state(), LoadState::Loaded(_));
    if keep {
        log::warn!(
            "revalidating {} failed, keeping the shown listing: {err}",
            tab.path().display()
        );
    }
    keep
}

/// Outcome of applying one finished load.
struct Applied {
    changed: bool,
    /// The window whose active tab just finished loading and may need a preload.
    active_loaded: Option<usize>,
}

/// Stats the folder before listing it, so a change made while listing leaves the recorded mtime older than the folder's.
pub(super) fn list_view(path: PathBuf) -> Result<DirView, ListError> {
    let listed_at = SystemTime::now();
    let mtime = dir_mtime(&path);
    let started = Instant::now();
    list_dir(&path, &list_options())
        .map(|listed| DirView::new(path, listed, started.elapsed(), mtime, listed_at))
}

pub(super) fn dir_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

/// Listing is sorted by the view model, so the backend's own sort is skipped.
fn list_options() -> ListOptions {
    ListOptions {
        sort: false,
        ..ListOptions::default()
    }
}
