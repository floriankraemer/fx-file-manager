//! The Icons view: entries as tiles with a preview or their icon, only the visible rows laid out.

use super::entries::{self, EntryContext, EntryInput};
use super::thumbs::{self, Textures};
use super::{icons, table};
use crate::{action::Action, theme};
use eframe::egui::text::{LayoutJob, TextWrapping};
use eframe::egui::{self, Align, Rect, ScrollArea, Sense, Ui, Vec2, pos2, vec2};
use fx_app::{Explorer, MAX_ICON_SIZE, MIN_ICON_SIZE, ThumbKey};
use std::ops::Range;

const PAD: f32 = 4.0;
const LABEL_ROWS: usize = 2;
/// Seconds the shown tile size takes to catch up with the wheel.
const ZOOM_EASE: f32 = 0.12;
const SCROLL_ID: &str = "icon-grid";

/// Tile geometry of one frame.
struct Layout {
    columns: usize,
    rows: usize,
    tile: Vec2,
    /// Edge of the preview square in points.
    icon: f32,
}

impl Layout {
    fn new(ui: &Ui, icon: f32, entries: usize) -> Self {
        let label = ui.text_style_height(&theme::list_style()) * LABEL_ROWS as f32;
        Self::sized(ui.available_width(), label, icon, entries)
    }

    /// Tiles have a fixed size; a wider window only fits more of them. Stretching them to the row
    /// width would move every tile and rewrap every name on each pixel of a resize.
    fn sized(width: f32, label: f32, icon: f32, entries: usize) -> Self {
        let tile = vec2(icon + 4.0 * PAD, icon + label + 3.0 * PAD);
        let columns = ((width.max(1.0) / tile.x).floor() as usize).max(1);
        Self {
            columns,
            rows: entries.div_ceil(columns),
            tile,
            icon,
        }
    }

    /// Indices of the entries in grid rows `rows`.
    fn entries(&self, rows: &Range<usize>, total: usize) -> Range<usize> {
        (rows.start * self.columns).min(total)..(rows.end * self.columns).min(total)
    }
}

/// What every tile reads this frame.
struct Tiles<'a> {
    entries: EntryContext<'a>,
    layout: &'a Layout,
    textures: Textures,
    pixels_per_point: f32,
}

/// What tiles report besides entry input: thumbnails still to produce, visible ones first.
#[derive(Default)]
struct Output {
    input: EntryInput,
    wanted: Vec<ThumbKey>,
}

/// Shows the grid and returns its column count, which keyboard navigation moves by.
pub fn show(
    ui: &mut Ui,
    explorer: &mut Explorer,
    scroll_to_selection: bool,
    actions: &mut Vec<Action>,
) -> usize {
    ui.style_mut().interaction.selectable_labels = false;
    thumbs::begin_frame(ui.ctx(), explorer);
    let Some(view) = table::loaded_view(ui, explorer.active()) else {
        return 1;
    };
    let settings = explorer.settings();
    let (icon_size, show_hidden) = (settings.icon_size, settings.show_hidden);
    let total = explorer.active().order().len();
    let layout = Layout::new(ui, shown_size(ui, icon_size), total);
    let modifiers = ui.input(|i| i.modifiers);
    let can_paste = explorer.can_paste();
    let mut out = Output::default();
    let scroll = {
        let tiles = Tiles {
            entries: EntryContext {
                explorer,
                view: &view,
                tab: explorer.active(),
                folder: explorer.active().path(),
                single_click: settings.list_single_click,
                can_paste,
            },
            layout: &layout,
            textures: thumbs::textures(ui.ctx()),
            pixels_per_point: ui.pixels_per_point(),
        };
        let offset = scroll_offset(ui, &tiles, scroll_to_selection);
        body(ui, &tiles, offset, &mut out, actions)
    };
    entries::background(
        ui,
        entries::below_content(&scroll),
        show_hidden,
        can_paste,
        actions,
    );
    zoom(ui, scroll.inner_rect, icon_size, layout.icon, actions);
    explorer.want_thumbnails(out.wanted);
    entries::apply(ui, explorer, out.input, modifiers);
    layout.columns
}

fn body(
    ui: &mut Ui,
    tiles: &Tiles,
    offset: Option<f32>,
    out: &mut Output,
    actions: &mut Vec<Action>,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    let mut area = ScrollArea::vertical().id_salt(SCROLL_ID).auto_shrink(false);
    if let Some(offset) = offset {
        area = area.vertical_scroll_offset(offset);
    }
    let layout = tiles.layout;
    area.show_rows(ui, layout.tile.y, layout.rows, |ui, rows| {
        for row in rows.clone() {
            grid_row(ui, row, tiles, out, actions);
        }
        prefetch(tiles, &rows, out);
    })
}

/// The offset that brings the focused tile into view, when the selection moved and it is outside.
fn scroll_offset(ui: &Ui, tiles: &Tiles, scroll_to_selection: bool) -> Option<f32> {
    let row = tiles
        .entries
        .tab
        .selected()
        .filter(|_| scroll_to_selection)?
        / tiles.layout.columns;
    let id = ui.make_persistent_id(egui::IdSalt::new(SCROLL_ID));
    let current =
        egui::scroll_area::State::load(ui.ctx(), id).map_or(0.0, |s| s.clamped_offset().y);
    let (top, height) = (row as f32 * tiles.layout.tile.y, tiles.layout.tile.y);
    let viewport = ui.available_height();
    if top < current {
        return Some(top);
    }
    (top + height > current + viewport).then_some(top + height - viewport)
}

fn grid_row(ui: &mut Ui, row: usize, tiles: &Tiles, out: &mut Output, actions: &mut Vec<Action>) {
    let layout = tiles.layout;
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), layout.tile.y), Sense::hover());
    let total = tiles.entries.tab.order().len();
    for i in layout.entries(&(row..row + 1), total) {
        let column = (i % layout.columns) as f32;
        let min = pos2(rect.left() + column * layout.tile.x, rect.top());
        tile(
            ui,
            i,
            Rect::from_min_size(min, layout.tile),
            tiles,
            out,
            actions,
        );
    }
}

fn tile(
    ui: &mut Ui,
    i: usize,
    rect: Rect,
    tiles: &Tiles,
    out: &mut Output,
    actions: &mut Vec<Action>,
) {
    let e = &tiles.entries;
    let entry = &e.view.listing.entries[e.tab.order()[i] as usize];
    let name = e.view.listing.name(entry).to_string_lossy();
    let resp = ui.interact(
        rect,
        ui.scope_id().with(("tile", i)),
        Sense::click_and_drag(),
    );
    let selected = e.tab.is_selected(i);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::ListBoxOption, true, selected, name.as_ref())
    });
    paint_highlight(ui, &resp, rect, selected);
    let icon = tiles.layout.icon;
    let preview = Rect::from_min_size(
        pos2(rect.center().x - icon / 2.0, rect.top() + PAD),
        vec2(icon, icon),
    );
    if !paint_thumbnail(ui, i, preview, tiles, &mut out.wanted) {
        icons::paint_entry_sized(ui, preview, entry.kind, &name, icon * 0.8);
    }
    paint_label(ui, rect, preview.bottom() + PAD, &name, selected);
    entries::respond(&resp, i, &name, entry.kind, e, &mut out.input, actions);
}

fn paint_highlight(ui: &Ui, resp: &egui::Response, rect: Rect, selected: bool) {
    let rect = rect.shrink(2.0);
    if selected {
        ui.painter().rect_filled(rect, 6.0, theme::selection_bg());
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, theme::panel_hi());
    }
}

fn thumb_px(tiles: &Tiles) -> u32 {
    (tiles.layout.icon * tiles.pixels_per_point).round() as u32
}

/// Paints the preview of entry `i` if it is ready; otherwise asks for it. Returns true if painted.
fn paint_thumbnail(
    ui: &Ui,
    i: usize,
    rect: Rect,
    tiles: &Tiles,
    wanted: &mut Vec<ThumbKey>,
) -> bool {
    let Some(key) = tiles.entries.explorer.thumb_key(i, thumb_px(tiles)) else {
        return false;
    };
    let Some((texture, size)) = tiles.textures.get(&key) else {
        if !tiles.textures.known(&key) {
            wanted.push(key);
        }
        return false;
    };
    // Physical pixels to points; never enlarged beyond one texel per pixel, so small images stay crisp.
    let points = size / tiles.pixels_per_point;
    let scale = (rect.width() / points.x)
        .min(rect.height() / points.y)
        .min(1.0);
    let target = Rect::from_center_size(rect.center(), points * scale);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter()
        .image(texture, target, uv, egui::Color32::WHITE);
    true
}

/// The name centered below the preview, wrapped onto at most two lines and elided after that.
fn paint_label(ui: &Ui, rect: Rect, top: f32, name: &str, selected: bool) {
    let color = if selected {
        theme::selection_text()
    } else {
        theme::text()
    };
    let font = theme::list_style().resolve(ui.style());
    let width = rect.width() - 2.0 * PAD;
    let mut job = LayoutJob::simple(name.to_owned(), font, color, width);
    job.wrap = TextWrapping {
        max_width: width,
        max_rows: LABEL_ROWS,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    job.halign = Align::Center;
    let galley = ui.painter().layout_job(job);
    ui.painter()
        .galley(pos2(rect.center().x, top), galley, color);
}

/// Queues the thumbnails of the screenful below the visible rows, after the visible ones.
fn prefetch(tiles: &Tiles, rows: &Range<usize>, out: &mut Output) {
    let ahead = rows.end..(rows.end + rows.len()).min(tiles.layout.rows);
    let total = tiles.entries.tab.order().len();
    let px = thumb_px(tiles);
    for i in tiles.layout.entries(&ahead, total) {
        let key = tiles.entries.explorer.thumb_key(i, px);
        if let Some(key) = key.filter(|k| !tiles.textures.known(k)) {
            out.wanted.push(key);
        }
    }
}

/// Ctrl+wheel zoom in progress. The target follows the wheel continuously; the setting is saved
/// once the shown size has caught up, so a gesture writes `settings.toml` once instead of every frame.
#[derive(Clone, Copy)]
struct Zoom {
    target: f32,
    /// The saved size `target` started from; a different one means the settings changed elsewhere.
    saved: u32,
}

fn zoom_id() -> egui::Id {
    egui::Id::unique("icon-grid-zoom")
}

/// The tile size to lay out this frame, easing toward the zoom target.
fn shown_size(ui: &Ui, saved: u32) -> f32 {
    let target = current_zoom(ui, saved).target;
    ui.ctx()
        .animate_value_with_time(zoom_id().with("shown"), target, ZOOM_EASE)
}

fn current_zoom(ui: &Ui, saved: u32) -> Zoom {
    let fresh = Zoom {
        target: saved as f32,
        saved,
    };
    let zoom: Option<Zoom> = ui.data(|d| d.get_temp(zoom_id()));
    zoom.filter(|z| z.saved == saved).unwrap_or(fresh)
}

/// Ctrl+wheel over the grid scales the target size by the wheel's zoom factor; the size is saved
/// once the animation toward it has finished.
fn zoom(ui: &Ui, area: Rect, saved: u32, shown: f32, actions: &mut Vec<Action>) {
    let mut zoom = current_zoom(ui, saved);
    let delta = ui.input(|i| i.zoom_delta());
    let zooming = ui.rect_contains_pointer(area) && (delta - 1.0).abs() > f32::EPSILON;
    if zooming {
        let (min, max) = (MIN_ICON_SIZE as f32, MAX_ICON_SIZE as f32);
        zoom.target = (zoom.target * delta).clamp(min, max);
    }
    let rounded = zoom.target.round() as u32;
    if !zooming && (shown - zoom.target).abs() < 0.5 && rounded != saved {
        zoom = Zoom {
            target: rounded as f32,
            saved: rounded,
        };
        actions.push(Action::SetIconSize(rounded));
    }
    ui.data_mut(|d| d.insert_temp(zoom_id(), zoom));
}

#[cfg(test)]
mod tests {
    use super::{Layout, show};
    use crate::action::Action;
    use egui::{PointerButton, Pos2};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use fx_app::{Explorer, LoadState, Settings, ViewMode};
    use std::time::{Duration, Instant};

    type World = (Explorer, Vec<Action>, usize);

    fn explorer(dir: &std::path::Path) -> Explorer {
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let settings = Settings {
            default_view: ViewMode::Icons,
            ..Settings::default()
        };
        let mut explorer = Explorer::with_settings(dir.to_path_buf(), settings, || {});
        let deadline = Instant::now() + Duration::from_secs(10);
        while matches!(explorer.active().state(), LoadState::Loading) {
            assert!(Instant::now() < deadline, "listing did not finish");
            explorer.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        explorer
    }

    fn harness(explorer: Explorer) -> Harness<'static, World> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            // A real frame interval, so two clicks fall within the double-click time.
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(
                |ui, (explorer, actions, columns): &mut World| {
                    if !ui
                        .style()
                        .text_styles
                        .contains_key(&crate::theme::list_style())
                    {
                        crate::theme::apply(ui.ctx(), &Settings::default());
                        let style = ui.ctx().global_style();
                        ui.set_style(style);
                    }
                    *columns = show(ui, explorer, false, actions);
                },
                (explorer, Vec::new(), 0),
            );
        harness.run();
        harness
    }

    fn click(harness: &mut Harness<'_, World>, pos: Pos2, times: usize) {
        harness.hover_at(pos);
        harness.run();
        for _ in 0..times {
            for pressed in [true, false] {
                harness.event(egui::Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                });
                harness.step();
            }
        }
        // A folder that opens shows a spinner, so the UI never settles; one more frame applies the input.
        harness.step();
    }

    #[test]
    fn resizing_the_window_does_not_resize_tiles() {
        let narrow = Layout::sized(500.0, 30.0, 96.0, 40);
        let wide = Layout::sized(537.0, 30.0, 96.0, 40);
        assert_eq!(narrow.tile, wide.tile);
        assert_eq!(narrow.columns, 4);
        let wider = Layout::sized(700.0, 30.0, 96.0, 40);
        assert_eq!((wider.tile, wider.columns), (narrow.tile, 6));
    }

    #[test]
    fn a_zoom_gesture_eases_and_saves_the_size_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut harness = harness(explorer(dir.path()));
        harness.hover_at(egui::pos2(400.0, 300.0));
        harness.step();
        harness.event(egui::Event::Zoom(1.5));
        harness.step();
        assert!(
            harness.state().1.is_empty(),
            "nothing is saved mid-animation"
        );
        for _ in 0..30 {
            harness.step();
        }
        let saved: Vec<u32> = harness
            .state()
            .1
            .iter()
            .filter_map(|a| match a {
                Action::SetIconSize(size) => Some(*size),
                _ => None,
            })
            .collect();
        assert_eq!(saved, vec![144]);
    }

    #[test]
    fn tiles_fill_rows_and_a_click_selects() {
        let dir = tempfile::tempdir().unwrap();
        let mut harness = harness(explorer(dir.path()));
        assert!(
            harness.state().2 > 1,
            "a wide window holds several tiles per row"
        );
        let pos = harness.get_by_label("b.txt").rect().center();
        click(&mut harness, pos, 1);
        let (explorer, _, _) = harness.state();
        let selected = explorer.active().selected().unwrap();
        assert_eq!(
            explorer.entry_path(selected).unwrap(),
            dir.path().join("b.txt")
        );
    }

    #[test]
    fn double_clicking_a_folder_tile_enters_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut harness = harness(explorer(dir.path()));
        let pos = harness.get_by_label("folder").rect().center();
        click(&mut harness, pos, 2);
        let target = dir.path().join("folder");
        let deadline = Instant::now() + Duration::from_secs(10);
        while harness.state().0.active().path() != target {
            assert!(Instant::now() < deadline, "the folder did not open");
            harness.state_mut().0.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
