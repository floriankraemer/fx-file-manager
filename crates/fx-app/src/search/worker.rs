use super::{SearchEvent, SearchStatus};
use fx_core::{Hit, SearchEvent as CoreEvent, SearchQuery, WalkControl, WalkOptions, search};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// Most time one `drain` spends applying events, so a flood of hits cannot stall a UI frame.
const DRAIN_BUDGET: Duration = Duration::from_millis(4);

pub(crate) struct SearchRequest {
    pub tab_id: u64,
    pub generation: u64,
    pub root: PathBuf,
    pub query: SearchQuery,
    pub show_hidden: bool,
    pub cap: usize,
    pub control: Arc<WalkControl>,
}

/// An event tagged with the tab and the generation of the search that produced it.
pub(crate) struct SearchMsg {
    pub tab_id: u64,
    pub generation: u64,
    pub event: SearchEvent,
}

/// Starts search workers and collects what they report. One per Explorer, shared by all tabs.
pub(crate) struct SearchRunner {
    tx: Sender<SearchMsg>,
    rx: Receiver<SearchMsg>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl SearchRunner {
    pub(crate) fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, wake }
    }

    pub(crate) fn spawn(&self, request: SearchRequest) {
        let tx = self.tx.clone();
        let wake = Arc::clone(&self.wake);
        thread::spawn(move || {
            let (tab_id, generation) = (request.tab_id, request.generation);
            let send = |event| {
                // A send only fails once the Explorer is gone, and then nobody needs the result.
                let _ = tx.send(SearchMsg {
                    tab_id,
                    generation,
                    event,
                });
                wake();
            };
            run(&request, send);
        });
    }

    /// Hands pending events to `apply` until the queue is empty or the time budget is spent;
    /// in the latter case another wake is requested. Returns true if `apply` reported a change.
    pub(crate) fn drain(&self, mut apply: impl FnMut(SearchMsg) -> bool) -> bool {
        let started = Instant::now();
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            changed |= apply(msg);
            if started.elapsed() >= DRAIN_BUDGET {
                (self.wake)();
                break;
            }
        }
        changed
    }
}

fn run(request: &SearchRequest, send: impl Fn(SearchEvent) + Sync) {
    let delivery = Delivery::new(request);
    let started = Instant::now();
    let opts = WalkOptions::default();
    let walked = search(
        &request.root,
        &request.query,
        &opts,
        &request.control,
        |event| match event {
            CoreEvent::Hits(hits) => {
                let hits = delivery.admit(hits);
                if !hits.is_empty() {
                    send(SearchEvent::Hits(hits));
                }
            }
            CoreEvent::Unreadable(folder) => send(SearchEvent::Unreadable(folder)),
        },
    );
    log_end(
        &request.root,
        &request.control,
        started,
        walked.as_ref().err(),
    );
    let status = match walked {
        Err(err) => SearchStatus::Failed(err.to_string()),
        Ok(_) if delivery.overflowed() => SearchStatus::StoppedAtCap,
        Ok(_) => SearchStatus::Complete,
    };
    send(SearchEvent::Done(status));
}

fn log_end(root: &Path, control: &WalkControl, started: Instant, err: Option<&fx_core::ListError>) {
    match err {
        None => log::info!(
            "search in {} walked {} dirs, {} unreadable, in {:?}",
            root.display(),
            control.dirs_read(),
            control.unreadable(),
            started.elapsed()
        ),
        Some(err) => log::warn!("search root {} failed: {err}", root.display()),
    }
}

/// Filters hidden hits and enforces the result cap before anything reaches the channel, from many
/// walk threads at once. Exactly `cap` matches is complete; the first one beyond it cancels the walk.
struct Delivery<'a> {
    show_hidden: bool,
    cap: usize,
    admitted: AtomicUsize,
    overflowed: AtomicBool,
    control: &'a WalkControl,
}

impl<'a> Delivery<'a> {
    fn new(request: &'a SearchRequest) -> Self {
        Self {
            show_hidden: request.show_hidden,
            cap: request.cap,
            admitted: AtomicUsize::new(0),
            overflowed: AtomicBool::new(false),
            control: &request.control,
        }
    }

    /// The part of `hits` that may be delivered. Across all calls, never more than `cap` hits in total.
    fn admit(&self, mut hits: Vec<Hit>) -> Vec<Hit> {
        if !self.show_hidden {
            hits.retain(|hit| !hit.hidden);
        }
        let before = self.admitted.fetch_add(hits.len(), Relaxed);
        let room = self.cap.saturating_sub(before);
        if hits.len() > room {
            hits.truncate(room);
            self.overflowed.store(true, Relaxed);
            self.control.cancel();
        }
        hits
    }

    fn overflowed(&self) -> bool {
        self.overflowed.load(Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx_core::EntryKind;

    fn request(cap: usize, show_hidden: bool) -> SearchRequest {
        SearchRequest {
            tab_id: 0,
            generation: 1,
            root: PathBuf::new(),
            query: SearchQuery::new("x", Default::default()),
            show_hidden,
            cap,
            control: Arc::default(),
        }
    }

    fn hits(n: usize, hidden: bool) -> Vec<Hit> {
        (0..n)
            .map(|i| Hit {
                path: PathBuf::from(format!("f{i}")),
                kind: EntryKind::File,
                hidden,
            })
            .collect()
    }

    #[test]
    fn hidden_hits_are_dropped_and_do_not_count_towards_the_cap() {
        let request = request(2, false);
        let delivery = Delivery::new(&request);
        assert!(delivery.admit(hits(5, true)).is_empty());
        assert_eq!(delivery.admit(hits(2, false)).len(), 2);
        assert!(!delivery.overflowed());
    }

    #[test]
    fn exactly_cap_hits_do_not_overflow_and_one_more_does() {
        let request = request(3, true);
        let delivery = Delivery::new(&request);
        assert_eq!(delivery.admit(hits(3, false)).len(), 3);
        assert!(!delivery.overflowed());
        assert!(!request.control.is_cancelled());
        assert!(delivery.admit(hits(1, false)).is_empty());
        assert!(delivery.overflowed());
        assert!(request.control.is_cancelled());
    }

    #[test]
    fn concurrent_batches_never_deliver_more_than_the_cap() {
        let request = request(1_000, true);
        let delivery = Delivery::new(&request);
        let delivered = AtomicUsize::new(0);
        let hammer = || {
            for _ in 0..100 {
                let n = delivery.admit(hits(3, false)).len();
                delivered.fetch_add(n, Relaxed);
            }
        };
        thread::scope(|scope| {
            (0..8).for_each(|_| {
                scope.spawn(hammer);
            });
        });
        assert_eq!(delivered.load(Relaxed), 1_000);
        assert!(delivery.overflowed());
    }
}
