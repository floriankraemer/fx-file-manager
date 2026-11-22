//! Sidebar locations, probed in the background: checking which folders and drives exist can block on a dead mount.

use super::Explorer;
use crate::places::{Place, places};
use crate::probe::timed;
use std::sync::Arc;
use std::thread;

impl Explorer {
    /// Probes the sidebar locations on a worker thread; the answer arrives through [`Explorer::poll`] and [`Explorer::take_places`].
    /// A probe that is still running (a dead mount) is not started again, so threads cannot pile up.
    pub fn refresh_places(&mut self) {
        if self.places_probing {
            log::debug!("places probe still running, skipped");
            return;
        }
        self.places_probing = true;
        let tx = self.places_tx.clone();
        let wake = Arc::clone(&self.wake);
        thread::spawn(move || {
            let found = timed("places probe", "sidebar", places);
            let _ = tx.send(found);
            wake();
        });
    }

    /// The locations of the latest finished probe, once. `None` until a new one arrived.
    pub fn take_places(&mut self) -> Option<Vec<Place>> {
        self.fresh_places.take()
    }

    pub(super) fn apply_places(&mut self) -> bool {
        let mut changed = false;
        while let Ok(found) = self.places_rx.try_recv() {
            self.places_probing = false;
            self.fresh_places = Some(found);
            changed = true;
        }
        changed
    }
}
