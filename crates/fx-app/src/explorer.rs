//! Owns the tabs and the background loader. Navigation goes through here because only Explorer can start loads.

use crate::ops::{
    EntryDetails, Job, Notice, NoticeKind, Outcome, details, unique_name, validate_name,
};
use crate::settings::Settings;
use crate::tab::{LoadState, Tab};
use crate::tree::{Children, FolderTree, TreeNode, list_children};
use crate::view::DirView;
use fx_core::{ListError, ListOptions, list_dir};
use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

#[cfg(target_os = "windows")]
const OPENER: &str = "explorer";
#[cfg(target_os = "macos")]
const OPENER: &str = "open";
#[cfg(all(unix, not(target_os = "macos")))]
const OPENER: &str = "xdg-open";

struct Completed {
    tab_id: u64,
    generation: u64,
    result: Result<DirView, ListError>,
}

struct TreeLoaded {
    generation: u64,
    path: PathBuf,
    result: Result<Vec<TreeNode>, String>,
}

pub struct Explorer {
    tabs: Vec<Tab>,
    active: usize,
    next_tab_id: u64,
    tx: Sender<Completed>,
    rx: Receiver<Completed>,
    job_tx: Sender<Outcome>,
    job_rx: Receiver<Outcome>,
    /// One file operation at a time: a second request is refused while this is set.
    // ponytail: serial jobs; add a queue if users need to chain operations while one runs
    job_in_flight: bool,
    tree: FolderTree,
    tree_tx: Sender<TreeLoaded>,
    tree_rx: Receiver<TreeLoaded>,
    /// Bumped when cached children become invalid wholesale; older tree results are dropped.
    tree_generation: u64,
    wake: Arc<dyn Fn() + Send + Sync>,
    settings: Settings,
    /// Where `set_settings` persists. `None` when the platform has no config directory.
    settings_path: Option<PathBuf>,
    notices: VecDeque<Notice>,
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
        let (tx, rx) = mpsc::channel();
        let (job_tx, job_rx) = mpsc::channel();
        let (tree_tx, tree_rx) = mpsc::channel();
        let mut explorer = Self {
            tabs: Vec::new(),
            active: 0,
            next_tab_id: 0,
            tx,
            rx,
            job_tx,
            job_rx,
            job_in_flight: false,
            tree: FolderTree::default(),
            tree_tx,
            tree_rx,
            tree_generation: 0,
            wake: Arc::new(wake),
            settings,
            settings_path: Settings::path(),
            notices: VecDeque::new(),
        };
        explorer.new_tab(start);
        explorer
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Applies `show_hidden` to every tab (selections survive by name) and persists the settings.
    /// A save failure becomes an error notice; the new settings stay in effect for this session.
    pub fn set_settings(&mut self, settings: Settings) {
        let hidden_changed = settings.show_hidden != self.settings.show_hidden;
        for tab in &mut self.tabs {
            tab.set_show_hidden(settings.show_hidden);
        }
        self.settings = settings;
        if hidden_changed {
            self.tree_generation += 1;
            let cached: Vec<PathBuf> = self.tree.children.keys().cloned().collect();
            for path in cached {
                self.load_tree(path);
            }
        }
        let saved = match &self.settings_path {
            Some(path) => self.settings.save_to(path),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no config directory available",
            )),
        };
        if let Err(err) = saved {
            self.notify(NoticeKind::Error, format!("Could not save settings: {err}"));
        }
    }

    pub fn tree(&self) -> &FolderTree {
        &self.tree
    }

    /// Expands a collapsed folder (loading its subfolders if needed) or collapses an expanded one. The cache is kept.
    pub fn tree_toggle(&mut self, path: PathBuf) {
        if !self.tree.expanded.remove(&path) {
            self.tree_expand(path);
        }
    }

    /// Expands `path`, loading its subfolders unless they are cached. A failed load is retried.
    pub fn tree_expand(&mut self, path: PathBuf) {
        if !matches!(
            self.tree.children.get(&path),
            Some(Children::Loading | Children::Loaded(_))
        ) {
            self.tree.children.insert(path.clone(), Children::Loading);
            self.load_tree(path.clone());
        }
        self.tree.expanded.insert(path);
    }

    /// Expands `root` and every folder between it and `path` (not `path` itself), where `root` is the deepest of `roots` containing `path`.
    /// The GUI calls this when the active tab's path changes.
    pub fn tree_reveal(&mut self, path: &Path, roots: &[PathBuf]) {
        let Some(root) = roots
            .iter()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
        else {
            return;
        };
        for dir in path.ancestors().skip(1) {
            if !dir.starts_with(root) {
                break;
            }
            self.tree_expand(dir.to_path_buf());
        }
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    /// Out-of-range indices are ignored.
    pub fn set_active(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    /// Opens a tab at `path`, makes it active and starts loading it.
    pub fn new_tab(&mut self, path: PathBuf) {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs
            .push(Tab::new(id, absolute(path), self.settings.show_hidden));
        self.active = self.tabs.len() - 1;
        self.start_load(self.active);
    }

    /// The last remaining tab is never closed. Out-of-range indices are ignored.
    pub fn close_tab(&mut self, index: usize) {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if index < self.active {
            self.active -= 1;
        } else {
            self.active = self.active.min(self.tabs.len() - 1);
        }
    }

    /// Navigates the active tab, recording history.
    pub fn navigate(&mut self, path: PathBuf) {
        let index = self.active;
        self.tabs[index].navigate_to(absolute(path));
        self.start_load(index);
    }

    pub fn back(&mut self) {
        let index = self.active;
        if self.tabs[index].go_back() {
            self.start_load(index);
        }
    }

    pub fn forward(&mut self) {
        let index = self.active;
        if self.tabs[index].go_forward() {
            self.start_load(index);
        }
    }

    pub fn up(&mut self) {
        let Some(parent) = self.active().path().parent().map(Path::to_path_buf) else {
            return;
        };
        self.navigate(parent);
    }

    pub fn refresh(&mut self) {
        let index = self.active;
        self.start_load(index);
    }

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
        if std::fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            self.navigate(path);
        } else if let Err(err) = open_with_system(&path) {
            eprintln!("fx: cannot open {}: {err}", path.display());
        }
    }

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
        if paths.is_empty() {
            return self.notify(NoticeKind::Error, "Nothing selected to delete");
        }
        self.submit(Job::Delete { paths, permanent });
    }

    /// Renames the entry at `row` in place. The name must be a single valid component and must not exist yet.
    pub fn rename(&mut self, row: usize, new_name: &str) {
        if let Err(err) = validate_name(new_name) {
            return self.notify(NoticeKind::Error, err.to_string());
        }
        let Some(from) = self.entry_path(row) else {
            return self.notify(NoticeKind::Error, "No such item to rename");
        };
        let Some(to) = from.parent().map(|dir| dir.join(new_name)) else {
            return self.notify(NoticeKind::Error, "This item cannot be renamed");
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

    /// "New folder", or "New folder (2)" etc. if that name is taken in the active tab's directory.
    pub fn suggest_folder_name(&self) -> String {
        let dir = self.active().path();
        unique_name("New folder", |name| {
            fs::symlink_metadata(dir.join(name)).is_err()
        })
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

    /// True while a file operation is running.
    pub fn busy(&self) -> bool {
        self.job_in_flight
    }

    /// The oldest pending notice, if any.
    pub fn take_notice(&mut self) -> Option<Notice> {
        self.notices.pop_front()
    }

    /// Applies finished loads and file operations. Results for closed tabs or superseded requests are dropped.
    /// Returns true if any tab or notice changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(done) = self.rx.try_recv() {
            let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id() == done.tab_id) else {
                continue;
            };
            changed |= tab.apply_load(done.generation, done.result.map_err(|e| e.to_string()));
        }
        while let Ok(done) = self.tree_rx.try_recv() {
            if done.generation != self.tree_generation {
                continue;
            }
            if let Some(slot) = self.tree.children.get_mut(&done.path) {
                *slot = match done.result {
                    Ok(nodes) => Children::Loaded(nodes),
                    Err(err) => Children::Failed(err),
                };
                changed = true;
            }
        }
        while let Ok(outcome) = self.job_rx.try_recv() {
            self.finish_job(outcome);
            changed = true;
        }
        changed
    }

    fn finish_job(&mut self, outcome: Outcome) {
        self.job_in_flight = false;
        self.notices.push_back(outcome.notice);
        for path in &outcome.touched {
            if self.tree.children.contains_key(path) {
                self.load_tree(path.clone());
            }
        }
        for index in 0..self.tabs.len() {
            let path = self.tabs[index].path().to_path_buf();
            if !outcome.touched.contains(&path) {
                continue;
            }
            if let Some(created) = &outcome.created
                && created.parent() == Some(path.as_path())
                && let Some(name) = created.file_name()
            {
                self.tabs[index].select_after_load(name.to_os_string());
            }
            self.start_load(index);
        }
    }

    fn submit(&mut self, job: Job) {
        if self.job_in_flight {
            return self.notify(NoticeKind::Error, "Another file operation is still running");
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

    fn notify(&mut self, kind: NoticeKind, text: impl Into<String>) {
        self.notices.push_back(Notice {
            kind,
            text: text.into(),
        });
    }

    fn load_tree(&self, path: PathBuf) {
        let generation = self.tree_generation;
        let show_hidden = self.settings.show_hidden;
        let tx = self.tree_tx.clone();
        let wake = Arc::clone(&self.wake);
        thread::spawn(move || {
            let result = list_children(&path, show_hidden);
            let _ = tx.send(TreeLoaded {
                generation,
                path,
                result,
            });
            wake();
        });
    }

    fn start_load(&mut self, index: usize) {
        let tab = &mut self.tabs[index];
        let generation = tab.begin_load();
        let tab_id = tab.id();
        let path = tab.path().to_path_buf();
        let tx = self.tx.clone();
        let wake = Arc::clone(&self.wake);
        thread::spawn(move || {
            let started = Instant::now();
            let result = list_dir(&path, &list_options())
                .map(|listed| DirView::new(path, listed, started.elapsed()));
            // A send only fails once the Explorer is gone, and then nobody needs the result.
            let _ = tx.send(Completed {
                tab_id,
                generation,
                result,
            });
            wake();
        });
    }
}

/// Listing is sorted by the view model, so the backend's own sort is skipped.
fn list_options() -> ListOptions {
    ListOptions {
        sort: false,
        ..ListOptions::default()
    }
}

/// Falls back to the path as given when the working directory is unavailable; the load then reports the real error.
fn absolute(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}

/// Spawns the OS opener and does not wait for it. The child is not reaped; a waiter thread would be needed if opens become frequent.
// ponytail: unreaped child per open; add a reaper thread if opens get frequent
fn open_with_system(path: &Path) -> io::Result<()> {
    Command::new(OPENER)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|mut child| {
            // Reap the child so it does not linger as a zombie.
            thread::spawn(move || child.wait());
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::SortColumn;
    use fx_core::EntryKind;
    use std::time::Duration;

    /// Waits until no file operation runs and no tab is loading.
    fn settle(explorer: &mut Explorer) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while explorer.busy()
            || explorer
                .tree
                .children
                .values()
                .any(|c| matches!(c, Children::Loading))
            || explorer
                .tabs()
                .iter()
                .any(|tab| matches!(tab.state(), LoadState::Loading))
        {
            assert!(Instant::now() < deadline, "work did not finish in time");
            explorer.poll();
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn names(explorer: &Explorer) -> Vec<String> {
        let tab = explorer.active();
        let LoadState::Loaded(view) = tab.state() else {
            panic!("tab is not loaded: {:?}", tab.state());
        };
        tab.order()
            .iter()
            .map(|&i| {
                view.listing
                    .name(&view.listing.entries[i as usize])
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    fn row_of(explorer: &Explorer, name: &str) -> usize {
        names(explorer)
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| panic!("no row named {name}"))
    }

    fn selected_names(explorer: &Explorer) -> Vec<String> {
        let all = names(explorer);
        explorer
            .active()
            .selection()
            .into_iter()
            .map(|row| all[row].clone())
            .collect()
    }

    fn last_notice(explorer: &mut Explorer) -> Notice {
        let mut last = None;
        while let Some(notice) = explorer.take_notice() {
            last = Some(notice);
        }
        last.expect("expected a notice")
    }

    fn explorer_in(root: &Path) -> Explorer {
        let mut explorer = Explorer::new(root.to_path_buf(), || {});
        settle(&mut explorer);
        explorer
    }

    #[test]
    fn navigates_sorts_filters_and_manages_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::create_dir(root.join("zeta")).unwrap();
        fs::create_dir(root.join("alpha")).unwrap();
        fs::write(root.join("big.bin"), vec![0u8; 2048]).unwrap();
        fs::write(root.join("small.txt"), b"x").unwrap();
        fs::write(root.join(".hidden"), b"").unwrap();
        fs::write(root.join("alpha").join("inner.md"), b"").unwrap();

        let mut explorer = Explorer::new(root.clone(), || {});
        settle(&mut explorer);
        assert_eq!(names(&explorer), ["alpha", "zeta", "big.bin", "small.txt"]);

        explorer.active_mut().set_sort_column(SortColumn::Size);
        assert_eq!(names(&explorer)[2..], ["small.txt", "big.bin"]);
        explorer.active_mut().set_sort_column(SortColumn::Size);
        assert_eq!(names(&explorer)[2..], ["big.bin", "small.txt"]);
        explorer.active_mut().set_sort(crate::SortSpec {
            column: SortColumn::Size,
            ascending: true,
        });
        assert_eq!(names(&explorer)[2..], ["small.txt", "big.bin"]);

        explorer.navigate(root.join("alpha"));
        settle(&mut explorer);
        assert_eq!(names(&explorer), ["inner.md"]);

        explorer.back();
        settle(&mut explorer);
        assert_eq!(explorer.active().path(), root);
        explorer.forward();
        settle(&mut explorer);
        assert_eq!(explorer.active().path(), root.join("alpha"));
        explorer.up();
        settle(&mut explorer);
        assert_eq!(explorer.active().path(), root);

        explorer.active_mut().set_filter("SMALL");
        assert_eq!(names(&explorer), ["small.txt"]);
        explorer.active_mut().set_filter("");

        assert!(!names(&explorer).contains(&".hidden".to_owned()));
        explorer.active_mut().set_show_hidden(true);
        assert!(names(&explorer).contains(&".hidden".to_owned()));

        explorer.new_tab(root.clone());
        assert_eq!(explorer.tabs().len(), 2);
        assert_eq!(explorer.active_index(), 1);
        explorer.close_tab(0);
        assert_eq!(explorer.tabs().len(), 1);
        explorer.close_tab(0);
        assert_eq!(explorer.tabs().len(), 1, "the last tab is never closed");
    }

    #[test]
    fn rename_validates_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), b"a").unwrap();
        fs::write(root.join("b.txt"), b"b").unwrap();
        let mut explorer = explorer_in(root);
        let a = row_of(&explorer, "a.txt");

        for bad in ["", "   ", "x/y", ".", ".."] {
            explorer.rename(a, bad);
            let notice = last_notice(&mut explorer);
            assert_eq!(notice.kind, NoticeKind::Error, "name {bad:?}");
        }
        assert!(root.join("a.txt").exists());

        explorer.rename(a, "b.txt");
        settle(&mut explorer);
        assert_eq!(last_notice(&mut explorer).kind, NoticeKind::Error);
        assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"a");
        assert_eq!(fs::read(root.join("b.txt")).unwrap(), b"b");

        explorer.rename(a, "c.txt");
        settle(&mut explorer);
        assert_eq!(last_notice(&mut explorer).kind, NoticeKind::Info);
        assert!(!root.join("a.txt").exists());
        assert_eq!(fs::read(root.join("c.txt")).unwrap(), b"a");
        assert!(names(&explorer).contains(&"c.txt".to_owned()));
    }

    #[test]
    fn suggests_a_free_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut explorer = explorer_in(dir.path());
        assert_eq!(explorer.suggest_folder_name(), "New folder");
        explorer.create_folder("New folder");
        settle(&mut explorer);
        assert_eq!(explorer.suggest_folder_name(), "New folder (2)");
    }

    #[test]
    fn create_folder_selects_the_new_folder_after_reload() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("file.txt"), b"").unwrap();
        let mut explorer = explorer_in(root);

        explorer.create_folder("a/b");
        assert_eq!(last_notice(&mut explorer).kind, NoticeKind::Error);

        explorer.create_folder("New");
        settle(&mut explorer);
        assert!(root.join("New").is_dir());
        assert_eq!(last_notice(&mut explorer).kind, NoticeKind::Info);
        assert_eq!(selected_names(&explorer), ["New"]);

        explorer.create_folder("New");
        settle(&mut explorer);
        assert_eq!(last_notice(&mut explorer).kind, NoticeKind::Error);
    }

    #[test]
    fn permanent_delete_removes_files_and_trees() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("file.txt"), b"x").unwrap();
        fs::create_dir_all(root.join("tree").join("deep")).unwrap();
        fs::write(root.join("tree").join("deep").join("leaf"), b"x").unwrap();
        let mut explorer = explorer_in(root);

        explorer.delete(&[row_of(&explorer, "file.txt")], true);
        settle(&mut explorer);
        assert_eq!(last_notice(&mut explorer).text, "Deleted 1 item");
        assert!(!root.join("file.txt").exists());

        explorer.delete(&[row_of(&explorer, "tree")], true);
        settle(&mut explorer);
        assert!(!root.join("tree").exists());
        assert!(names(&explorer).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn permanent_delete_of_a_symlink_keeps_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("target")).unwrap();
        fs::write(root.join("target").join("precious.txt"), b"keep").unwrap();
        std::os::unix::fs::symlink(root.join("target"), root.join("link")).unwrap();
        let mut explorer = explorer_in(root);

        explorer.delete(&[row_of(&explorer, "link")], true);
        settle(&mut explorer);
        assert!(
            fs::symlink_metadata(root.join("link")).is_err(),
            "link is gone"
        );
        assert_eq!(
            fs::read(root.join("target").join("precious.txt")).unwrap(),
            b"keep",
            "target survives"
        );
    }

    #[test]
    fn multi_delete_continues_past_a_failing_item() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for name in ["one", "gone", "two"] {
            fs::write(root.join(name), b"x").unwrap();
        }
        let mut explorer = explorer_in(root);
        let rows = [
            row_of(&explorer, "one"),
            row_of(&explorer, "gone"),
            row_of(&explorer, "two"),
        ];
        // Vanishes after the listing, so its delete fails.
        fs::remove_file(root.join("gone")).unwrap();

        explorer.delete(&rows, true);
        settle(&mut explorer);
        let notice = last_notice(&mut explorer);
        assert_eq!(notice.kind, NoticeKind::Error);
        assert!(
            notice.text.starts_with("Deleted 2 of 3 items."),
            "{}",
            notice.text
        );
        assert!(
            notice.text.contains("Could not delete gone"),
            "{}",
            notice.text
        );
        assert!(!root.join("one").exists());
        assert!(!root.join("two").exists());
    }

    #[test]
    fn delete_ignores_out_of_range_rows_and_refuses_while_busy() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("keep"), b"x").unwrap();
        let mut explorer = explorer_in(root);

        explorer.delete(&[99], true);
        assert_eq!(last_notice(&mut explorer).kind, NoticeKind::Error);
        assert!(root.join("keep").exists());

        explorer.delete(&[0], true);
        explorer.create_folder("second");
        assert!(explorer.busy());
        assert_eq!(
            last_notice(&mut explorer).text,
            "Another file operation is still running"
        );
        settle(&mut explorer);
        assert!(!root.join("keep").exists());
        assert!(!root.join("second").exists(), "refused create is not run");
    }

    /// Uses the real OS trash, so it leaves the file in the user's trash on purpose.
    #[test]
    #[ignore = "writes into the real OS trash; run with --ignored"]
    fn trash_delete_moves_the_file_away() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("bin.txt"), b"x").unwrap();
        let mut explorer = explorer_in(root);

        explorer.delete(&[row_of(&explorer, "bin.txt")], false);
        settle(&mut explorer);
        let notice = last_notice(&mut explorer);
        assert_eq!(notice.text, "Moved 1 item to trash");
        assert!(!root.join("bin.txt").exists());
    }

    #[test]
    fn click_builds_windows_style_selection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for name in ["a", "b", "c", "d", "e"] {
            fs::write(root.join(name), b"").unwrap();
        }
        let mut explorer = explorer_in(root);
        let tab = explorer.active_mut();

        tab.click(1, false, false);
        assert_eq!(tab.selection(), [1]);
        tab.click(3, true, false);
        assert_eq!(tab.selection(), [1, 3]);
        assert_eq!(tab.selected(), Some(3), "ctrl-click moves the focus");
        tab.click(3, true, false);
        assert_eq!(tab.selection(), [1], "ctrl-click toggles off");
        tab.click(4, false, true);
        assert_eq!(
            tab.selection(),
            [3, 4],
            "shift range from the anchor (row 3)"
        );
        assert_eq!(tab.selected(), Some(3), "shift keeps the anchor");
        tab.click(1, true, true);
        assert_eq!(tab.selection(), [1, 2, 3, 4], "ctrl+shift adds the range");
        tab.click(0, false, false);
        assert_eq!(tab.selection(), [0]);
        assert!(tab.is_selected(0) && !tab.is_selected(1));

        tab.select_all();
        assert_eq!(tab.selection(), [0, 1, 2, 3, 4]);
        tab.click(99, false, false);
        assert_eq!(tab.selection().len(), 5, "out-of-range clicks are ignored");
        tab.select(Some(2));
        assert_eq!(tab.selection(), [2]);
        tab.move_selection(1);
        assert_eq!(tab.selection(), [3], "keyboard moves are single-select");
    }

    #[test]
    fn selection_survives_resort_and_refresh_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), b"1").unwrap();
        fs::write(root.join("b.txt"), b"22").unwrap();
        fs::write(root.join("c.txt"), b"333").unwrap();
        let mut explorer = explorer_in(root);
        explorer.active_mut().click(0, false, false);
        explorer.active_mut().click(2, true, false);
        assert_eq!(selected_names(&explorer), ["a.txt", "c.txt"]);

        explorer.active_mut().set_sort_column(SortColumn::Size);
        assert_eq!(selected_names(&explorer), ["a.txt", "c.txt"]);

        explorer.refresh();
        settle(&mut explorer);
        assert_eq!(selected_names(&explorer), ["a.txt", "c.txt"]);
    }

    #[test]
    fn entry_details_describe_files_dirs_and_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("file.txt"), b"12345").unwrap();
        fs::create_dir(root.join("folder")).unwrap();
        let explorer = explorer_in(root);

        let file = explorer
            .entry_details(row_of(&explorer, "file.txt"))
            .unwrap();
        assert_eq!(file.name, "file.txt");
        assert_eq!(file.path, root.join("file.txt"));
        assert_eq!(file.kind, EntryKind::File);
        assert_eq!(file.size, Some(5));
        assert!(file.modified.is_some());

        let folder = explorer.entry_details(row_of(&explorer, "folder")).unwrap();
        assert_eq!(folder.kind, EntryKind::Dir);
        assert_eq!(folder.size, None, "directories are not walked");
        assert!(explorer.entry_details(99).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn entry_details_show_unix_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("locked.txt");
        fs::write(&file, b"").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
        let explorer = explorer_in(root);

        let details = explorer
            .entry_details(row_of(&explorer, "locked.txt"))
            .unwrap();
        assert_eq!(details.permissions, "rw-r-----");
    }

    #[test]
    fn open_in_new_tab_only_for_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("file.txt"), b"").unwrap();
        fs::create_dir(root.join("folder")).unwrap();
        let mut explorer = explorer_in(root);

        explorer.open_in_new_tab(row_of(&explorer, "file.txt"));
        assert_eq!(explorer.tabs().len(), 1);
        explorer.open_in_new_tab(row_of(&explorer, "folder"));
        assert_eq!(explorer.tabs().len(), 2);
        assert_eq!(explorer.active().path(), root.join("folder"));
    }

    #[test]
    fn settings_apply_to_tabs_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".hidden"), b"").unwrap();
        let mut explorer = Explorer::with_settings(root.to_path_buf(), Settings::default(), || {});
        explorer.settings_path = Some(root.join("cfg").join("settings.toml"));
        settle(&mut explorer);
        assert!(!names(&explorer).contains(&".hidden".to_owned()));

        let updated = Settings {
            show_hidden: true,
            ..Settings::default()
        };
        explorer.set_settings(updated.clone());
        assert!(names(&explorer).contains(&".hidden".to_owned()));
        assert_eq!(explorer.settings(), &updated);
        assert_eq!(
            Settings::load_from(&root.join("cfg").join("settings.toml")),
            updated
        );

        explorer.new_tab(root.to_path_buf());
        settle(&mut explorer);
        assert!(
            names(&explorer).contains(&".hidden".to_owned()),
            "new tabs inherit it"
        );
    }

    #[test]
    fn settings_save_failure_becomes_an_error_notice() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let blocker = root.join("not-a-dir");
        fs::write(&blocker, b"").unwrap();
        let mut explorer = explorer_in(root);
        explorer.settings_path = Some(blocker.join("settings.toml"));

        explorer.set_settings(Settings {
            show_hidden: true,
            ..Settings::default()
        });
        let notice = last_notice(&mut explorer);
        assert_eq!(notice.kind, NoticeKind::Error);
        assert!(
            notice.text.starts_with("Could not save settings"),
            "{}",
            notice.text
        );
    }

    fn child_names(explorer: &Explorer, dir: &Path) -> Vec<String> {
        match explorer.tree().children(dir) {
            Some(Children::Loaded(nodes)) => nodes.iter().map(|n| n.name.clone()).collect(),
            other => panic!("not loaded: {other:?}"),
        }
    }

    #[test]
    fn tree_toggle_loads_sorted_folders_only_and_keeps_cache() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for name in ["d10", "d2", "Alpha", ".hid"] {
            fs::create_dir(root.join(name)).unwrap();
        }
        fs::write(root.join("file.txt"), b"").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("d2"), root.join("linkdir")).unwrap();
            std::os::unix::fs::symlink(root.join("file.txt"), root.join("linkfile")).unwrap();
        }
        let mut explorer = explorer_in(root);

        assert!(explorer.tree().children(root).is_none());
        explorer.tree_toggle(root.to_path_buf());
        assert!(explorer.tree().is_expanded(root));
        assert_eq!(explorer.tree().children(root), Some(&Children::Loading));
        settle(&mut explorer);
        #[cfg(unix)]
        assert_eq!(
            child_names(&explorer, root),
            ["Alpha", "d2", "d10", "linkdir"]
        );
        #[cfg(not(unix))]
        assert_eq!(child_names(&explorer, root), ["Alpha", "d2", "d10"]);

        explorer.tree_toggle(root.to_path_buf());
        assert!(!explorer.tree().is_expanded(root));
        assert!(matches!(
            explorer.tree().children(root),
            Some(Children::Loaded(_))
        ));

        explorer.tree_toggle(root.join("missing"));
        settle(&mut explorer);
        assert!(matches!(
            explorer.tree().children(&root.join("missing")),
            Some(Children::Failed(_))
        ));
    }

    #[test]
    fn tree_show_hidden_reloads_cached_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join(".hid")).unwrap();
        fs::create_dir(root.join("vis")).unwrap();
        let mut explorer = explorer_in(root);
        let cfg = tempfile::tempdir().unwrap();
        explorer.settings_path = Some(cfg.path().join("settings.toml"));
        explorer.tree_expand(root.to_path_buf());
        settle(&mut explorer);
        assert_eq!(child_names(&explorer, root), ["vis"]);

        explorer.set_settings(Settings {
            show_hidden: true,
            ..Settings::default()
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while child_names(&explorer, root) != [".hid", "vis"] {
            assert!(
                Instant::now() < deadline,
                "reload did not finish in time: {:?}",
                explorer.tree().children(root)
            );
            explorer.poll();
            thread::sleep(Duration::from_millis(1));
        }
        assert!(explorer.tree().is_expanded(root));
    }

    #[test]
    fn tree_reveal_expands_ancestors_below_the_deepest_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let deep = root.join("a").join("b").join("c");
        fs::create_dir_all(&deep).unwrap();
        fs::create_dir_all(root.join("a").join("sibling")).unwrap();
        let mut explorer = explorer_in(root);

        let roots = [root.to_path_buf(), root.join("a")];
        explorer.tree_reveal(&deep, &roots);
        settle(&mut explorer);
        assert!(explorer.tree().is_expanded(&root.join("a")));
        assert!(explorer.tree().is_expanded(&root.join("a").join("b")));
        assert!(!explorer.tree().is_expanded(&deep));
        assert!(!explorer.tree().is_expanded(root), "above the deepest root");
        assert!(
            !explorer.tree().is_expanded(&root.join("a").join("sibling")),
            "siblings of the path stay collapsed"
        );
        assert_eq!(child_names(&explorer, &root.join("a").join("b")), ["c"]);

        explorer.tree_reveal(root, &[]);
        assert!(!explorer.tree().is_expanded(root), "no root, no change");
    }

    #[test]
    fn tree_reloads_after_a_folder_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("one")).unwrap();
        let mut explorer = explorer_in(root);
        explorer.tree_expand(root.to_path_buf());
        settle(&mut explorer);
        assert_eq!(child_names(&explorer, root), ["one"]);

        explorer.create_folder("two");
        settle(&mut explorer);
        let deadline = Instant::now() + Duration::from_secs(10);
        while child_names(&explorer, root) != ["one", "two"] {
            assert!(
                Instant::now() < deadline,
                "reload did not finish in time: {:?}",
                explorer.tree().children(root)
            );
            explorer.poll();
            thread::sleep(Duration::from_millis(1));
        }
        assert!(explorer.tree().is_expanded(root));
    }
}
