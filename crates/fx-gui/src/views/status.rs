use crate::theme;
use eframe::egui::{Layout, Ui};
use fx_app::{Explorer, LoadState, format_millis, format_size};
use fx_core::EntryKind;

pub fn show(ui: &mut Ui, explorer: &Explorer) {
    let tab = explorer.active();
    let LoadState::Loaded(view) = tab.state() else {
        ui.label(" ");
        return;
    };
    ui.horizontal(|ui| {
        let counts = [
            (view.files, "files"),
            (view.dirs, "dirs"),
            (view.symlinks, "links"),
            (view.others, "other"),
        ]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, label)| format!("{n} {label}"))
        .collect::<Vec<_>>()
        .join(" · ");
        ui.colored_label(
            theme::DIM,
            format!(
                "{} items · {counts} · {} · storage: {} · listed in {}",
                tab.order().len(),
                format_size(view.total_file_bytes),
                view.storage,
                format_millis(view.elapsed),
            ),
        );
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
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
                ui.colored_label(theme::ACCENT, text);
            }
        });
    });
}
