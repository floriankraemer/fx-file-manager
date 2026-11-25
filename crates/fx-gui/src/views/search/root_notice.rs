use crate::views::modal::{button_row, default_focus, modal};
use eframe::egui::Context;
use fx_app::{Explorer, t, tr};

/// The notice before searching from a filesystem root. Returns true when it should close.
pub fn show(ctx: &Context, explorer: &mut Explorer) -> bool {
    modal(ctx, "search-root-dialog", |ui| {
        ui.heading(t("search.root_notice.title"));
        ui.add_space(6.0);
        let root = explorer.active().path().display().to_string();
        ui.label(tr("search.root_notice.body", &[("root", &root)]));
        let (mut go, mut silence) = (false, false);
        button_row(ui, |ui| {
            let proceed = ui.button(t("search.root_notice.continue"));
            silence = ui.button(t("search.root_notice.dont_show")).clicked();
            default_focus(ui, &proceed);
            go = proceed.clicked();
        });
        if go || silence {
            explorer.acknowledge_root_notice(silence);
        }
        go || silence
    })
}
