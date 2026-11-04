//! Rayon pools: cached per thread count for listing (spawning threads per call costs ~100us+),
//! and one fresh pool per transfer for copying.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub(crate) fn pool(threads: usize) -> Arc<rayon::ThreadPool> {
    static POOLS: OnceLock<Mutex<HashMap<usize, Arc<rayon::ThreadPool>>>> = OnceLock::new();
    let mut pools = POOLS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    pools
        .entry(threads)
        .or_insert_with(|| {
            Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .thread_name(|i| format!("fx-worker-{i}"))
                    .build()
                    .expect("failed to build thread pool"),
            )
        })
        .clone()
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
