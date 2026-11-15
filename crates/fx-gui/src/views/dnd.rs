//! Dragging list entries onto folders in the list or the tree, and between windows.
//! The drop moves; holding the `drag_copy_modifier` setting's key copies instead.
//! Dropping on another window copies or moves into that window's folder.

use crate::theme;
use eframe::egui::{
    self, Area, Context, CursorIcon, DragAndDrop, Id, Order, Pos2, Rect, Response, Stroke,
    ViewportId,
};
use fx_app::{DragModifier, WindowId, tn};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The drag payload: absolute paths of the dragged entries and the window they come from.
pub struct Dragged {
    pub paths: Vec<PathBuf>,
    pub from: WindowId,
}

pub fn start(ctx: &Context, from: WindowId, paths: Vec<PathBuf>) {
    if !paths.is_empty() {
        DragAndDrop::set_payload(ctx, Dragged { paths, from });
    }
}

/// The window that `pointer` (in global coordinates) is over, ignoring `source`.
pub fn window_at(
    pointer: Pos2,
    windows: &[(WindowId, Rect)],
    source: WindowId,
) -> Option<WindowId> {
    windows
        .iter()
        .find(|(id, rect)| *id != source && rect.contains(pointer))
        .map(|(id, _)| *id)
}

/// The pointer in global coordinates, as seen by `viewport`; `None` when it has no position there.
pub fn global_pointer(ctx: &Context, viewport: ViewportId) -> Option<Pos2> {
    ctx.input_for(viewport, |i| {
        let inner = i.viewport().inner_rect?;
        Some(inner.min + i.pointer.latest_pos()?.to_vec2())
    })
}

/// The global content rectangle of `viewport`, if the platform reports it.
pub fn content_rect(ctx: &Context, viewport: ViewportId) -> Option<Rect> {
    ctx.input_for(viewport, |i| i.viewport().inner_rect)
}

/// True in the frame `viewport` saw the pointer button released.
pub fn released(ctx: &Context, viewport: ViewportId) -> bool {
    ctx.input_for(viewport, |i| i.pointer.any_released())
}

/// True while the copy modifier is held.
pub fn copying(ctx: &Context, modifier: DragModifier) -> bool {
    ctx.input(|i| match modifier {
        DragModifier::Ctrl => i.modifiers.ctrl,
        DragModifier::Shift => i.modifiers.shift,
        DragModifier::Alt => i.modifiers.alt,
    })
}

/// Highlights `resp` while entries that may land in `folder` hover it.
/// Returns them when they are dropped there.
pub fn target(resp: &Response, folder: &Path) -> Option<Arc<Dragged>> {
    let hovering = resp.dnd_hover_payload::<Dragged>()?;
    // A folder never lands in itself or its own subfolder.
    if hovering.paths.iter().any(|src| folder.starts_with(src)) {
        return None;
    }
    let painter = egui::Painter::new(resp.ctx.clone(), resp.layer_id, resp.interact_rect);
    painter.rect_stroke(
        resp.rect.shrink(1.0),
        4.0,
        Stroke::new(2.0, theme::accent()),
        egui::StrokeKind::Inside,
    );
    resp.dnd_release_payload::<Dragged>()
}

/// Outlines the window a drag from another window hovers.
pub fn highlight_window(ctx: &Context) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        Order::Foreground,
        Id::unique("cross-window-drop"),
    ));
    painter.rect_stroke(
        ctx.content_rect().shrink(2.0),
        4.0,
        Stroke::new(2.0, theme::accent()),
        egui::StrokeKind::Inside,
    );
}

/// Labels the pointer with what a drop would do, while a drag is in progress.
pub fn feedback(ctx: &Context, modifier: DragModifier) {
    let Some(dragged) = DragAndDrop::payload::<Dragged>(ctx) else {
        return;
    };
    let Some(pos) = ctx.pointer_interact_pos() else {
        return;
    };
    let copy = copying(ctx, modifier);
    ctx.set_cursor_icon(if copy {
        CursorIcon::Copy
    } else {
        CursorIcon::Grabbing
    });
    let key = if copy { "drag.copy" } else { "drag.move" };
    let text = tn(key, dragged.paths.len() as u64, &[]);
    Area::new(Id::unique("drag-feedback"))
        .order(Order::Tooltip)
        .fixed_pos(pos + egui::vec2(16.0, 12.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(text);
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> egui::Rect {
        egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1))
    }

    #[test]
    fn pointer_over_another_window_finds_it() {
        let windows = [
            (1, rect(0.0, 0.0, 100.0, 100.0)),
            (2, rect(200.0, 0.0, 300.0, 100.0)),
        ];
        assert_eq!(window_at(egui::pos2(250.0, 50.0), &windows, 1), Some(2));
    }

    #[test]
    fn pointer_outside_all_windows_finds_none() {
        let windows = [
            (1, rect(0.0, 0.0, 100.0, 100.0)),
            (2, rect(200.0, 0.0, 300.0, 100.0)),
        ];
        assert_eq!(window_at(egui::pos2(150.0, 50.0), &windows, 1), None);
    }

    #[test]
    fn source_window_is_never_the_target() {
        let windows = [(1, rect(0.0, 0.0, 100.0, 100.0))];
        assert_eq!(window_at(egui::pos2(50.0, 50.0), &windows, 1), None);
    }

    #[test]
    fn negative_coordinates_hit_a_window_left_of_origin() {
        let windows = [
            (1, rect(0.0, 0.0, 100.0, 100.0)),
            (2, rect(-300.0, 0.0, -100.0, 100.0)),
        ];
        assert_eq!(window_at(egui::pos2(-150.0, 50.0), &windows, 1), Some(2));
    }
}
