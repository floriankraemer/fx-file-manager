//! Native change notifications for open folders: inotify on Linux, ReadDirectoryChangesW on Windows,
//! FSEvents on macOS. Every folder is watched non-recursively, so only its own entries report.

use crate::StorageKind;
use notify::{Config, Event, EventKindMask, PathOp, RecommendedWatcher, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

/// Watches a set of folders and reports which of them had entries added, removed, renamed or changed.
pub struct DirWatcher {
    tx: Sender<Vec<PathBuf>>,
}

impl DirWatcher {
    /// `changed` runs on a notifier thread with the folder whose entries changed, or `None` when
    /// the OS dropped events and every watched folder may have changed.
    pub fn new(changed: impl Fn(Option<&Path>) + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("fx-watch".into())
            .spawn(move || run(&rx, changed));
        if let Err(err) = spawned {
            log::warn!("cannot start the folder watcher: {err}");
        }
        Self { tx }
    }

    /// Replaces the watched folders. Returns at once: watches are added and removed on a worker
    /// thread, so a folder on a hung network mount cannot stall the caller.
    pub fn watch(&self, dirs: Vec<PathBuf>) {
        // Fails only when the worker could not start; the caller's mtime polling still covers changes.
        let _ = self.tx.send(dirs);
    }
}

fn run(rx: &Receiver<Vec<PathBuf>>, changed: impl Fn(Option<&Path>) + Send + 'static) {
    let config = Config::default().with_event_kinds(EventKindMask::CORE);
    let mut watcher = match RecommendedWatcher::new(move |event| report(&event, &changed), config) {
        Ok(watcher) => watcher,
        Err(err) => return log::warn!("folder change notifications unavailable: {err}"),
    };
    let mut watched = HashSet::new();
    while let Ok(mut dirs) = rx.recv() {
        // Only the newest set matters; skip the ones queued while a slow watch call ran.
        while let Ok(newer) = rx.try_recv() {
            dirs = newer;
        }
        let wanted: HashSet<PathBuf> = dirs.into_iter().filter(|d| watchable(d)).collect();
        apply(&mut watcher, diff(&watched, &wanted));
        watched = wanted;
    }
}

fn report(event: &notify::Result<Event>, changed: &impl Fn(Option<&Path>)) {
    let event = match event {
        Ok(event) => event,
        Err(err) => return log::debug!("folder watch error: {err}"),
    };
    if event.need_rescan() {
        return changed(None);
    }
    if event.kind.is_access() {
        return;
    }
    for parent in event.paths.iter().filter_map(|path| path.parent()) {
        changed(Some(parent));
    }
}

/// An open directory handle on Windows keeps a USB drive from being ejected; its mtime is still polled.
fn watchable(dir: &Path) -> bool {
    !cfg!(windows) || crate::sys::native::detect_storage(dir) != StorageKind::Removable
}

fn diff(watched: &HashSet<PathBuf>, wanted: &HashSet<PathBuf>) -> Vec<PathOp> {
    let gone = watched.difference(wanted).map(PathOp::unwatch);
    let new = wanted.difference(watched).map(PathOp::watch_non_recursive);
    gone.chain(new).collect()
}

/// Applies all `ops` in one batch (one FSEvents stream rebuild on macOS), skipping any that fail,
/// such as a folder deleted meanwhile.
fn apply(watcher: &mut RecommendedWatcher, mut ops: Vec<PathOp>) {
    while !ops.is_empty() {
        let Err(err) = watcher.update_paths(ops) else {
            return;
        };
        let Some(op) = &err.origin else {
            return log::debug!("folder watch update failed: {}", err.source);
        };
        log::debug!("cannot watch {}: {}", op.as_path().display(), err.source);
        ops = err.remaining;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(10);

    fn watcher() -> (DirWatcher, Receiver<Option<PathBuf>>) {
        let (tx, rx) = mpsc::channel();
        let watcher = DirWatcher::new(move |dir| {
            let _ = tx.send(dir.map(Path::to_path_buf));
        });
        (watcher, rx)
    }

    /// Waits until `dir` reports, ignoring other folders; the watch starts asynchronously, so
    /// `touch` is repeated until the first notification arrives.
    fn wait_for(rx: &Receiver<Option<PathBuf>>, dir: &Path, touch: impl Fn(u32)) {
        for attempt in 0..(TIMEOUT.as_millis() / 100) as u32 {
            touch(attempt);
            if reported(rx, dir) {
                return;
            }
        }
        panic!("no change reported for {}", dir.display());
    }

    /// Whether `dir`, or every folder, reports within the next 100 ms.
    fn reported(rx: &Receiver<Option<PathBuf>>, dir: &Path) -> bool {
        std::iter::from_fn(|| rx.recv_timeout(Duration::from_millis(100)).ok())
            .any(|got| got.as_deref().is_none_or(|got| same_dir(got, dir)))
    }

    /// macOS reports temp folders under /private/var while tempfile hands out /var.
    fn same_dir(a: &Path, b: &Path) -> bool {
        a == b || fs::canonicalize(a).ok() == fs::canonicalize(b).ok()
    }

    #[test]
    fn reports_created_and_removed_entries() {
        let dir = tempfile::tempdir().unwrap();
        let (watcher, rx) = watcher();
        watcher.watch(vec![dir.path().to_path_buf()]);
        wait_for(&rx, dir.path(), |n| {
            fs::write(dir.path().join(format!("{n}.txt")), b"x").unwrap();
        });
        fs::create_dir(dir.path().join("sub")).unwrap();
        wait_for(&rx, dir.path(), |_| {});
    }

    #[test]
    fn stops_reporting_unwatched_folders() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let (watcher, rx) = watcher();
        watcher.watch(vec![dir.path().to_path_buf()]);
        wait_for(&rx, dir.path(), |n| {
            fs::write(dir.path().join(format!("{n}.txt")), b"x").unwrap();
        });
        watcher.watch(vec![other.path().to_path_buf()]);
        wait_for(&rx, other.path(), |n| {
            fs::write(other.path().join(format!("{n}.txt")), b"x").unwrap();
        });
        while rx.try_recv().is_ok() {}
        fs::write(dir.path().join("late.txt"), b"x").unwrap();
        let late = rx.recv_timeout(Duration::from_millis(300));
        assert!(
            late.as_ref().map_or(true, |got| got
                .as_deref()
                .is_some_and(|got| !same_dir(got, dir.path()))),
            "unwatched folder still reported: {late:?}"
        );
    }

    #[test]
    fn a_missing_folder_does_not_block_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let (watcher, rx) = watcher();
        watcher.watch(vec![dir.path().join("missing"), dir.path().to_path_buf()]);
        wait_for(&rx, dir.path(), |n| {
            fs::write(dir.path().join(format!("{n}.txt")), b"x").unwrap();
        });
    }
}
