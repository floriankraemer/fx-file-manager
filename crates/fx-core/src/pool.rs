//! Rayon pools cached per thread count (spawning threads per call costs ~100us+).

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
