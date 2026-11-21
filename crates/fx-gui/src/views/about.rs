//! About window: version, build identity, author and license.

use super::dialogs::button_row;
use super::menu::ENTRY_PADDING;
use crate::theme;
use eframe::egui::{
    CentralPanel, Context, Frame, Grid, Key, Margin, Modifiers, RichText, Ui, ViewportBuilder,
    ViewportCommand, ViewportId,
};
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

pub fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("about")
}

/// Brings the about window to the front.
pub fn focus(ctx: &Context) {
    ctx.send_viewport_cmd_to(viewport_id(), ViewportCommand::Focus);
}

/// Draws the about window. Returns true when it should be closed.
pub fn show(ctx: &Context) -> bool {
    let builder = ViewportBuilder::default()
        .with_title(t("menu.about"))
        .with_inner_size([420.0, 360.0])
        .with_resizable(false);
    let mut close = false;
    ctx.show_viewport_immediate(viewport_id(), builder, |ui, _class| {
        let ctx = ui.ctx().clone();
        close |= ctx.input(|i| i.viewport().close_requested())
            || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let style = ui.style().clone();
        let pad = ENTRY_PADDING.x as i8;
        CentralPanel::default()
            .frame(Frame::central_panel(&style).inner_margin(Margin::same(pad)))
            .show(ui, |ui| close |= content(ui));
    });
    close
}

/// Window content; true when Close was clicked.
fn content(ui: &mut Ui) -> bool {
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
    ui.label(RichText::new(t("about.icons")).small().color(theme::dim()));
    let mut close = false;
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = ENTRY_PADDING;
        button_row(ui, |ui| {
            close = ui.button(t("button.close")).clicked();
            if ui
                .button(t("button.copy"))
                .on_hover_text(t("about.copy_tip"))
                .clicked()
            {
                ui.ctx().copy_text(report(NAME, VERSION, HASH, DATE));
            }
        });
    });
    close
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
