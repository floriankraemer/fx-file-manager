//! Background check that the tabs opened at startup still point at folders. Startup never waits for it.

use super::Explorer;
use crate::probe::timed;
use crate::settings::home_dir;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

/// Result of the background `is_dir` check of one startup tab path.
pub(super) struct TabChecked {
    path: PathBuf,
    is_dir: bool,
}

impl Explorer {
    /// Checks every distinct tab folder on its own thread, so a dead mount only stalls its own check.
    /// A missing folder's tab is dropped, or sent to the fallback folder when it is its window's only tab.
    pub(super) fn check_restored_tabs(&mut self) {
        let paths: HashSet<PathBuf> = self
            .windows
            .iter()
            .flat_map(|w| w.tabs.iter().map(|tab| tab.path().to_path_buf()))
            .collect();
        for path in paths {
            self.pending_tab_checks += 1;
            let tx = self.tab_check_tx.clone();
            let wake = Arc::clone(&self.wake);
            thread::spawn(move || {
                let is_dir = timed("tab folder check", path.display(), || path.is_dir());
                let _ = tx.send(TabChecked { path, is_dir });
                wake();
            });
        }
    }

    /// Applies finished startup checks. Returns true if a tab was dropped or redirected.
    pub(super) fn apply_tab_checks(&mut self) -> bool {
        let mut changed = false;
        while let Ok(checked) = self.tab_check_rx.try_recv() {
            self.pending_tab_checks -= 1;
            if !checked.is_dir {
                changed |= self.resolve_missing(&checked.path);
            }
        }
        changed
    }

    fn resolve_missing(&mut self, missing: &Path) -> bool {
        log::info!("saved tab folder {} is gone", missing.display());
        let replacement = self.replacement_for(missing);
        let mut changed = false;
        for w in 0..self.windows.len() {
            changed |= self.resolve_missing_in(w, missing, &replacement);
        }
        changed
    }

    fn resolve_missing_in(&mut self, w: usize, missing: &Path, replacement: &Path) -> bool {
        let mut changed = false;
        while let Some(index) = self.windows[w]
            .tabs
            .iter()
            .position(|tab| tab.path() == missing)
        {
            if self.windows[w].tabs.len() == 1 {
                return changed | self.redirect(w, index, replacement);
            }
            self.remove_tab(w, index);
            changed = true;
        }
        changed
    }

    fn replacement_for(&self, missing: &Path) -> PathBuf {
        if missing != self.restore_fallback {
            return self.restore_fallback.clone();
        }
        home_dir().unwrap_or_else(|| PathBuf::from("/"))
    }

    fn redirect(&mut self, w: usize, index: usize, replacement: &Path) -> bool {
        if self.windows[w].tabs[index].path() == replacement {
            return false;
        }
        self.windows[w].tabs[index].navigate_to(replacement.to_path_buf());
        self.start_load(w, index);
        true
    }
}
