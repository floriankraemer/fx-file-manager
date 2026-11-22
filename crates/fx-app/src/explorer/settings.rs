//! Applying and persisting user settings.

use super::Explorer;
use crate::settings::Settings;
use crate::view::SortSpec;
use std::io;

impl Explorer {
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Applies `show_hidden` to every tab (selections survive by name) and persists the settings.
    /// A save failure becomes an error notice; the new settings stay in effect for this session.
    pub fn set_settings(&mut self, settings: Settings) {
        let hidden_changed = settings.show_hidden != self.settings.show_hidden;
        for tab in self.windows.iter_mut().flat_map(|w| &mut w.tabs) {
            if tab.show_hidden() != settings.show_hidden {
                tab.set_show_hidden(settings.show_hidden);
            }
            if tab.folder_order() != settings.folder_order {
                tab.set_folder_order(settings.folder_order);
            }
            if !settings.column_shown(tab.sort().column.list_column()) {
                tab.set_sort(SortSpec::default());
            }
        }
        self.session_dirty |= settings.restore_session != self.settings.restore_session;
        let preload_changed = settings.preload_subfolders != self.settings.preload_subfolders;
        self.settings = settings;
        if preload_changed {
            for w in 0..self.windows.len() {
                self.maybe_preload(w);
            }
        }
        if hidden_changed {
            self.reload_tree();
        }
        let saved = match &self.settings_path {
            Some(path) => self.settings.save_to(path),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no config directory available",
            )),
        };
        self.report_save("settings", saved);
    }
}
