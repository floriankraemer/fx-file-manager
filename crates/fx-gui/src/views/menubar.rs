//! Top menu bar: File and View.

use super::commandbar::menu_item;
use crate::action::Action;
use eframe::egui::{self, Ui};
use fx_app::{Density, Explorer, t};

pub fn show(ui: &mut Ui, explorer: &Explorer, actions: &mut Vec<Action>) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button(t("menu.file"), |ui| {
            if menu_item(ui, t("cmd.settings"), "Ctrl+,", true) {
                actions.push(Action::OpenSettings);
                ui.close();
            }
        });
        ui.menu_button(t("menu.view"), |ui| view_menu(ui, explorer, actions));
        ui.menu_button(t("menu.help"), |ui| {
            if menu_item(ui, t("menu.about"), "F1", true) {
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
