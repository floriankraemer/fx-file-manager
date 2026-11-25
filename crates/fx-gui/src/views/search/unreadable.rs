use crate::theme;
use eframe::egui::{self, Align, Context, Layout, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};
use fx_app::{SearchView, t, tr};

const ROW_HEIGHT: f32 = 20.0;
const PATH_WIDTH: f32 = 320.0;

/// The folders the walk could not read, with the reason; closing it clears `open`.
pub fn window(ctx: &Context, view: &SearchView, open: &mut bool) {
    egui::Window::new(t("search.unreadable_title"))
        .open(open)
        .default_size([640.0, 320.0])
        .show(ctx, |ui| content(ui, view));
}

fn content(ui: &mut Ui, view: &SearchView) {
    let listed = view.unreadable_folders();
    if (listed.len() as u64) < view.unreadable() {
        let note = tr(
            "search.unreadable_capped",
            &[
                ("n", &listed.len().to_string()),
                ("total", &view.unreadable().to_string()),
            ],
        );
        ui.label(RichText::new(note).color(theme::dim()));
    }
    TableBuilder::new(ui)
        .id_salt("search-unreadable-rows")
        .striped(true)
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::initial(PATH_WIDTH).at_least(80.0).clip(true))
        .column(Column::remainder().at_least(80.0).clip(true))
        .sense(Sense::hover())
        .header(ROW_HEIGHT, |mut header| {
            for title in [t("search.col_folder"), t("search.col_error")] {
                header.col(|ui| {
                    ui.label(RichText::new(title).strong().color(theme::header_text()));
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, listed.len(), |mut row| {
                let folder = &listed[row.index()];
                row.col(|ui| {
                    ui.add(egui::Label::new(folder.path.display().to_string()).truncate());
                });
                row.col(|ui| {
                    let error = RichText::new(&folder.error).color(theme::dim());
                    ui.add(egui::Label::new(error).truncate());
                });
            });
        });
}
