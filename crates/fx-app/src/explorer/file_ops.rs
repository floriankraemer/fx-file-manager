//! Delete, rename and create-folder jobs, run one at a time in the background, and applying their outcomes.

use super::Explorer;
use crate::ops::{Job, NoticeKind, Outcome, unique_name, validate_name};
use crate::tab::LoadState;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

impl Explorer {
    /// Deletes the entries at `rows` (indices into `active().order()`) in the background.
    /// `permanent = false` moves them to the OS trash; `true` removes them for good, never following symlinks.
    /// Out-of-range rows are ignored.
    pub fn delete(&mut self, rows: &[usize], permanent: bool) {
        let mut rows = rows.to_vec();
        rows.sort_unstable();
        rows.dedup();
        let paths: Vec<PathBuf> = rows
            .iter()
            .filter_map(|&row| self.entry_path(row))
            .collect();
        self.delete_paths(paths, permanent);
    }

    /// Like `delete`, for paths that are not rows of the active listing (e.g. folders in the tree).
    pub fn delete_paths(&mut self, paths: Vec<PathBuf>, permanent: bool) {
        if paths.is_empty() {
            return self.notify(NoticeKind::Error, crate::t("notice.nothing_to_delete"));
        }
        self.submit(Job::Delete { paths, permanent });
    }

    /// Renames the entry at `row` in place. The name must be a single valid component and must not exist yet.
    pub fn rename(&mut self, row: usize, new_name: &str) {
        if let Err(err) = validate_name(new_name) {
            return self.notify(NoticeKind::Error, err.to_string());
        }
        let Some(from) = self.entry_path(row) else {
            return self.notify(NoticeKind::Error, crate::t("notice.rename_missing"));
        };
        let Some(to) = from.parent().map(|dir| dir.join(new_name)) else {
            return self.notify(NoticeKind::Error, crate::t("notice.rename_impossible"));
        };
        self.submit(Job::Rename { from, to });
    }

    /// Creates a folder in the active tab's directory and selects it once the listing reloads.
    pub fn create_folder(&mut self, name: &str) {
        if let Err(err) = validate_name(name) {
            return self.notify(NoticeKind::Error, err.to_string());
        }
        let path = self.active().path().join(name);
        self.submit(Job::CreateFolder { path });
    }

    /// "New folder", or "New folder (2)" etc. if that name is taken in the active tab's listing.
    /// Reads the shown listing only, so it never touches the filesystem; a clash with an entry created
    /// since the last load is reported by the create job.
    pub fn suggest_folder_name(&self) -> String {
        let taken = self.listed_names();
        unique_name(crate::t("cmd.new_folder"), |name| {
            !taken.contains(&fold_case(name))
        })
    }

    fn listed_names(&self) -> HashSet<String> {
        let LoadState::Loaded(view) = self.active().state() else {
            return HashSet::new();
        };
        let listing = &view.listing;
        listing
            .entries
            .iter()
            .map(|entry| fold_case(&listing.name(entry).to_string_lossy()))
            .collect()
    }

    /// True while a file operation or transfer is running.
    pub fn busy(&self) -> bool {
        self.job_in_flight || self.transfers.active()
    }

    fn submit(&mut self, job: Job) {
        if self.job_in_flight {
            return self.notify(NoticeKind::Error, crate::t("notice.busy"));
        }
        self.job_in_flight = true;
        let tx = self.job_tx.clone();
        let wake = Arc::clone(&self.wake);
        thread::spawn(move || {
            let outcome = job.run();
            // As with loads: a failed send means the Explorer is gone and nobody needs the result.
            let _ = tx.send(outcome);
            wake();
        });
    }

    /// Applies finished jobs. Returns true if any finished.
    pub(super) fn apply_job_outcomes(&mut self) -> bool {
        let mut changed = false;
        while let Ok(outcome) = self.job_rx.try_recv() {
            self.job_in_flight = false;
            self.apply_outcome(outcome);
            changed = true;
        }
        changed
    }

    /// Shows the notice and reloads every listing the operation touched, in all windows.
    pub(super) fn apply_outcome(&mut self, outcome: Outcome) {
        self.drop_touched_preloads(&outcome);
        self.reload_touched_trees(&outcome);
        for w in 0..self.windows.len() {
            for index in 0..self.windows[w].tabs.len() {
                self.reload_tab_after(w, index, &outcome);
            }
        }
        self.push_notice(outcome.notice);
    }

    fn drop_touched_preloads(&mut self, outcome: &Outcome) {
        for window in &mut self.windows {
            window.cancel_preload();
            window
                .preloaded
                .retain(|cached, _| !outcome.touched.iter().any(|t| overlaps(t, cached)));
        }
    }

    /// One load per path is enough: the result fills every window's slot for it.
    fn reload_touched_trees(&mut self, outcome: &Outcome) {
        let loaded = outcome
            .touched
            .iter()
            .filter(|path| {
                self.windows
                    .iter()
                    .any(|w| w.tree.children.contains_key(*path))
            })
            .cloned()
            .collect();
        self.load_trees(loaded);
    }

    /// Moves tab `index` of window `w` out of a removed folder, or reloads it if the operation touched its folder.
    fn reload_tab_after(&mut self, w: usize, index: usize, outcome: &Outcome) {
        let tab = &mut self.windows[w].tabs[index];
        let path = tab.path().to_path_buf();
        if outcome.removed.iter().any(|gone| path.starts_with(gone)) {
            tab.navigate_to(nearest_existing(&path));
            return self.start_load(w, index);
        }
        if !outcome.touched.contains(&path) {
            return;
        }
        if let Some(name) = created_name_in(outcome, &path) {
            tab.select_after_load(name);
        }
        self.start_load(w, index);
    }
}

/// Windows and macOS volumes are usually case-insensitive, so "new folder" clashes with "New folder" there.
fn fold_case(name: &str) -> String {
    if cfg!(any(windows, target_os = "macos")) {
        name.to_lowercase()
    } else {
        name.to_owned()
    }
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

fn nearest_existing(path: &Path) -> PathBuf {
    path.ancestors()
        .find(|a| a.exists())
        .unwrap_or(path)
        .to_path_buf()
}

/// The name of the entry the operation created directly inside `dir`.
fn created_name_in(outcome: &Outcome, dir: &Path) -> Option<OsString> {
    let created = outcome.created.as_ref()?;
    if created.parent() != Some(dir) {
        return None;
    }
    created.file_name().map(OsStr::to_os_string)
}
