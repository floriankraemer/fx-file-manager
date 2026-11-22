//! Opening the folder or file behind a row. The listing already knows most kinds; only a symlink needs its target checked, off the UI thread.

use super::{Explorer, open_with_system};
use crate::probe::timed;
use fx_core::EntryKind;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OpenTarget {
    ThisTab,
    NewTab,
}

/// Result of the background check whether a symlink or unusual entry is a folder.
pub(super) struct OpenChecked {
    generation: u64,
    path: PathBuf,
    is_dir: bool,
    target: OpenTarget,
}

impl Explorer {
    /// Enters `path` when it is a folder (or a symlink to one); opens a file with the system handler.
    /// `kind` is what the listing said about it, so only a symlink or an unusual entry costs a background check.
    pub(super) fn open_path(&mut self, path: PathBuf, kind: EntryKind, target: OpenTarget) {
        match kind {
            EntryKind::Dir => self.enter(path, target),
            EntryKind::File => open_file(&path, target),
            EntryKind::Symlink | EntryKind::Other => self.probe_open(path, target),
        }
    }

    fn enter(&mut self, path: PathBuf, target: OpenTarget) {
        match target {
            OpenTarget::ThisTab => self.navigate(path),
            OpenTarget::NewTab => self.new_tab(path),
        }
    }

    /// Only the latest request is acted on, so a slow check cannot override what the user opened since.
    fn probe_open(&mut self, path: PathBuf, target: OpenTarget) {
        self.open_generation += 1;
        let generation = self.open_generation;
        let tx = self.open_tx.clone();
        let wake = Arc::clone(&self.wake);
        thread::spawn(move || {
            let is_dir = timed("folder check", path.display(), || {
                fs::metadata(&path).is_ok_and(|meta| meta.is_dir())
            });
            let _ = tx.send(OpenChecked {
                generation,
                path,
                is_dir,
                target,
            });
            wake();
        });
    }

    /// Acts on finished folder checks. Returns true if a folder was entered.
    pub(super) fn apply_open_checks(&mut self) -> bool {
        let mut changed = false;
        while let Ok(checked) = self.open_rx.try_recv() {
            if checked.generation != self.open_generation {
                continue;
            }
            if checked.is_dir {
                self.enter(checked.path, checked.target);
                changed = true;
            } else {
                open_file(&checked.path, checked.target);
            }
        }
        changed
    }
}

fn open_file(path: &Path, target: OpenTarget) {
    if target == OpenTarget::NewTab {
        return;
    }
    if let Err(err) = open_with_system(path) {
        log::warn!("cannot open {}: {err}", path.display());
    }
}
