//! Dragging list entries onto folders in the list or the tree, within one window.
//! The drop moves; holding the `drag_copy_modifier` setting's key copies instead.

use crate::theme;
use eframe::egui::{self, Area, Context, CursorIcon, DragAndDrop, Id, Order, Response, Stroke};
use fx_app::{DragModifier, tn};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The drag payload: absolute paths of the dragged entries.
pub struct Dragged(pub Vec<PathBuf>);

pub fn start(ctx: &Context, paths: Vec<PathBuf>) {
    if !paths.is_empty() {
        DragAndDrop::set_payload(ctx, Dragged(paths));
    }
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
    if hovering.0.iter().any(|src| folder.starts_with(src)) {
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
    let text = tn(key, dragged.0.len() as u64, &[]);
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
