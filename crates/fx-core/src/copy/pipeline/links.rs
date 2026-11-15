//! Hardlink groups: the first name of a multiply-linked file is copied, the others link to it.

use super::lock;
use crate::copy::progress::{Shared, cancelled};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// The first destination of each multiply-linked source file, keyed by (device, inode). A name that
/// arrives while the first is still being copied waits for it, so a group is never copied twice.
#[derive(Default)]
pub(super) struct LinkGroups {
    slots: Mutex<HashMap<(u64, u64), LinkSlot>>,
    changed: Condvar,
}

enum LinkSlot {
    Copying,
    Done(PathBuf),
}

pub(super) enum Claim<'a> {
    /// The group's first copy is finished: link to it.
    Existing(PathBuf),
    /// This worker copies the data and then calls [`LinkLeader::finish`].
    Leader(LinkLeader<'a>),
}

/// Held while the first name of a group is copied. Dropped without `finish` (error, skip, cancel), it
/// frees the group so the next waiting name becomes the one to copy.
pub(super) struct LinkLeader<'a> {
    groups: &'a LinkGroups,
    key: (u64, u64),
    done: bool,
}

impl LinkLeader<'_> {
    pub(super) fn finish(mut self, dst: PathBuf) {
        lock(&self.groups.slots).insert(self.key, LinkSlot::Done(dst));
        self.done = true;
        self.groups.changed.notify_all();
    }
}

impl Drop for LinkLeader<'_> {
    fn drop(&mut self) {
        if !self.done {
            lock(&self.groups.slots).remove(&self.key);
            self.groups.changed.notify_all();
        }
    }
}

impl LinkGroups {
    pub(super) fn claim(&self, key: (u64, u64), sh: &Shared) -> io::Result<Claim<'_>> {
        let mut slots = lock(&self.slots);
        while matches!(slots.get(&key), Some(LinkSlot::Copying)) {
            if sh.is_cancelled() {
                return Err(cancelled());
            }
            slots = self
                .changed
                .wait_timeout(slots, Duration::from_millis(50))
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        if let Some(LinkSlot::Done(path)) = slots.get(&key) {
            return Ok(Claim::Existing(path.clone()));
        }
        slots.insert(key, LinkSlot::Copying);
        Ok(Claim::Leader(LinkLeader {
            groups: self,
            key,
            done: false,
        }))
    }
}
