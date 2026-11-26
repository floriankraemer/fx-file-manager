//! Background thumbnail generation for the Icons view.
//!
//! The view asks each frame for the thumbnails it shows and lacks, visible ones first. That list replaces
//! the queue, so entries scrolled out of view are never produced. Running jobs finish; their results stay
//! useful because the view caches them.

use fx_thumbs::{Request, Thumb, Thumbnailer};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::SystemTime;

/// Identifies one thumbnail: a changed file has another mtime or size, so its old thumbnail is never reused.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThumbKey {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub size: u64,
    /// Edge length in physical pixels, already snapped to a thumbnail bucket.
    pub px: u32,
}

/// More would only contend for the disk; decoding is a fraction of the cost of reading on any storage.
pub const MAX_WORKERS: usize = 4;

#[derive(Default)]
struct Queue {
    pending: VecDeque<ThumbKey>,
    running: HashSet<ThumbKey>,
    /// Workers allowed to run at once, set per request from the folder's storage.
    limit: usize,
    shutdown: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }
}

type Done = (ThumbKey, Option<Arc<Thumb>>);

pub struct Thumbnails {
    shared: Arc<Shared>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    workers: usize,
    /// Keys that produced no thumbnail; asked for again only once the file changes.
    failed: HashSet<ThumbKey>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Thumbnails {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue::default()),
                ready: Condvar::new(),
            }),
            tx,
            rx,
            workers: 0,
            failed: HashSet::new(),
            wake,
        }
    }

    /// Replaces the queue with `keys`, in that order, using at most `workers` threads.
    /// Keys that failed before or are being produced right now are skipped.
    pub fn want(&mut self, keys: Vec<ThumbKey>, workers: usize) {
        let workers = workers.clamp(1, MAX_WORKERS);
        {
            let mut queue = self.shared.lock();
            let skip = |k: &ThumbKey| self.failed.contains(k) || queue.running.contains(k);
            let pending: VecDeque<ThumbKey> = keys.into_iter().filter(|k| !skip(k)).collect();
            queue.pending = pending;
            queue.limit = workers;
        }
        self.shared.ready.notify_all();
        while self.workers < workers {
            self.spawn_worker();
        }
    }

    /// Thumbnails finished since the last call. Failures are remembered instead of returned.
    pub fn take(&mut self) -> Vec<(ThumbKey, Arc<Thumb>)> {
        let mut ready = Vec::new();
        while let Ok((key, thumb)) = self.rx.try_recv() {
            match thumb {
                Some(thumb) => ready.push((key, thumb)),
                None => {
                    self.failed.insert(key);
                }
            }
        }
        ready
    }

    fn spawn_worker(&mut self) {
        let shared = Arc::clone(&self.shared);
        let tx = self.tx.clone();
        let wake = Arc::clone(&self.wake);
        self.workers += 1;
        let spawned = thread::Builder::new()
            .name("fx-thumbs".into())
            .spawn(move || work(&shared, &tx, &*wake));
        if let Err(e) = spawned {
            log::warn!("cannot start a thumbnail worker: {e}");
        }
    }
}

impl Drop for Thumbnails {
    fn drop(&mut self) {
        self.shared.lock().shutdown = true;
        self.shared.ready.notify_all();
    }
}

fn work(shared: &Shared, tx: &Sender<Done>, wake: &(dyn Fn() + Send + Sync)) {
    let mut thumbnailer = Thumbnailer::new();
    while let Some(key) = next(shared) {
        let request = Request {
            path: &key.path,
            modified: key.modified,
            size: key.size,
            px: key.px,
        };
        let thumb = match thumbnailer.thumbnail(&request) {
            Ok(thumb) => Some(Arc::new(thumb)),
            Err(e) => {
                log::debug!("no thumbnail for {}: {e}", key.path.display());
                None
            }
        };
        shared.lock().running.remove(&key);
        shared.ready.notify_all();
        if tx.send((key, thumb)).is_err() {
            return;
        }
        wake();
    }
}

/// Blocks until a key may run under the current limit; `None` on shutdown.
fn next(shared: &Shared) -> Option<ThumbKey> {
    let mut queue = shared.lock();
    loop {
        if queue.shutdown {
            return None;
        }
        if queue.running.len() < queue.limit
            && let Some(key) = queue.pending.pop_front()
        {
            queue.running.insert(key.clone());
            return Some(key);
        }
        queue = shared.ready.wait(queue).unwrap_or_else(|e| e.into_inner());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::{Duration, Instant};

    fn key(path: &Path) -> ThumbKey {
        let meta = std::fs::metadata(path).ok();
        ThumbKey {
            path: path.to_path_buf(),
            modified: meta.as_ref().and_then(|m| m.modified().ok()),
            size: meta.map_or(0, |m| m.len()),
            px: 128,
        }
    }

    fn png(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        image::RgbImage::from_pixel(4, 4, image::Rgb([1, 2, 3]))
            .save(&path)
            .unwrap();
        path
    }

    /// Takes results until `n` arrived or failures were recorded for the rest, or a timeout.
    fn settle(thumbs: &mut Thumbnails, n: usize) -> Vec<ThumbKey> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut got = Vec::new();
        while got.len() + thumbs.failed.len() < n && Instant::now() < deadline {
            got.extend(thumbs.take().into_iter().map(|(k, _)| k));
            thread::sleep(Duration::from_millis(5));
        }
        got
    }

    #[test]
    fn wanted_keys_are_produced_and_failures_are_not_retried() {
        let dir = tempfile::tempdir().unwrap();
        let good = key(&png(dir.path(), "a.png"));
        // Missing, so no backend can thumbnail it; QuickLook would preview a text file.
        let bad = key(&dir.path().join("gone.png"));
        let mut thumbs = Thumbnails::new(Arc::new(|| {}));
        thumbs.want(vec![good.clone(), bad.clone()], 2);
        assert_eq!(settle(&mut thumbs, 2), vec![good]);
        assert!(thumbs.failed.contains(&bad));

        thumbs.want(vec![bad], 2);
        assert!(thumbs.shared.lock().pending.is_empty());
    }

    #[test]
    fn a_new_request_replaces_what_is_still_pending() {
        let mut thumbs = Thumbnails::new(Arc::new(|| {}));
        {
            // Keep workers from taking anything while the queue is inspected.
            thumbs.shared.lock().limit = 0;
        }
        let a = key(Path::new("/nonexistent/a.png"));
        let b = key(Path::new("/nonexistent/b.png"));
        let mut queue = thumbs.shared.lock();
        queue.pending = VecDeque::from([a.clone()]);
        queue.running.insert(b.clone());
        drop(queue);
        let c = key(Path::new("/nonexistent/c.png"));
        thumbs.want(vec![b, c.clone()], 1);
        let queue = thumbs.shared.lock();
        assert!(
            !queue.pending.contains(&a),
            "scrolled-away keys are dropped"
        );
        assert!(
            queue.pending.iter().all(|k| *k == c),
            "running keys are not queued twice"
        );
    }
}
