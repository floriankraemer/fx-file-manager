//! Acting on the rows of the active tab: opening, properties and helper errors.

use super::chooser::open_with_chooser;
use super::{Explorer, open_with_system};
use crate::ops::{EntryDetails, details};
use crate::tab::LoadState;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

impl Explorer {
    /// Enters a directory (or a symlink to one); opens anything else with the system handler.
    /// Does nothing while the tab is not loaded.
    pub fn activate(&mut self, row: usize) {
        let tab = self.active();
        let LoadState::Loaded(view) = tab.state() else {
            return;
        };
        let Some(&index) = tab.order().get(row) else {
            return;
        };
        let path = tab
            .path()
            .join(view.listing.name(&view.listing.entries[index as usize]));
        self.open_path(path);
    }

    /// Enters `path` when it is a directory (or a symlink to one); opens anything else with the system handler.
    pub(super) fn open_path(&mut self, path: PathBuf) {
        if fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            self.navigate(path);
        } else if let Err(err) = open_with_system(&path) {
            eprintln!("fx: cannot open {}: {err}", path.display());
        }
    }

    /// Lets the user pick the application for the file at `row`; shows a notice when the system has no chooser.
    pub fn open_with(&self, row: usize) {
        let Some(path) = self.entry_path(row) else {
            return;
        };
        let tx = self.error_tx.clone();
        let wake = Arc::clone(&self.wake);
        open_with_chooser(&path, move || {
            let _ = tx.send(crate::t("notice.open_with_unavailable").to_owned());
            wake();
        });
    }

    /// Shows the errors background helpers reported. Returns true if there were any.
    pub(super) fn apply_helper_errors(&mut self) -> bool {
        let mut changed = false;
        while let Ok(text) = self.error_rx.try_recv() {
            self.notify_error(text);
            changed = true;
        }
        changed
    }

    /// Opens the folder at `row` in a new tab. Non-folders are ignored.
    pub fn open_in_new_tab(&mut self, row: usize) {
        let Some(path) = self.entry_path(row) else {
            return;
        };
        if fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            self.new_tab(path);
        }
    }

    /// Full path of a visible row of the active tab.
    pub fn entry_path(&self, row: usize) -> Option<PathBuf> {
        let tab = self.active();
        Some(tab.path().join(tab.name_at(row)?))
    }

    /// Properties for a visible row of the active tab. Reads metadata now, on the calling thread; `None` if the row or entry is gone.
    pub fn entry_details(&self, row: usize) -> Option<EntryDetails> {
        details(&self.entry_path(row)?)
    }
}
