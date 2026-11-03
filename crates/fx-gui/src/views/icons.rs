//! Crisp vector entry icons, painted so no icon font is needed.

use crate::theme;
use eframe::egui::{Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use fx_core::EntryKind;

const SIZE: Vec2 = vec2(16.0, 14.0);
const FOLDER: Color32 = Color32::from_rgb(0xEB, 0xCB, 0x8B);

pub fn entry(ui: &mut Ui, kind: EntryKind) {
    let (rect, _) = ui.allocate_exact_size(SIZE, Sense::hover());
    let rect = Rect::from_center_size(rect.center().round(), SIZE);
    match kind {
        EntryKind::Dir => folder(ui, rect),
        EntryKind::Symlink => {
            file(ui, rect, theme::ACCENT);
            arrow(ui, rect);
        }
        EntryKind::File | EntryKind::Other => file(ui, rect, theme::TEXT),
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
    let stroke = Stroke::new(1.4, theme::ACCENT);
    let c = pos2(r.center().x, r.bottom() - 4.5);
    let (a, b) = (c + vec2(-2.0, 2.0), c + vec2(2.0, -2.0));
    let p = ui.painter();
    p.line_segment([a, b], stroke);
    p.line_segment([b + vec2(-3.0, 0.0), b], stroke);
    p.line_segment([b, b + vec2(0.0, 3.0)], stroke);
}
