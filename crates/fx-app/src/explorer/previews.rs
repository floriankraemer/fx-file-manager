//! Thumbnails of the active tab's files, for the Icons view.

use super::Explorer;
use crate::tab::LoadState;
use crate::thumbnails::{MAX_WORKERS, ThumbKey};
use fx_core::{EntryKind, StorageKind};
use fx_thumbs::Thumb;
use std::sync::Arc;

impl Explorer {
    /// What identifies the thumbnail of `row` at `px` physical pixels; `None` when it gets none:
    /// folders, thumbnails switched off, or network storage unless allowed there.
    pub fn thumb_key(&self, row: usize, px: u32) -> Option<ThumbKey> {
        if !self.settings.thumbnails {
            return None;
        }
        let tab = self.active();
        let LoadState::Loaded(view) = tab.state() else {
            return None;
        };
        if view.storage == StorageKind::Network && !self.settings.thumbnails_on_network {
            return None;
        }
        let entry = &view.listing.entries[*tab.order().get(row)? as usize];
        if !matches!(entry.kind, EntryKind::File | EntryKind::Symlink) {
            return None;
        }
        Some(ThumbKey {
            path: tab.path().join(view.listing.name(entry)),
            modified: entry.modified,
            size: entry.size.unwrap_or(0),
            px: fx_thumbs::bucket(px),
        })
    }

    /// Replaces the queued thumbnail work with `keys`, most wanted first.
    pub fn want_thumbnails(&mut self, keys: Vec<ThumbKey>) {
        let storage = match self.active().state() {
            LoadState::Loaded(view) => view.storage,
            _ => StorageKind::Unknown,
        };
        self.thumbnails.want(keys, workers(storage));
    }

    /// Thumbnails finished since the last call.
    pub fn take_thumbnails(&mut self) -> Vec<(ThumbKey, Arc<Thumb>)> {
        self.thumbnails.take()
    }
}

/// Spinning disks and USB sticks slow down under parallel reads, solid state and caches do not.
fn workers(storage: StorageKind) -> usize {
    match storage {
        StorageKind::Hdd | StorageKind::Removable | StorageKind::Network => 1,
        StorageKind::Ssd | StorageKind::Unknown => {
            let cpus = std::thread::available_parallelism().map_or(2, |n| n.get());
            (cpus / 2).clamp(1, MAX_WORKERS)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ViewMode;
    use crate::explorer::tests::{row_of, settle};
    use crate::settings::Settings;
    use std::time::{Duration, Instant};

    #[test]
    fn files_get_keys_and_their_thumbnails_arrive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        image::RgbImage::from_pixel(8, 8, image::Rgb([9, 9, 9]))
            .save(dir.path().join("pic.png"))
            .unwrap();
        let settings = Settings {
            default_view: ViewMode::Icons,
            ..Settings::default()
        };
        let mut explorer = Explorer::with_settings(dir.path().to_path_buf(), settings, || {});
        settle(&mut explorer);
        assert_eq!(explorer.active().view_mode(), ViewMode::Icons);
        assert_eq!(explorer.thumb_key(row_of(&explorer, "sub"), 100), None);
        let key = explorer
            .thumb_key(row_of(&explorer, "pic.png"), 100)
            .unwrap();
        assert_eq!(key.px, 128);

        explorer.want_thumbnails(vec![key.clone()]);
        let deadline = Instant::now() + Duration::from_secs(10);
        let ready = loop {
            let ready = explorer.take_thumbnails();
            if !ready.is_empty() || Instant::now() > deadline {
                break ready;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].0, key);
    }

    #[test]
    fn thumbnails_switched_off_give_no_keys() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), "x").unwrap();
        let settings = Settings {
            thumbnails: false,
            ..Settings::default()
        };
        let mut explorer = Explorer::with_settings(dir.path().to_path_buf(), settings, || {});
        settle(&mut explorer);
        assert_eq!(explorer.thumb_key(0, 128), None);
    }

    #[test]
    fn seek_bound_storage_gets_one_worker() {
        assert_eq!(workers(StorageKind::Hdd), 1);
        assert_eq!(workers(StorageKind::Network), 1);
        assert!((1..=MAX_WORKERS).contains(&workers(StorageKind::Ssd)));
    }
}
