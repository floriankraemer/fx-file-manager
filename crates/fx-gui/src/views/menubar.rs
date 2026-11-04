//! Top menu bar: File and View.

use super::commandbar::menu_item;
use crate::action::Action;
use eframe::egui::{self, Ui};
use fx_app::{Density, Explorer};

pub fn show(ui: &mut Ui, explorer: &Explorer, actions: &mut Vec<Action>) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            if menu_item(ui, "Settings", "Ctrl+,", true) {
                actions.push(Action::OpenSettings);
                ui.close();
            }
        });
        ui.menu_button("View", |ui| view_menu(ui, explorer, actions));
        ui.menu_button("Help", |ui| {
            if menu_item(ui, "About fx", "F1", true) {
                actions.push(Action::OpenAbout);
                ui.close();
            }
        });
    });
}

fn view_menu(ui: &mut Ui, explorer: &Explorer, actions: &mut Vec<Action>) {
    let settings = explorer.settings();
    let mut hidden = settings.show_hidden;
    if ui.checkbox(&mut hidden, "Show hidden items").changed() {
        actions.push(Action::SetShowHidden(hidden));
    }
    ui.separator();
    for (density, label) in [
        (Density::Compact, "Compact"),
        (Density::Comfortable, "Comfortable"),
    ] {
        if ui.radio(settings.density == density, label).clicked() {
            actions.push(Action::SetDensity(density));
        }
    }
}
