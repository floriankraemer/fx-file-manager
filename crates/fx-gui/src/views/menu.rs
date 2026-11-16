//! The one menu-entry convention: every menu entry is built with these helpers.
//!
//! Menu bar menus, context menus and popups all use `item`, `check` and `radio`.
//! Each applies the same padding and renders as a full-width button that is flat until hovered,
//! so entries look identical wherever they appear.
//! Never use `ui.checkbox`, `ui.radio` or a bare `Button` inside a menu.

use eframe::egui::{self, Atom, Button, Color32, Response, Stroke, Style, Ui, Vec2, pos2, vec2};

/// Padding of top-level menu bar buttons (egui's default is 2x0).
pub const BAR_PADDING: Vec2 = vec2(8.0, 4.0);
/// Padding inside every menu entry.
pub const ENTRY_PADDING: Vec2 = vec2(12.0, 5.0);
/// Vertical gap between menu entries.
const ENTRY_SPACING_Y: f32 = 2.0;
/// Width of the leading check/radio indicator slot.
const INDICATOR: f32 = 14.0;

/// Style for the top-level menu bar buttons.
pub fn bar_style(style: &mut Style) {
    egui::containers::menu::menu_style(style);
    style.spacing.button_padding = BAR_PADDING;
}

/// A flat button for bars (menu bar, tab strip, toolbar): `BAR_PADDING`, no frame until hovered.
pub fn bar_button(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    ui.scope(|ui| {
        ui.style_mut().spacing.button_padding = BAR_PADDING;
        ui.add_enabled(enabled, Button::new(label).frame_when_inactive(false))
    })
    .inner
}

/// A menu entry with a right-aligned shortcut hint. Returns true when clicked.
pub fn item(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool) -> bool {
    item_at(ui, label, shortcut, enabled, false).clicked()
}

/// An entry that can be highlighted, returning its response so the caller can scroll to or click it.
pub fn item_at(
    ui: &mut Ui,
    label: &str,
    shortcut: &str,
    enabled: bool,
    selected: bool,
) -> Response {
    entry(
        ui,
        Button::new(label)
            .shortcut_text(shortcut)
            .selected(selected),
        enabled,
    )
}

/// A checkable entry; the caller toggles its state when this returns true.
pub fn check(ui: &mut Ui, label: &str, checked: bool) -> bool {
    let mark = indicator(checked, |painter, rect, color| {
        let c = rect.center();
        let stroke = Stroke::new(1.6, color);
        painter.line_segment([pos2(c.x - 4.0, c.y), pos2(c.x - 1.5, c.y + 3.0)], stroke);
        painter.line_segment(
            [pos2(c.x - 1.5, c.y + 3.0), pos2(c.x + 4.0, c.y - 3.5)],
            stroke,
        );
    });
    entry(ui, Button::new((mark, label)), true).clicked()
}

/// A radio entry; the caller applies the choice when this returns true.
pub fn radio(ui: &mut Ui, label: &str, selected: bool) -> bool {
    let dot = indicator(selected, |painter, rect, color| {
        painter.circle_filled(rect.center(), 3.0, color);
    });
    entry(ui, Button::new((dot, label)), true).clicked()
}

fn indicator<'a>(on: bool, draw: impl Fn(&egui::Painter, egui::Rect, Color32) + 'a) -> Atom<'a> {
    Atom::paint(vec2(INDICATOR, INDICATOR), move |ui, args| {
        if on {
            draw(ui.painter(), args.rect, args.fallback_text_color);
        }
    })
}

fn entry(ui: &mut Ui, button: Button<'_>, enabled: bool) -> Response {
    ui.scope(|ui| {
        let style = ui.style_mut();
        style.spacing.button_padding = ENTRY_PADDING;
        style.spacing.item_spacing.y = ENTRY_SPACING_Y;
        ui.add_enabled(enabled, button.frame_when_inactive(false))
    })
    .inner
}
