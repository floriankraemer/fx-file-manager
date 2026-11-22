//! Properties of a row, read in the background so a slow mount cannot stall the window.

use super::Explorer;
use crate::ops::{EntryDetails, details};
use crate::probe::timed;
use std::sync::Arc;
use std::thread;

/// Progress of the properties request of the most recent row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetailsState {
    Loading,
    Ready(EntryDetails),
    /// The entry vanished or cannot be read.
    Gone,
}

pub(super) struct DetailsLoaded {
    generation: u64,
    result: Option<EntryDetails>,
}

impl Explorer {
    /// Starts reading the properties of visible row `row` of the active tab. Returns false if there is no such row.
    /// Follow the answer with [`Explorer::details`]; asking again drops an older, still running request.
    pub fn request_details(&mut self, row: usize) -> bool {
        let Some(path) = self.entry_path(row) else {
            return false;
        };
        self.details_generation += 1;
        self.details = Some(DetailsState::Loading);
        let generation = self.details_generation;
        let tx = self.details_tx.clone();
        let wake = Arc::clone(&self.wake);
        thread::spawn(move || {
            let result = timed("properties", path.display(), || details(&path));
            let _ = tx.send(DetailsLoaded { generation, result });
            wake();
        });
        true
    }

    pub fn details(&self) -> Option<&DetailsState> {
        self.details.as_ref()
    }

    /// Stores the answer of the latest request. Returns true if it arrived.
    pub(super) fn apply_details(&mut self) -> bool {
        let mut changed = false;
        while let Ok(loaded) = self.details_rx.try_recv() {
            if loaded.generation != self.details_generation {
                continue;
            }
            self.details = Some(
                loaded
                    .result
                    .map_or(DetailsState::Gone, DetailsState::Ready),
            );
            changed = true;
        }
        changed
    }
}
