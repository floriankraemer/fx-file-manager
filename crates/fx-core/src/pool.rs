//! Rayon pools: cached per thread count for listing (spawning threads per call costs ~100us+),
//! and one fresh pool per transfer for copying.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Runs `op` on the cached pool for `threads`. When no pool can be built (the OS refuses to spawn
/// threads) it retries with one thread and finally runs `op` on the calling thread.
// ponytail: the inline fallback still reaches rayon's global pool for nested parallelism; make walk serial if that matters.
pub(crate) fn run_in_pool<R: Send>(threads: usize, op: impl FnOnce() -> R + Send) -> R {
    match pool(threads).or_else(|| pool(1)) {
        Some(pool) => pool.install(op),
        None => op(),
    }
}

fn pool(threads: usize) -> Option<Arc<rayon::ThreadPool>> {
    static POOLS: OnceLock<Mutex<HashMap<usize, Arc<rayon::ThreadPool>>>> = OnceLock::new();
    let mut pools = POOLS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if let Some(pool) = pools.get(&threads) {
        return Some(pool.clone());
    }
    let built = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("fx-worker-{i}"))
        .build();
    match built {
        Ok(pool) => Some(pools.entry(threads).or_insert(Arc::new(pool)).clone()),
        Err(e) => {
            log::warn!("cannot build a {threads}-thread pool: {e}");
            None
        }
    }
}

/// A pool owned by one transfer. Not cached: copy workers can block for long (a conflict waiting for the
/// user, a full disk, a paused job), and that must never stall listing or another transfer.
pub(crate) fn copy_pool(threads: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .thread_name(|i| format!("fx-copy-{i}"))
        .build()
        .expect("failed to build copy pool")
}
