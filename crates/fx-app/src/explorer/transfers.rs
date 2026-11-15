//! The shared clipboard, drag and drop, and the copy/move transfer queue.

use super::Explorer;
use crate::ops::{NoticeKind, display_name};
use crate::transfers::TransferInfo;
use fx_core::{Op, Request, Resolution};
use std::path::{Path, PathBuf};

impl Explorer {
    /// Remembers the entries at `rows` for a later `paste`. `cut = true` moves them on paste.
    pub fn copy_to_clipboard(&mut self, rows: &[usize], cut: bool) {
        let paths: Vec<PathBuf> = rows
            .iter()
            .filter_map(|&row| self.entry_path(row))
            .collect();
        if paths.is_empty() {
            return self.notify(NoticeKind::Error, crate::t("notice.nothing_selected"));
        }
        self.clipboard = Some((paths, cut));
    }

    pub fn can_paste(&self) -> bool {
        self.clipboard.is_some()
    }

    /// Pastes the clipboard into `folder` (default: the active tab's folder) as a background transfer.
    /// A cut clipboard is spent; cutting into the item's own folder does nothing.
    pub fn paste(&mut self, folder: Option<PathBuf>) {
        let Some((sources, cut)) = self.clipboard.clone() else {
            return;
        };
        let dest = folder.unwrap_or_else(|| self.active().path().to_path_buf());
        if cut {
            self.clipboard = None;
        }
        self.drop_into(sources, &dest, !cut);
    }

    /// Copies (`copy = true`) or moves `sources` into the folder `dest` as a background transfer.
    /// A move into the item's own folder does nothing.
    pub fn drop_into(&mut self, sources: Vec<PathBuf>, dest: &Path, copy: bool) {
        let sources: Vec<PathBuf> = sources
            .into_iter()
            .filter(|src| copy || src.parent() != Some(dest))
            .collect();
        if sources.is_empty() {
            return;
        }
        let op = if copy { Op::Copy } else { Op::Move };
        let mut req = Request::into_dir(&sources, dest, op, self.settings.conflict_policy.into());
        if self.settings.copy_workers > 0 {
            req.overrides.workers = Some(self.settings.copy_workers as usize);
        }
        req.overrides.verify = self.settings.verify_copies;
        let key = if copy {
            "transfer.copying"
        } else {
            "transfer.moving"
        };
        let label = crate::tn(key, sources.len() as u64, &[("dest", &display_name(dest))]);
        self.transfers.submit(req, label);
        self.poll_transfers();
    }

    /// Copy and move jobs for a transfer panel, oldest first.
    pub fn transfers(&self) -> Vec<TransferInfo> {
        self.transfers.infos()
    }

    /// True while a transfer runs or waits; a UI should then poll about ten times a second.
    pub fn transfers_active(&self) -> bool {
        self.transfers.active()
    }

    pub fn pause_transfer(&self, id: u64) {
        self.transfers.pause(id);
    }

    /// Also answers "disk full" with retry.
    pub fn resume_transfer(&self, id: u64) {
        self.transfers.resume(id);
    }

    pub fn cancel_transfer(&mut self, id: u64) {
        self.transfers.cancel(id);
    }

    /// Answers "disk full" with skip: files that do not fit fail instead of pausing.
    pub fn skip_disk_full(&self, id: u64) {
        self.transfers.skip_disk_full(id);
    }

    pub fn answer_conflict(
        &self,
        id: u64,
        conflict: u64,
        resolution: Resolution,
        apply_to_all: bool,
    ) {
        self.transfers
            .answer(id, conflict, resolution, apply_to_all);
    }

    /// Removes a finished transfer (with errors) from the panel.
    pub fn dismiss_transfer(&mut self, id: u64) {
        self.transfers.dismiss(id);
    }

    pub(super) fn poll_transfers(&mut self) -> bool {
        let (outcomes, changed) = self.transfers.poll(&self.wake);
        for outcome in outcomes {
            self.apply_outcome(outcome);
        }
        changed
    }
}
