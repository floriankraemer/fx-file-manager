//! Timing of filesystem checks that run off the UI thread, because a dead network mount can stall them.

use std::fmt::Display;
use std::time::{Duration, Instant};

const SLOW: Duration = Duration::from_secs(2);

/// Runs `check` and logs at debug when it took longer than two seconds.
pub(crate) fn timed<T>(what: &str, subject: impl Display, check: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = check();
    let elapsed = started.elapsed();
    if elapsed > SLOW {
        log::debug!("{what} of {subject} was slow: {elapsed:?}");
    }
    result
}
