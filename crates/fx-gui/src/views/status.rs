use crate::theme;
use eframe::egui::{Layout, Ui};
use fx_app::{Explorer, LoadState, format_millis, format_size, tn, tr};
use fx_core::EntryKind;

pub fn show(ui: &mut Ui, explorer: &Explorer) {
    let tab = explorer.active();
    let LoadState::Loaded(view) = tab.state() else {
        ui.horizontal(|ui| {
            ui.label(" ");
            if explorer.busy() {
                ui.spinner();
            }
        });
        return;
    };
    ui.horizontal(|ui| {
        let counts = [
            (view.files, "status.files"),
            (view.dirs, "status.dirs"),
            (view.symlinks, "status.links"),
            (view.others, "status.misc"),
        ]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|&(n, key)| tn(key, n as u64, &[]))
        .collect::<Vec<_>>()
        .join(" · ");
        ui.colored_label(
            theme::dim(),
            tr(
                "status.summary",
                &[
                    ("items", &tn("status.items", tab.order().len() as u64, &[])),
                    ("counts", &counts),
                    ("size", &format_size(view.total_file_bytes)),
                    ("storage", &view.storage.to_string()),
                    ("time", &format_millis(view.elapsed)),
                ],
            ),
        );
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            if explorer.busy() {
                ui.spinner();
            }
            let selected = tab
                .selected()
                .and_then(|row| tab.order().get(row))
                .map(|&i| &view.listing.entries[i as usize]);
            if let Some(entry) = selected {
                let name = view.listing.name(entry).to_string_lossy();
                let text = match (entry.kind, entry.size) {
                    (EntryKind::File, Some(size)) => format!("{name} · {}", format_size(size)),
                    _ => name.into_owned(),
                };
                ui.colored_label(theme::accent(), text);
            }
        });
    });
}
