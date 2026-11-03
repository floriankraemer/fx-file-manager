//! Crisp vector entry icons, painted so no icon font is needed.

use crate::theme;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use fx_core::EntryKind;

const SIZE: Vec2 = vec2(16.0, 14.0);
const FOLDER: Color32 = Color32::from_rgb(0xEB, 0xCB, 0x8B);

pub fn entry(ui: &mut Ui, kind: EntryKind) {
    let (rect, _) = ui.allocate_exact_size(SIZE, Sense::hover());
    let rect = Rect::from_center_size(rect.center().round(), SIZE);
    match kind {
        EntryKind::Dir => folder(ui, rect),
        EntryKind::Symlink => {
            file(ui, rect, theme::accent());
            arrow(ui, rect);
        }
        EntryKind::File | EntryKind::Other => file(ui, rect, theme::text()),
    }
}

fn folder(ui: &Ui, r: Rect) {
    let p = ui.painter();
    let tab = Rect::from_min_size(r.min + vec2(0.5, 1.0), vec2(6.0, 4.0));
    p.rect_filled(tab, 1.5, FOLDER.gamma_multiply(0.8));
    let body = Rect::from_min_max(r.min + vec2(0.5, 3.0), r.max - vec2(0.5, 1.0));
    p.rect_filled(body, 2.0, FOLDER);
}

fn file(ui: &Ui, r: Rect, color: Color32) {
    let (l, t, rt, b) = (
        r.left() + 2.5,
        r.top() + 0.5,
        r.right() - 2.5,
        r.bottom() - 0.5,
    );
    let fold = 4.0;
    let outline: Vec<Pos2> = vec![
        pos2(l, t),
        pos2(rt - fold, t),
        pos2(rt, t + fold),
        pos2(rt, b),
        pos2(l, b),
    ];
    let stroke = Stroke::new(1.2, color);
    let p = ui.painter();
    p.add(Shape::convex_polygon(
        outline,
        color.gamma_multiply(0.12),
        stroke,
    ));
    p.line_segment([pos2(rt - fold, t), pos2(rt - fold, t + fold)], stroke);
    p.line_segment([pos2(rt - fold, t + fold), pos2(rt, t + fold)], stroke);
}

/// Small up-right arrow over the file's lower half.
fn arrow(ui: &Ui, r: Rect) {
    let stroke = Stroke::new(1.4, theme::accent());
    let c = pos2(r.center().x, r.bottom() - 4.5);
    let (a, b) = (c + vec2(-2.0, 2.0), c + vec2(2.0, -2.0));
    let p = ui.painter();
    p.line_segment([a, b], stroke);
    p.line_segment([b + vec2(-3.0, 0.0), b], stroke);
    p.line_segment([b, b + vec2(0.0, 3.0)], stroke);
}

/// Command-bar glyphs, drawn from strokes so they render without any icon font.
#[derive(Clone, Copy)]
pub enum Glyph {
    NewFolder,
    Rename,
    Delete,
    Sort,
    View,
    More,
    Chevron,
}

/// Paints `glyph` centered in `rect` (a 16x16 area).
pub fn glyph(ui: &Ui, rect: Rect, glyph: Glyph, color: Color32) {
    let p = ui.painter();
    let c = rect.center();
    let s = Stroke::new(1.3, color);
    let at = |x: f32, y: f32| c + vec2(x, y);
    match glyph {
        Glyph::NewFolder => {
            let body = Rect::from_min_max(at(-7.0, -4.0), at(5.0, 5.0));
            p.rect_stroke(body, 1.5, s, egui::StrokeKind::Inside);
            p.line_segment([at(-7.0, -4.0), at(-7.0, -6.0)], s);
            p.line_segment([at(-7.0, -6.0), at(-2.5, -6.0)], s);
            p.line_segment([at(-2.5, -6.0), at(-1.0, -4.0)], s);
            let plus = Stroke::new(1.5, theme::accent());
            p.line_segment([at(5.0, 0.0), at(5.0, 7.0)], plus);
            p.line_segment([at(1.5, 3.5), at(8.5, 3.5)], plus);
        }
        Glyph::Rename => {
            let field = Rect::from_min_max(at(-8.0, -4.5), at(8.0, 4.5));
            p.rect_stroke(field, 2.0, s, egui::StrokeKind::Inside);
            p.line_segment(
                [at(0.0, -3.0), at(0.0, 3.0)],
                Stroke::new(1.5, theme::accent()),
            );
            p.line_segment([at(-1.8, -3.0), at(1.8, -3.0)], s);
            p.line_segment([at(-1.8, 3.0), at(1.8, 3.0)], s);
        }
        Glyph::Delete => {
            p.line_segment([at(-6.0, -4.5), at(6.0, -4.5)], s);
            p.line_segment([at(-2.0, -4.5), at(-2.0, -6.5)], s);
            p.line_segment([at(2.0, -4.5), at(2.0, -6.5)], s);
            p.line_segment([at(-2.0, -6.5), at(2.0, -6.5)], s);
            let can = vec![at(-4.5, -4.5), at(4.5, -4.5), at(3.8, 6.5), at(-3.8, 6.5)];
            p.add(Shape::closed_line(can, s));
            p.line_segment([at(-1.5, -2.0), at(-1.5, 4.0)], s);
            p.line_segment([at(1.5, -2.0), at(1.5, 4.0)], s);
        }
        Glyph::Sort => {
            for (i, half) in [7.0, 5.0, 3.0].into_iter().enumerate() {
                let y = -4.5 + 4.5 * i as f32;
                p.line_segment([at(-8.0, y), at(-8.0 + half * 1.6, y)], s);
            }
            let x = 6.0;
            p.line_segment([at(x, -6.0), at(x, 5.0)], s);
            p.line_segment([at(x - 2.5, 2.5), at(x, 5.5)], s);
            p.line_segment([at(x + 2.5, 2.5), at(x, 5.5)], s);
        }
        Glyph::View => {
            let frame = Rect::from_min_max(at(-7.5, -6.0), at(7.5, 6.0));
            p.rect_stroke(frame, 2.0, s, egui::StrokeKind::Inside);
            p.line_segment([at(-7.5, -2.5), at(7.5, -2.5)], s);
            p.line_segment([at(-3.0, -2.5), at(-3.0, 6.0)], s);
        }
        Glyph::More => {
            for dx in [-5.0, 0.0, 5.0] {
                p.circle_filled(at(dx, 0.0), 1.5, color);
            }
        }
        Glyph::Chevron => {
            p.line_segment([at(-3.0, -1.5), at(0.0, 1.5)], s);
            p.line_segment([at(3.0, -1.5), at(0.0, 1.5)], s);
        }
    }
}
