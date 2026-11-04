//! Windows-11-style command bar: file commands on the left, Sort/View dropdowns, and an overflow menu on the right.

use super::icons::{self, Glyph};
use crate::{action::Action, theme};
use eframe::egui::{Align, Layout, Popup, Response, Sense, TextStyle, Ui, vec2};
use fx_app::{Explorer, SortColumn, SortSpec, t};

const HEIGHT: f32 = 28.0;

pub fn show(ui: &mut Ui, explorer: &Explorer, actions: &mut Vec<Action>) {
    let has_selection = !explorer.active().selection().is_empty();
    let one_selected = explorer.active().selection().len() == 1;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if command(ui, Glyph::NewFolder, t("cmd.new_folder"), true, false)
            .on_hover_text(t("tip.new_folder"))
            .clicked()
        {
            actions.push(Action::NewFolder);
        }
        if command(ui, Glyph::Rename, t("cmd.rename"), one_selected, false)
            .on_hover_text(t("tip.rename"))
            .clicked()
        {
            actions.push(Action::Rename);
        }
        if command(ui, Glyph::Delete, t("cmd.delete"), has_selection, false)
            .on_hover_text(t("tip.delete"))
            .clicked()
        {
            actions.push(Action::Delete { permanent: false });
        }
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        let resp = command(ui, Glyph::Sort, t("cmd.sort"), true, true);
        Popup::menu(&resp).show(|ui| sort_menu(ui, explorer.active().sort(), actions));

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let resp = command(ui, Glyph::More, "", true, false);
            Popup::menu(&resp).show(|ui| {
                if menu_item(ui, t("cmd.select_all"), "Ctrl+A", true) {
                    actions.push(Action::SelectAll);
                }
            });
        });
    });
}

fn sort_menu(ui: &mut Ui, sort: SortSpec, actions: &mut Vec<Action>) {
    let columns = [
        (SortColumn::Name, t("col.name")),
        (SortColumn::Ext, t("col.ext")),
        (SortColumn::Size, t("col.size")),
        (SortColumn::Modified, t("col.modified")),
    ];
    for (column, label) in columns {
        if ui.radio(sort.column == column, label).clicked() {
            actions.push(Action::SetSort(SortSpec { column, ..sort }));
        }
    }
    ui.separator();
    for (ascending, label) in [(true, t("sort.ascending")), (false, t("sort.descending"))] {
        if ui.radio(sort.ascending == ascending, label).clicked() {
            actions.push(Action::SetSort(SortSpec { ascending, ..sort }));
        }
    }
}

/// A menu entry with a right-aligned shortcut hint. Returns true when clicked.
pub fn menu_item(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(label)
            .shortcut_text(shortcut)
            .frame_when_inactive(false),
    )
    .clicked()
}

/// A flat command button: painted icon, label, optional dropdown chevron.
fn command(ui: &mut Ui, glyph: Glyph, label: &str, enabled: bool, chevron: bool) -> Response {
    let font = TextStyle::Button.resolve(ui.style());
    let color = if enabled {
        theme::text()
    } else {
        theme::dim().gamma_multiply(0.6)
    };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, color);
    let text_w = if label.is_empty() {
        0.0
    } else {
        galley.size().x + 6.0
    };
    let width = 8.0 + 16.0 + text_w + if chevron { 14.0 } else { 0.0 } + 8.0;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, HEIGHT), sense);
    if enabled && (resp.hovered() || resp.is_pointer_button_down_on()) {
        let fill = if resp.is_pointer_button_down_on() {
            theme::border()
        } else {
            theme::panel_hi()
        };
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    let icon = egui::Rect::from_center_size(rect.left_center() + vec2(16.0, 0.0), vec2(16.0, 16.0));
    icons::glyph(ui, icon, glyph, color);
    if !label.is_empty() {
        ui.painter().galley(
            egui::pos2(icon.right() + 6.0, rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    }
    if chevron {
        let chev =
            egui::Rect::from_center_size(rect.right_center() - vec2(14.0, 0.0), vec2(10.0, 10.0));
        icons::glyph(ui, chev, Glyph::Chevron, color);
    }
    resp
}
