//! Dragging list entries onto folders in the tree, and between windows.
//! A drop on the listing lands in its active folder; a drop on the tree lands in the folder under the pointer.
//! The drop moves; holding the `drag_copy_modifier` setting's key copies instead.

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

/// Hands the dragged files to the operating system, so programs other than fx can receive them.
pub fn start_external(paths: Vec<PathBuf>) {
    #[cfg(all(unix, not(target_os = "macos")))]
    super::xdnd::start(paths);
    #[cfg(any(windows, target_os = "macos"))]
    super::native_drag::start(paths);
}

/// A place a drag can land: a tree folder or a window's listing (its active folder), in global coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct DropArea {
    pub viewport: ViewportId,
    pub folder: PathBuf,
    pub rect: Rect,
}

/// The area under `pointer` that a drag from `source` lands on. A tree folder wins over the listing
/// of the window it sits in. Returns `None` outside every area, inside `source`, or when the folder is one of `sources`.
pub fn drop_target(
    pointer: Pos2,
    tree: &[DropArea],
    windows: &[DropArea],
    source: Option<ViewportId>,
    sources: &[PathBuf],
) -> Option<DropArea> {
    let area = tree
        .iter()
        .rev()
        .find(|a| Some(a.viewport) != source && a.rect.contains(pointer))
        .or_else(|| {
            windows
                .iter()
                .find(|a| Some(a.viewport) != source && a.rect.contains(pointer))
        })?;
    if sources.iter().any(|src| area.folder.starts_with(src)) {
        return None;
    }
    Some(area.clone())
}

const AREAS_KEY: &str = "dnd-drop-areas";
const TRACK_KEY: &str = "dnd-track-areas";

/// Other windows may start a drag while this one idles, so with several windows the areas are always kept.
pub fn track_always(ctx: &Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(Id::unique(TRACK_KEY), on));
}

/// Whether drop areas matter now: a drag is under way, files hover from outside, or other windows exist.
pub fn recording(ctx: &Context) -> bool {
    DragAndDrop::has_payload_of_type::<Dragged>(ctx)
        || os_hovering(ctx)
        || ctx
            .data(|d| d.get_temp(Id::unique(TRACK_KEY)))
            .unwrap_or(false)
}

/// Registers the tree folder `resp` shows as a drop area, for drags from any window.
/// Tree folders register every frame, so they are dropped per window in `forget_areas`.
pub fn record_tree_folder(resp: &Response, folder: &Path) {
    let ctx = &resp.ctx;
    if !recording(ctx) {
        return;
    }
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

/// Drops the tree folders `ctx`'s viewport registered in the previous frame.
pub fn forget_areas(ctx: &Context) {
    let viewport = ctx.viewport_id();
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<DropArea>>(Id::unique(AREAS_KEY))
            .retain(|a| a.viewport != viewport);
    });
}

/// The tree folders registered by all windows.
pub fn tree_folders(ctx: &Context) -> Vec<DropArea> {
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

/// Converts physical screen pixels to egui's global points. macOS reports points already.
#[cfg(not(target_os = "macos"))]
pub fn to_points(pixels: (i32, i32), pixels_per_point: f32) -> Pos2 {
    egui::pos2(
        pixels.0 as f32 / pixels_per_point,
        pixels.1 as f32 / pixels_per_point,
    )
}

/// Where the cursor is on screen, in points. Files dragged in from other programs carry no
/// position in winit's events, so this reads the cursor directly. Wayland has no such read.
pub fn screen_pointer(ctx: &Context) -> Option<Pos2> {
    platform::cursor(ctx.pixels_per_point())
}

#[cfg(windows)]
mod platform {
    use super::to_points;
    use eframe::egui::Pos2;
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    pub fn cursor(pixels_per_point: f32) -> Option<Pos2> {
        let mut point = POINT { x: 0, y: 0 };
        // SAFETY: GetCursorPos writes one POINT through the valid pointer it is given.
        let ok = unsafe { GetCursorPos(&mut point) } != 0;
        ok.then(|| to_points((point.x, point.y), pixels_per_point))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use eframe::egui::{Pos2, pos2};
    use std::ffi::c_void;

    #[repr(C)]
    struct CgPoint {
        x: f64,
        y: f64,
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventGetLocation(event: *const c_void) -> CgPoint;
        fn CFRelease(cf: *const c_void);
    }

    /// Quartz reports the cursor in points with a top-left origin, as egui's global coordinates.
    pub fn cursor(_pixels_per_point: f32) -> Option<Pos2> {
        // SAFETY: a null source asks for a fresh event at the cursor; it is released after reading.
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            if event.is_null() {
                return None;
            }
            let at = CGEventGetLocation(event);
            CFRelease(event);
            Some(pos2(at.x as f32, at.y as f32))
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::to_points;
    use eframe::egui::Pos2;
    use std::sync::OnceLock;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;
    use x11rb::rust_connection::RustConnection;

    /// The X connection and the root window of its first screen, opened once.
    static X: OnceLock<Option<(RustConnection, u32)>> = OnceLock::new();

    /// Under Wayland the X server only knows the pointer over its own windows, so its reading goes stale.
    pub fn cursor(pixels_per_point: f32) -> Option<Pos2> {
        let wayland = std::env::var("WAYLAND_DISPLAY").ok();
        let display = std::env::var("DISPLAY").ok();
        if !x11_session(wayland.as_deref(), display.as_deref()) {
            return None;
        }
        let pixels = query_x_pointer()?;
        Some(to_points(pixels, pixels_per_point))
    }

    fn x11_session(wayland: Option<&str>, display: Option<&str>) -> bool {
        display.is_some() && wayland.is_none()
    }

    fn query_x_pointer() -> Option<(i32, i32)> {
        let x = X.get_or_init(|| {
            let (conn, screen) = x11rb::connect(None).ok()?;
            let root = conn.setup().roots.get(screen)?.root;
            Some((conn, root))
        });
        let (conn, root) = x.as_ref()?;
        let reply = conn.query_pointer(*root).ok()?.reply().ok()?;
        Some((i32::from(reply.root_x), i32::from(reply.root_y)))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn x11_is_used_only_outside_wayland() {
            assert!(x11_session(None, Some(":0")));
            assert!(!x11_session(Some("wayland-0"), Some(":0")));
            assert!(!x11_session(None, None));
        }

        #[test]
        fn x_server_reports_the_pointer_when_one_is_running() {
            if x11rb::connect(None).is_err() {
                return;
            }
            assert!(query_x_pointer().is_some());
        }
    }
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
    record_tree_folder(resp, folder);
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
    fn tree_folder_of_another_window_is_the_target() {
        let tree = [area("b", "/b/sub", rect(250.0, 40.0, 290.0, 60.0))];
        let windows = [area("b", "/b", rect(200.0, 0.0, 300.0, 100.0))];
        let hit = drop_target(
            egui::pos2(260.0, 50.0),
            &tree,
            &windows,
            Some(ViewportId::ROOT),
            &[],
        );
        assert_eq!(hit.map(|a| a.folder), Some(PathBuf::from("/b/sub")));
    }

    #[test]
    fn other_window_outside_its_rows_targets_its_folder() {
        let tree = [area("b", "/b/sub", rect(250.0, 40.0, 290.0, 60.0))];
        let windows = [area("b", "/b", rect(200.0, 0.0, 300.0, 100.0))];
        let hit = drop_target(
            egui::pos2(210.0, 90.0),
            &tree,
            &windows,
            Some(ViewportId::ROOT),
            &[],
        );
        assert_eq!(hit.map(|a| a.folder), Some(PathBuf::from("/b")));
    }

    #[test]
    fn source_window_is_never_the_target() {
        let tree = [area("a", "/a/sub", rect(10.0, 10.0, 50.0, 30.0))];
        let windows = [area("a", "/a", rect(0.0, 0.0, 100.0, 100.0))];
        let hit = drop_target(
            egui::pos2(20.0, 20.0),
            &tree,
            &windows,
            Some(ViewportId::from_hash_of("a")),
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
            Some(ViewportId::ROOT),
            &[],
        );
        assert_eq!(hit, None);
    }

    #[test]
    fn folder_inside_a_dragged_path_is_no_target() {
        let tree = [area("b", "/a/x/inner", rect(250.0, 40.0, 290.0, 60.0))];
        let windows = [area("b", "/b", rect(200.0, 0.0, 300.0, 100.0))];
        let hit = drop_target(
            egui::pos2(260.0, 50.0),
            &tree,
            &windows,
            Some(ViewportId::ROOT),
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
            Some(ViewportId::ROOT),
            &[],
        );
        assert_eq!(hit.map(|a| a.folder), Some(PathBuf::from("/b")));
    }

    #[test]
    fn tree_folder_area_follows_its_row_on_screen() {
        use egui_kittest::Harness;
        let mut harness = Harness::new_ui(|ui| {
            track_always(ui.ctx(), true);
            let resp =
                ui.allocate_rect(rect(10.0, 10.0, 100.0, 30.0), egui::Sense::click_and_drag());
            target(&resp, Path::new("/x"));
        });
        harness
            .input_mut()
            .viewports
            .entry(ViewportId::ROOT)
            .or_default()
            .inner_rect = Some(rect(100.0, 100.0, 800.0, 600.0));
        harness.run();
        let areas = tree_folders(&harness.ctx);
        assert_eq!(areas.len(), 1);
        assert_eq!(areas[0].rect, rect(110.0, 110.0, 200.0, 130.0));
    }

    #[test]
    fn external_drop_targets_tree_folder_without_source_window() {
        let tree = [area("b", "/b/sub", rect(250.0, 40.0, 290.0, 60.0))];
        let hit = drop_target(egui::pos2(260.0, 50.0), &tree, &[], None, &[]);
        assert_eq!(hit.map(|a| a.folder), Some(PathBuf::from("/b/sub")));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn screen_pixels_become_points() {
        assert_eq!(to_points((300, 200), 1.5), egui::pos2(200.0, 400.0 / 3.0));
    }
}
