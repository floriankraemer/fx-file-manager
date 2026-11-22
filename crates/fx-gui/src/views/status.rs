use crate::theme;
use eframe::egui::{Layout, Ui};
use fx_app::{
    Explorer, LoadState, SearchStatus, SearchView, format_elapsed, format_millis, format_size, t,
    tn, tr,
};
use fx_core::EntryKind;
use std::time::Duration;

/// Repaint rate while a search runs, so the live counts and the clock keep moving between batches.
const SEARCH_REPAINT: Duration = Duration::from_millis(100);

pub fn show(ui: &mut Ui, explorer: &Explorer) {
    if let Some(search) = explorer.search() {
        return search_summary(ui, search);
    }
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

fn search_summary(ui: &mut Ui, search: &SearchView) {
    ui.horizontal(|ui| {
        if search.is_running() {
            ui.spinner();
            ui.ctx().request_repaint_after(SEARCH_REPAINT);
        }
        let (text, color) = match search.status() {
            SearchStatus::Running => (t("search.running").to_owned(), theme::dim()),
            SearchStatus::Complete => (t("search.complete").to_owned(), theme::accent()),
            SearchStatus::Stopped => (t("search.stopped").to_owned(), theme::dim()),
            SearchStatus::StoppedAtCap => (t("search.stopped_at_cap").to_owned(), theme::error()),
            SearchStatus::Failed(_) => (t("search.stopped").to_owned(), theme::error()),
        };
        ui.colored_label(color, text);
        ui.colored_label(theme::dim(), counts(search));
        if search.unreadable() > 0 {
            ui.colored_label(
                theme::error(),
                tn("search.unreadable", search.unreadable(), &[]),
            );
        }
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            ui.colored_label(theme::dim(), search.root().display().to_string());
        });
    });
}

fn counts(search: &SearchView) -> String {
    [
        tn("search.matches", search.matches() as u64, &[]),
        tn("search.folders", search.dirs_walked(), &[]),
        format_elapsed(search.elapsed()),
    ]
    .join(" · ")
}
