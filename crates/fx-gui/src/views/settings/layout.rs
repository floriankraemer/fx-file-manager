//! Building blocks of the settings window: titles, group boxes, the two-column form and sidebar rows.

use super::rows::{Form, Kind, Row};
use super::{COLUMN_GAP, CONTROL_HEIGHT, GROUP_GAP, ROW_GAP, SIDEBAR_ROW};
use crate::theme;
use eframe::egui::{self, Align, Layout, RichText, Sense, Ui};
use fx_app::t;

/// A page or search-result title.
pub(super) fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).heading().strong());
    ui.add_space(4.0);
}

/// A bordered group with its title top-left inside, like a Qt group box.
pub(super) fn group_box(ui: &mut Ui, title: &str, add_contents: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, theme::panel_hi()))
        .corner_radius(6)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong());
            ui.add_space(ROW_GAP);
            add_contents(ui);
        });
    ui.add_space(GROUP_GAP);
}

/// Width of the label column: the widest field label of the rows shown, so every group lines up.
fn label_width(ui: &Ui, rows: &[&Row]) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    rows.iter()
        .filter(|r| r.kind == Kind::Field)
        .map(|r| {
            ui.painter()
                .layout_no_wrap(
                    t(r.label).to_owned(),
                    font.clone(),
                    egui::Color32::PLACEHOLDER,
                )
                .size()
                .x
        })
        .fold(0.0, f32::max)
        .ceil()
}

/// Rows as group boxes; field rows are label | control, check rows span both columns.
/// In search results (`searching`) the box titles read "Category › Group".
pub(super) fn form(ui: &mut Ui, rows: &[&Row], form: &mut Form, searching: bool) {
    let label_w = label_width(ui, rows);
    for group in rows.chunk_by(|a, b| a.category == b.category && a.group == b.group) {
        let heading = group_heading(group[0], searching);
        group_box(ui, &heading, |ui| group_rows(ui, group, form, label_w));
    }
}

/// The rows inside one group box.
fn group_rows(ui: &mut Ui, rows: &[&Row], form: &mut Form, label_w: f32) {
    ui.spacing_mut().item_spacing.y = ROW_GAP;
    for r in rows {
        form_row(ui, r, form, label_w);
    }
}

/// The title of the group box `row` sits in.
fn group_heading(row: &Row, searching: bool) -> String {
    if !searching {
        return t(row.group).to_owned();
    }
    fx_app::tr(
        "settings.search_group",
        &[("category", row.category.label()), ("group", t(row.group))],
    )
}

/// One row of the form: a check row spans both columns, a field row is label | control.
fn form_row(ui: &mut Ui, r: &Row, form: &mut Form, label_w: f32) {
    let label = t(r.label);
    if r.kind == Kind::Check {
        ui.vertical(|ui| (r.show)(ui, form, label));
        return;
    }
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = COLUMN_GAP;
        ui.allocate_ui_with_layout(
            egui::vec2(label_w, CONTROL_HEIGHT),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_min_size(egui::vec2(label_w, CONTROL_HEIGHT));
                ui.label(label);
            },
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            (r.show)(ui, form, label);
        });
    });
}

/// A sidebar category entry: full-width 26 px row, rounded fill when selected or hovered.
pub(super) fn category_row(
    ui: &mut Ui,
    label: &str,
    selected: bool,
    dimmed: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), SIDEBAR_ROW),
        Sense::click(),
    );
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, label));
    let fill = if selected {
        Some(theme::selection_bg())
    } else if resp.hovered() {
        Some(theme::panel_hi())
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, 6.0, fill);
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect,
            6.0,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().text(
        rect.left_center() + egui::vec2(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Body.resolve(ui.style()),
        if dimmed { theme::dim() } else { theme::text() },
    );
    resp
}
