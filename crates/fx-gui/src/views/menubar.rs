//! Top menu bar: File and View.

use super::commandbar::menu_item;
use crate::action::Action;
use crate::command::{Command, hint};
use eframe::egui::{self, Style, Ui, containers::menu, vec2};
use fx_app::{Density, Explorer, t};

/// Menu bar items padded 8x4 and menu entries 12x5, like the IDE; egui's default bar padding is 2x0.
fn bar_style(style: &mut Style) {
    menu::menu_style(style);
    style.spacing.button_padding = vec2(8.0, 4.0);
}

fn menu_entry_style(style: &mut Style) {
    menu::menu_style(style);
    style.spacing.button_padding = vec2(12.0, 5.0);
    style.spacing.item_spacing.y = 2.0;
}

pub fn show(ui: &mut Ui, explorer: &Explorer, actions: &mut Vec<Action>) {
    egui::MenuBar::new()
        .style(bar_style)
        .config(menu::MenuConfig::new().style(menu_entry_style))
        .ui(ui, |ui| {
            ui.menu_button(t("menu.file"), |ui| {
                if menu_item(ui, t("cmd.settings"), &hint(Command::OpenSettings), true) {
                    actions.push(Action::OpenSettings);
                    ui.close();
                }
            });
            ui.menu_button(t("menu.view"), |ui| view_menu(ui, explorer, actions));
            ui.menu_button(t("menu.help"), |ui| {
                if menu_item(ui, t("menu.about"), &hint(Command::About), true) {
                    actions.push(Action::OpenAbout);
                    ui.close();
                }
            });
        });
}

fn view_menu(ui: &mut Ui, explorer: &Explorer, actions: &mut Vec<Action>) {
    let settings = explorer.settings();
    let mut hidden = settings.show_hidden;
    if ui
        .checkbox(&mut hidden, t("settings.show_hidden"))
        .changed()
    {
        actions.push(Action::SetShowHidden(hidden));
    }
    ui.separator();
    for (density, label) in [
        (Density::Compact, t("density.compact")),
        (Density::Comfortable, t("density.comfortable")),
    ] {
        if ui.radio(settings.density == density, label).clicked() {
            actions.push(Action::SetDensity(density));
        }
    }
}
