//! About dialog: version, build identity, author and license.

use super::dialogs::{button_row, modal};
use crate::theme;
use eframe::egui::{Button, Context, Grid, RichText};

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
        ui.label(RichText::new("A fast, cross-platform file manager").color(theme::dim()));
        ui.add_space(8.0);
        Grid::new("about-rows").num_columns(2).show(ui, |ui| {
            for (label, value) in [
                ("Version", VERSION),
                ("Build hash", HASH),
                ("Build date", DATE),
                ("Author", AUTHORS),
                ("License", LICENSE),
            ] {
                ui.label(RichText::new(label).color(theme::dim()));
                ui.label(value);
                ui.end_row();
            }
            ui.label(RichText::new("Repository").color(theme::dim()));
            ui.hyperlink(REPOSITORY);
            ui.end_row();
        });
        let mut close = false;
        button_row(ui, |ui| {
            close |= ui.add(Button::new("Close")).clicked();
            if ui
                .button("Copy")
                .on_hover_text("Copy version and build info")
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
