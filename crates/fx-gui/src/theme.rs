//! Nord dark and light themes, compact spacing, and the age-bucket color mapping.
//! Colors are functions because they follow the active theme.

use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke, TextStyle};
use fx_app::{AgeBucket, ThemeChoice};
use std::sync::atomic::{AtomicBool, Ordering};

struct Palette {
    bg: Color32,
    panel: Color32,
    panel_hi: Color32,
    border: Color32,
    text: Color32,
    dim: Color32,
    accent: Color32,
    header_text: Color32,
    selected: Color32,
    error: Color32,
    faint: Color32,
    separator: Color32,
    selection_bg: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const DARK: Palette = Palette {
    bg: rgb(0x2E3440),
    panel: rgb(0x3B4252),
    panel_hi: rgb(0x434C5E),
    border: rgb(0x4C566A),
    text: rgb(0xD8DEE9),
    dim: rgb(0x9AA5B8),
    accent: rgb(0x88C0D0),
    header_text: rgb(0xECEFF4),
    selected: rgb(0x4C566A),
    error: rgb(0xBF616A),
    faint: rgb(0x343B4A),
    separator: rgb(0x444C5E),
    selection_bg: rgb(0x4C6A7A),
};

const LIGHT: Palette = Palette {
    bg: rgb(0xECEFF4),
    panel: rgb(0xE5E9F0),
    panel_hi: rgb(0xD8DEE9),
    border: rgb(0xC0C8D6),
    text: rgb(0x2E3440),
    dim: rgb(0x66708A),
    accent: rgb(0x4C7399),
    header_text: rgb(0x2E3440),
    selected: rgb(0xCBD8E8),
    error: rgb(0xB0414C),
    faint: rgb(0xE2E6EE),
    separator: rgb(0xD3D9E4),
    selection_bg: rgb(0xB4CAE0),
};

static LIGHT_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn is_light() -> bool {
    LIGHT_ACTIVE.load(Ordering::Relaxed)
}

fn palette() -> &'static Palette {
    if LIGHT_ACTIVE.load(Ordering::Relaxed) {
        &LIGHT
    } else {
        &DARK
    }
}

macro_rules! colors {
    ($($name:ident),*) => {$(
        pub fn $name() -> Color32 {
            palette().$name
        }
    )*};
}
colors!(
    bg,
    panel_hi,
    border,
    text,
    dim,
    accent,
    header_text,
    selected,
    error
);

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

pub fn apply(ctx: &egui::Context, choice: ThemeChoice) {
    let light = choice == ThemeChoice::Light;
    LIGHT_ACTIVE.store(light, Ordering::Relaxed);
    let (theme, p, base) = if light {
        (egui::Theme::Light, &LIGHT, egui::Visuals::light())
    } else {
        (egui::Theme::Dark, &DARK, egui::Visuals::dark())
    };
    let mut style = (*ctx.style_of(theme)).clone();
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
    *v = base;
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.faint;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.selection_bg;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.window_stroke = Stroke::new(1.0, p.border);
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
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.separator);
    v.widgets.inactive.weak_bg_fill = p.panel_hi;
    v.widgets.inactive.bg_fill = p.panel_hi;
    v.widgets.hovered.weak_bg_fill = p.border;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.active.weak_bg_fill = p.border;
    v.widgets.open.weak_bg_fill = p.border;
    ctx.set_theme(theme);
    ctx.set_style_of(theme, style);
}
