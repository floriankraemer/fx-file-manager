//! About dialog: version, build identity, author and license.

use super::dialogs::{button_row, modal};
use crate::theme;
use eframe::egui::{Button, Context, Grid, RichText};
use fx_app::t;

const NAME: &str = "fx";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const AUTHORS: &str = env!("CARGO_PKG_AUTHORS");
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const LICENSE: &str = env!("CARGO_PKG_LICENSE");
const HASH: &str = env!("FX_GIT_HASH");
const DATE: &str = env!("FX_BUILD_DATE");

/// Plain-text build identity for bug reports.
fn report(name: &str, version: &str, hash: &str, date: &str) -> String {
    format!("{name} {version} (build {hash}, {date})")
}

pub fn show(ctx: &Context) -> bool {
    modal(ctx, "about-dialog", |ui| {
        ui.heading(NAME);
        ui.label(RichText::new(t("about.tagline")).color(theme::dim()));
        ui.add_space(8.0);
        Grid::new("about-rows").num_columns(2).show(ui, |ui| {
            for (label, value) in [
                (t("about.version"), VERSION),
                (t("about.build_hash"), HASH),
                (t("about.build_date"), DATE),
                (t("about.author"), AUTHORS),
                (t("about.license"), LICENSE),
            ] {
                ui.label(RichText::new(label).color(theme::dim()));
                ui.label(value);
                ui.end_row();
            }
            ui.label(RichText::new(t("about.repository")).color(theme::dim()));
            ui.hyperlink(REPOSITORY);
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new(t("about.translations"))
                .small()
                .color(theme::dim()),
        );
        let mut close = false;
        button_row(ui, |ui| {
            close |= ui.add(Button::new(t("button.close"))).clicked();
            if ui
                .button(t("button.copy"))
                .on_hover_text(t("about.copy_tip"))
                .clicked()
            {
                ui.ctx().copy_text(report(NAME, VERSION, HASH, DATE));
            }
        });
        close
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_format() {
        assert_eq!(
            report("fx", "0.1.0", "abc123", "2026-01-02"),
            "fx 0.1.0 (build abc123, 2026-01-02)"
        );
    }

    #[test]
    fn build_info_present() {
        assert!(!HASH.is_empty() && DATE.len() == 10 && !AUTHORS.is_empty());
    }
}
