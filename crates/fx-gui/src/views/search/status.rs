use crate::action::Action;
use crate::theme;
use eframe::egui::{self, Layout, Ui};
use fx_app::{SearchStatus, SearchView, format_elapsed, t, tn};
use std::time::Duration;

/// Repaint rate while a search runs, so the live counts and the clock keep moving between batches.
const SEARCH_REPAINT: Duration = Duration::from_millis(100);

pub fn summary(ui: &mut Ui, search: &SearchView, _actions: &mut Vec<Action>) {
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
