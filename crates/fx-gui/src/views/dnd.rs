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

/// A place a drag can land: a folder row or a whole window's current folder, in global coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct DropArea {
    pub viewport: ViewportId,
    pub folder: PathBuf,
    pub rect: Rect,
}

/// The area under `pointer` that a drag from `source` lands on. Folder rows win over the window
/// they sit in. Returns `None` outside every area, inside `source`, or when the folder is one of `sources`.
pub fn drop_target(
    pointer: Pos2,
    rows: &[DropArea],
    windows: &[DropArea],
    source: ViewportId,
    sources: &[PathBuf],
) -> Option<DropArea> {
    let area = rows
        .iter()
        .rev()
        .find(|a| a.viewport != source && a.rect.contains(pointer))
        .or_else(|| {
            windows
                .iter()
                .find(|a| a.viewport != source && a.rect.contains(pointer))
        })?;
    if sources.iter().any(|src| area.folder.starts_with(src)) {
        return None;
    }
    Some(area.clone())
}

const AREAS_KEY: &str = "dnd-drop-areas";

/// Registers the folder row `resp` shows as a drop area, for drags from any window.
/// Rows register every frame, so the list is cleared per window in `forget_areas`.
pub fn record_row(resp: &Response, folder: &Path) {
    let ctx = &resp.ctx;
    let viewport = ctx.viewport_id();
    let Some(origin) = content_rect(ctx, viewport) else {
        return;
    };
    let area = DropArea {
        viewport,
        folder: folder.to_path_buf(),
        rect: resp.interact_rect.translate(origin.min.to_vec2()),
    };
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<DropArea>>(Id::unique(AREAS_KEY))
            .push(area)
    });
}

/// Drops the folder rows `ctx`'s viewport registered in the previous frame.
pub fn forget_areas(ctx: &Context) {
    let viewport = ctx.viewport_id();
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<DropArea>>(Id::unique(AREAS_KEY))
            .retain(|a| a.viewport != viewport);
    });
}

/// The folder rows registered by all windows.
pub fn rows(ctx: &Context) -> Vec<DropArea> {
    ctx.data(|d| d.get_temp::<Vec<DropArea>>(Id::unique(AREAS_KEY)))
        .unwrap_or_default()
}

/// Paths of files dropped from outside fx into the window this frame belongs to.
pub fn os_dropped_paths(ctx: &Context) -> Vec<PathBuf> {
    ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|file| file.path().to_path_buf())
            .collect()
    })
}

/// True while files from outside fx hover this window.
pub fn os_hovering(ctx: &Context) -> bool {
    ctx.input(|i| !i.raw.hovered_files.is_empty())
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
    record_row(resp, folder);
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

/// Outlines `area` (global coordinates) in this window.
pub fn highlight(ctx: &Context, area: Rect) {
    let origin = content_rect(ctx, ctx.viewport_id()).map_or(Pos2::ZERO, |r| r.min);
    let painter = ctx.layer_painter(egui::LayerId::new(
        Order::Foreground,
        Id::unique("drop-highlight"),
    ));
    painter.rect_stroke(
        area.translate(-origin.to_vec2()).shrink(2.0),
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

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1))
    }

    fn area(viewport: &str, folder: &str, rect: Rect) -> DropArea {
        DropArea {
            viewport: ViewportId::from_hash_of(viewport),
            folder: PathBuf::from(folder),
            rect,
        }
    }

    #[test]
    fn folder_row_of_another_window_is_the_target() {
        let rows = [area("b", "/b/sub", rect(250.0, 40.0, 290.0, 60.0))];
        let windows = [area("b", "/b", rect(200.0, 0.0, 300.0, 100.0))];
        let hit = drop_target(
            egui::pos2(260.0, 50.0),
            &rows,
            &windows,
            ViewportId::ROOT,
            &[],
        );
        assert_eq!(hit.map(|a| a.folder), Some(PathBuf::from("/b/sub")));
    }

    #[test]
    fn other_window_outside_its_rows_targets_its_folder() {
        let rows = [area("b", "/b/sub", rect(250.0, 40.0, 290.0, 60.0))];
        let windows = [area("b", "/b", rect(200.0, 0.0, 300.0, 100.0))];
        let hit = drop_target(
            egui::pos2(210.0, 90.0),
            &rows,
            &windows,
            ViewportId::ROOT,
            &[],
        );
        assert_eq!(hit.map(|a| a.folder), Some(PathBuf::from("/b")));
    }

    #[test]
    fn source_window_is_never_the_target() {
        let rows = [area("a", "/a/sub", rect(10.0, 10.0, 50.0, 30.0))];
        let windows = [area("a", "/a", rect(0.0, 0.0, 100.0, 100.0))];
        let hit = drop_target(
            egui::pos2(20.0, 20.0),
            &rows,
            &windows,
            ViewportId::from_hash_of("a"),
            &[],
        );
        assert_eq!(hit, None);
    }

    #[test]
    fn pointer_outside_all_areas_has_no_target() {
        let windows = [area("b", "/b", rect(200.0, 0.0, 300.0, 100.0))];
        let hit = drop_target(
            egui::pos2(150.0, 50.0),
            &[],
            &windows,
            ViewportId::ROOT,
            &[],
        );
        assert_eq!(hit, None);
    }

    #[test]
    fn folder_inside_a_dragged_path_is_no_target() {
        let rows = [area("b", "/a/x/inner", rect(250.0, 40.0, 290.0, 60.0))];
        let windows = [area("b", "/b", rect(200.0, 0.0, 300.0, 100.0))];
        let hit = drop_target(
            egui::pos2(260.0, 50.0),
            &rows,
            &windows,
            ViewportId::ROOT,
            &[PathBuf::from("/a/x")],
        );
        assert_eq!(hit, None);
    }

    #[test]
    fn negative_coordinates_hit_a_window_left_of_origin() {
        let windows = [area("b", "/b", rect(-300.0, 0.0, -100.0, 100.0))];
        let hit = drop_target(
            egui::pos2(-150.0, 50.0),
            &[],
            &windows,
            ViewportId::ROOT,
            &[],
        );
        assert_eq!(hit.map(|a| a.folder), Some(PathBuf::from("/b")));
    }
}
