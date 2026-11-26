//! Top menu bar: File and View.

use super::menu::{self, bar_style};
use super::search::SearchAction;
use crate::action::Action;
use crate::command::{Command, hint};
use eframe::egui::{self, Ui};
use fx_app::{Density, Explorer, t};

pub fn show(ui: &mut Ui, explorer: &Explorer, actions: &mut Vec<Action>) {
    egui::MenuBar::new().style(bar_style).ui(ui, |ui| {
        ui.menu_button(t("menu.file"), |ui| {
            if menu::item(ui, t("cmd.new_window"), &hint(Command::NewWindow), true) {
                actions.push(Action::NewWindow);
                ui.close();
            }
            if menu::item(ui, t("command.search"), &hint(Command::Search), true) {
                actions.push(Action::Search(SearchAction::Open));
                ui.close();
            }
            if menu::item(
                ui,
                t("command.command_palette"),
                &hint(Command::Palette),
                true,
            ) {
                actions.push(Action::OpenCommandPalette);
                ui.close();
            }
            if menu::item(ui, t("cmd.settings"), &hint(Command::OpenSettings), true) {
                actions.push(Action::OpenSettings);
                ui.close();
            }
            ui.separator();
            if menu::item(ui, t("cmd.exit"), &hint(Command::Exit), true) {
                actions.push(Action::Exit);
                ui.close();
            }
        });
        ui.menu_button(t("menu.view"), |ui| view_menu(ui, explorer, actions));
        ui.menu_button(t("menu.help"), |ui| {
            if menu::item(ui, t("menu.about"), &hint(Command::About), true) {
                actions.push(Action::OpenAbout);
                ui.close();
            }
        });
    });
}

fn view_menu(ui: &mut Ui, explorer: &Explorer, actions: &mut Vec<Action>) {
    let settings = explorer.settings();
    if menu::check(ui, t("settings.show_hidden"), settings.show_hidden) {
        actions.push(Action::SetShowHidden(!settings.show_hidden));
    }
    ui.separator();
    super::commandbar::view_menu(ui, explorer.active().view_mode(), actions);
    ui.separator();
    for (density, label) in [
        (Density::Compact, t("density.compact")),
        (Density::Comfortable, t("density.comfortable")),
    ] {
        if menu::radio(ui, label, settings.density == density) {
            actions.push(Action::SetDensity(density));
        }
    }
}
