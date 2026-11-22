//! Optimistic preloading of the active folder's subfolders, so entering one paints instantly.

use super::loading::list_view;
use super::{Explorer, Window, WindowId};
use crate::tab::LoadState;
use crate::view::DirView;
use fx_core::{EntryKind, ListError, StorageKind};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::thread;

/// At most this many subfolders are preloaded, in display order.
pub(super) const PRELOAD_MAX: usize = 16;
/// Listings larger than this are not cached; they would cost more memory than the instant paint is worth.
const PRELOAD_MAX_ENTRIES: usize = 20_000;

pub(super) struct PreloadDone {
    pub(super) window: WindowId,
    pub(super) generation: u64,
    pub(super) path: PathBuf,
    pub(super) result: Result<DirView, ListError>,
}

impl Window {
    /// Cancels the running preload and forgets its results.
    pub(super) fn cancel_preload(&mut self) {
        self.preload_gen.fetch_add(1, Ordering::AcqRel);
        self.preload_parent = None;
    }
}

impl Explorer {
    /// Cancels any running preload of window `w`, then lists its active folder's subfolders one by one in the background.
    /// Skipped when disabled, on network storage, or unless the active tab is loaded.
    pub(super) fn maybe_preload(&mut self, w: usize) {
        let enabled = self.settings.preload_subfolders;
        let window = &mut self.windows[w];
        window.cancel_preload();
        window.preloaded.clear();
        if !enabled {
            return;
        }
        let Some((parent, folders)) = preload_targets(window) else {
            return;
        };
        window.preload_parent = Some(parent);
        let window_id = window.id;
        let generation = window.preload_gen.load(Ordering::Acquire);
        let current = Arc::clone(&window.preload_gen);
        let tx = self.preload_tx.clone();
        let wake = Arc::clone(&self.wake);
        thread::spawn(move || {
            preload_folders(folders, window_id, generation, &current, &tx, &*wake);
        });
    }

    /// Caches finished preloads of their window's current preload; closed windows' and cancelled ones are dropped.
    pub(super) fn apply_preloads(&mut self) {
        while let Ok(done) = self.preload_rx.try_recv() {
            // A closed window's results are dropped with it.
            if let Some(window) = self.windows.iter_mut().find(|w| w.id == done.window)
                && done.generation == window.preload_gen.load(Ordering::Acquire)
                && let Ok(view) = done.result
                && view.listing.entries.len() <= PRELOAD_MAX_ENTRIES
            {
                window.preloaded.insert(done.path, view);
            }
        }
    }
}

/// The active folder and its first subfolders to list, unless the tab is unloaded or on network storage.
fn preload_targets(window: &Window) -> Option<(PathBuf, Vec<PathBuf>)> {
    let tab = &window.tabs[window.active];
    let LoadState::Loaded(view) = tab.state() else {
        return None;
    };
    if view.storage == StorageKind::Network {
        return None;
    }
    // Symlinks have their own kind, so only real directories are listed.
    let folders = tab
        .order()
        .iter()
        .map(|&i| &view.listing.entries[i as usize])
        .filter(|entry| entry.kind == EntryKind::Dir)
        .map(|entry| view.path.join(view.listing.name(entry)))
        .take(PRELOAD_MAX)
        .collect();
    Some((view.path.clone(), folders))
}

/// Lists `folders` one by one on the calling worker thread, stopping as soon as `current` moves past `generation`.
fn preload_folders(
    folders: Vec<PathBuf>,
    window: WindowId,
    generation: u64,
    current: &AtomicU64,
    tx: &Sender<PreloadDone>,
    wake: &(dyn Fn() + Send + Sync),
) {
    for path in folders {
        if current.load(Ordering::Acquire) != generation {
            return;
        }
        let result = list_view(path.clone());
        if current.load(Ordering::Acquire) != generation {
            return;
        }
        let _ = tx.send(PreloadDone {
            window,
            generation,
            path,
            result,
        });
        wake();
    }
}
