//! Nord-ish dark theme, compact spacing, and the age-bucket color mapping.

use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke, TextStyle};
use fx_app::AgeBucket;

pub const BG: Color32 = Color32::from_rgb(0x2E, 0x34, 0x40);
pub const PANEL: Color32 = Color32::from_rgb(0x3B, 0x42, 0x52);
pub const PANEL_HI: Color32 = Color32::from_rgb(0x43, 0x4C, 0x5E);
pub const BORDER: Color32 = Color32::from_rgb(0x4C, 0x56, 0x6A);
pub const TEXT: Color32 = Color32::from_rgb(0xD8, 0xDE, 0xE9);
pub const DIM: Color32 = Color32::from_rgb(0x9A, 0xA5, 0xB8);
pub const ACCENT: Color32 = Color32::from_rgb(0x88, 0xC0, 0xD0);
pub const HEADER_TEXT: Color32 = Color32::from_rgb(0xEC, 0xEF, 0xF4);
pub const SELECTED: Color32 = Color32::from_rgb(0x4C, 0x56, 0x6A);
pub const ERROR: Color32 = Color32::from_rgb(0xBF, 0x61, 0x6A);

pub fn age_color(bucket: AgeBucket) -> Color32 {
    match bucket {
        AgeBucket::Minutes => Color32::from_rgb(0xBF, 0x61, 0x6A),
        AgeBucket::Hours => Color32::from_rgb(0xD0, 0x87, 0x70),
        AgeBucket::Days => Color32::from_rgb(0xAE, 0xC0, 0x7A),
        AgeBucket::Weeks => Color32::from_rgb(0x8F, 0xBC, 0xBB),
        AgeBucket::Months => Color32::from_rgb(0x81, 0xA1, 0xC1),
        AgeBucket::Years => Color32::from_rgb(0x6B, 0x75, 0x89),
    }
}

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Body, FontId::proportional(13.5)),
        (TextStyle::Button, FontId::proportional(13.5)),
        (TextStyle::Heading, FontId::proportional(16.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(6.0, 3.0);
    style.spacing.button_padding = egui::vec2(6.0, 2.0);
    style.spacing.interact_size.y = 20.0;

    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = Color32::from_rgb(0x34, 0x3B, 0x4A);
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = Color32::from_rgb(0x4C, 0x6A, 0x7A);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    let radius = CornerRadius::same(3);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = radius;
    }
    // Subtle: also drives the table's column separator lines.
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x44, 0x4C, 0x5E));
    v.widgets.inactive.weak_bg_fill = PANEL_HI;
    v.widgets.inactive.bg_fill = PANEL_HI;
    v.widgets.hovered.weak_bg_fill = BORDER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.weak_bg_fill = BORDER;
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_style_of(egui::Theme::Dark, style);
}
