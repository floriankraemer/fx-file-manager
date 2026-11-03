//! Owns the tabs and the background loader. Navigation goes through here because only Explorer can start loads.

use crate::tab::{LoadState, Tab};
use crate::view::DirView;
use fx_core::{ListError, ListOptions, list_dir};
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

pub struct Explorer {
    tabs: Vec<Tab>,
    active: usize,
    next_tab_id: u64,
    tx: Sender<Completed>,
    rx: Receiver<Completed>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Explorer {
    /// `wake` is called from loader threads when a result is ready; a UI passes `ctx.request_repaint`.
    pub fn new(start: PathBuf, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut explorer = Self {
            tabs: Vec::new(),
            active: 0,
            next_tab_id: 0,
            tx,
            rx,
            wake: Arc::new(wake),
        };
        explorer.new_tab(start);
        explorer
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
        self.tabs.push(Tab::new(id, absolute(path)));
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

    /// Applies finished loads. Results for closed tabs or superseded requests are dropped.
    /// Returns true if any tab changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(done) = self.rx.try_recv() {
            let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id() == done.tab_id) else {
                continue;
            };
            changed |= tab.apply_load(done.generation, done.result.map_err(|e| e.to_string()));
        }
        changed
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
    use std::fs;
    use std::time::Duration;

    fn settle(explorer: &mut Explorer) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while matches!(explorer.active().state(), LoadState::Loading) {
            assert!(Instant::now() < deadline, "listing did not finish in time");
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
}
