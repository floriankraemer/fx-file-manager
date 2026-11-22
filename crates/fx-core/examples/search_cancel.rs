//! Measures how long `search` takes to return after its cancel flag is set mid-walk.
//!
//! `cargo run --release -p fx-core --example search_cancel -- <ROOT> [RUNS]`

use fx_core::{GlobScope, SearchQuery, WalkOptions, WalkProgress, search};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use std::{env, thread};

/// Cancel once the walk is this far along, so the pool is busy when the flag flips.
const CANCEL_AFTER_DIRS: u64 = 500;

fn main() {
    let mut args = env::args().skip(1);
    let root = args.next().expect("usage: search_cancel <ROOT> [RUNS]");
    let runs: usize = args
        .next()
        .map_or(20, |n| n.parse().expect("RUNS is a number"));
    let mut latencies: Vec<Duration> = (0..runs)
        .filter_map(|_| one_run(Path::new(&root)))
        .collect();
    latencies.sort();
    let (Some(median), Some(max)) = (latencies.get(latencies.len() / 2), latencies.last()) else {
        eprintln!("the walk finished before it could be cancelled; use a bigger tree");
        return;
    };
    println!(
        "cancel latency over {} runs: median {median:?}, max {max:?}",
        latencies.len()
    );
}

/// Time from setting the flag to `search` returning; `None` if the walk ended first.
fn one_run(root: &Path) -> Option<Duration> {
    let cancel = AtomicBool::new(false);
    let progress = WalkProgress::default();
    let query = SearchQuery::new("", GlobScope::Name);
    let opts = WalkOptions::default();
    thread::scope(|scope| {
        let walker = scope.spawn(|| {
            let _ = search(root, &query, &opts, &cancel, &progress, |_| {});
            Instant::now()
        });
        while progress.dirs_read.load(Ordering::Relaxed) < CANCEL_AFTER_DIRS {
            if walker.is_finished() {
                return None;
            }
            thread::yield_now();
        }
        let flagged = Instant::now();
        cancel.store(true, Ordering::Relaxed);
        let returned = walker.join().expect("search thread panicked");
        Some(returned.saturating_duration_since(flagged))
    })
}
