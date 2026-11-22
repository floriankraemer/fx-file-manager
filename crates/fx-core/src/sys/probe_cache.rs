//! Process-wide cache for slow per-volume probes (sysfs reads, device ioctls), keyed by volume identity.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// Short enough that a re-plugged stick reusing a device number is probed again.
const TTL: Duration = Duration::from_secs(60);
const CAPACITY: usize = 64;

pub(crate) struct TtlCache<K, V> {
    entries: Vec<(K, V, Instant)>,
}

impl<K: Eq, V: Copy> TtlCache<K, V> {
    pub(crate) const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn get(&self, key: &K, now: Instant) -> Option<V> {
        self.entries
            .iter()
            .find(|(k, _, at)| k == key && now.saturating_duration_since(*at) < TTL)
            .map(|&(_, v, _)| v)
    }

    fn insert(&mut self, key: K, value: V, now: Instant) {
        self.entries.retain(|(k, _, _)| *k != key);
        if self.entries.len() >= CAPACITY {
            let oldest = (0..self.entries.len()).min_by_key(|&i| self.entries[i].2);
            if let Some(i) = oldest {
                self.entries.swap_remove(i);
            }
        }
        self.entries.push((key, value, now));
    }
}

fn lock<K, V>(cache: &Mutex<TtlCache<K, V>>) -> MutexGuard<'_, TtlCache<K, V>> {
    cache.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The lock is never held while `probe` runs, so a slow device blocks only its own caller.
pub(crate) fn cached<K: Eq, V: Copy>(
    cache: &Mutex<TtlCache<K, V>>,
    key: K,
    probe: impl FnOnce() -> V,
) -> V {
    if let Some(hit) = lock(cache).get(&key, Instant::now()) {
        return hit;
    }
    let value = probe();
    lock(cache).insert(key, value, Instant::now());
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_expires_after_ttl() {
        let (mut cache, t0) = (TtlCache::new(), Instant::now());
        cache.insert(1u64, 'a', t0);
        assert_eq!(cache.get(&1, t0 + TTL - Duration::from_secs(1)), Some('a'));
        assert_eq!(cache.get(&1, t0 + TTL), None);
        assert_eq!(cache.get(&2, t0), None);
    }

    #[test]
    fn reinserting_a_key_replaces_and_refreshes_it() {
        let (mut cache, t0) = (TtlCache::new(), Instant::now());
        cache.insert(1u64, 'a', t0);
        cache.insert(1, 'b', t0 + TTL);
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.get(&1, t0 + TTL), Some('b'));
    }

    #[test]
    fn size_is_bounded_and_the_oldest_goes_first() {
        let (mut cache, t0) = (TtlCache::new(), Instant::now());
        for i in 0..=CAPACITY as u64 {
            cache.insert(i, i, t0 + Duration::from_millis(i));
        }
        assert_eq!(cache.entries.len(), CAPACITY);
        assert_eq!(cache.get(&0, t0), None);
        assert_eq!(cache.get(&1, t0), Some(1));
        assert_eq!(cache.get(&(CAPACITY as u64), t0), Some(CAPACITY as u64));
    }

    #[test]
    fn probe_runs_once_per_key() {
        static CACHE: Mutex<TtlCache<u8, u8>> = Mutex::new(TtlCache::new());
        let mut probes = 0;
        for _ in 0..3 {
            let v = cached(&CACHE, 9, || {
                probes += 1;
                42
            });
            assert_eq!(v, 42);
        }
        assert_eq!(probes, 1);
    }
}
