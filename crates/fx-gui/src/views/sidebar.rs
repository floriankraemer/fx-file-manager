use super::icons::{self, Glyph};
use super::{dnd, menu};
use crate::command::{Command, hint};
use crate::theme;
use eframe::egui::text::{LayoutJob, TextWrapping};
use eframe::egui::{
    Color32, CursorIcon, Label, Rect, Response, RichText, ScrollArea, Sense, Stroke, StrokeKind,
    Ui, pos2, vec2,
};
use fx_app::{Children, Explorer, Place, PlaceGroup, TreeNode, bookmark_places, t, tr};
use std::path::{Path, PathBuf};

const INDENT: f32 = 16.0;
const CHEVRON_W: f32 = 12.0;
/// Size of the arrow that ends each step of the path trail.
const STEP_SIZE: f32 = 14.0;

enum Action {
    Navigate(PathBuf),
    Toggle(PathBuf),
    OpenInNewTab(PathBuf),
    PasteInto(PathBuf),
    CopyPath(PathBuf),
    Delete(PathBuf),
    Drop(Vec<PathBuf>, PathBuf),
    NotFound(PathBuf),
    AddBookmark(PathBuf),
    RemoveBookmark(usize),
    /// Move the first group to the slot of the second.
    MoveGroup(PlaceGroup, PlaceGroup),
    /// Move the bookmark at the first index to the second.
    MoveBookmark(usize, usize),
}

/// Which bookmark a row shows and whether its folder can be used.
#[derive(Clone, Copy)]
struct BookmarkRow {
    index: usize,
    available: bool,
}

pub fn show(
    ui: &mut Ui,
    explorer: &mut Explorer,
    places: &[Place],
    actions: &mut Vec<crate::action::Action>,
) {
    let (path, scroll) = reveal_active(ui, explorer, places);
    let mut action = None;
    // A reserved (non-floating) bar keeps the row highlight from running under it.
    ui.spacing_mut().scroll.floating = false;
    ui.spacing_mut().scroll.foreground_color = false;
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut walk = TreeWalk {
                explorer,
                current: &path,
                scroll,
                last_bookmark_rows: Vec::new(),
                bookmark_rows: Vec::new(),
                action: &mut action,
                actions,
            };
            place_groups(ui, places, &mut walk);
        });
    if let Some(action) = action {
        apply(ui, explorer, action, actions);
    }
}

/// Reveals the active path whenever it changes and scrolls to it on the following frame.
/// Returns the active path and whether to scroll to it this frame.
fn reveal_active(ui: &Ui, explorer: &mut Explorer, places: &[Place]) -> (PathBuf, bool) {
    let key = ui.scope_id().with("revealed");
    let path = explorer.active().path().to_path_buf();
    let last: Option<PathBuf> = ui.data(|d| d.get_temp(key));
    let scroll =
        last.as_ref() == Some(&path) && ui.data(|d| d.get_temp(key.with("scroll")) == Some(true));
    if last.as_ref() != Some(&path) {
        let roots: Vec<PathBuf> = places.iter().map(|p| p.path.clone()).collect();
        explorer.tree_reveal(&path, &roots);
        ui.data_mut(|d| {
            d.insert_temp(key, path.clone());
            d.insert_temp(key.with("scroll"), true);
        });
    } else if scroll {
        ui.data_mut(|d| d.insert_temp(key.with("scroll"), false));
    }
    (path, scroll)
}

/// What every row of one tree pass reads, and where the pass collects its result.
struct TreeWalk<'a> {
    explorer: &'a Explorer,
    /// The active folder, highlighted and scrolled to.
    current: &'a Path,
    scroll: bool,
    /// Top and bottom edge of each bookmark row, last frame and this frame (later rows are not laid out yet while dragging).
    last_bookmark_rows: Vec<(f32, f32)>,
    bookmark_rows: Vec<(f32, f32)>,
    action: &'a mut Option<Action>,
    actions: &'a mut Vec<crate::action::Action>,
}

/// Every shown, non-empty group: a draggable heading followed by its top-level folders.
fn place_groups(ui: &mut Ui, places: &[Place], walk: &mut TreeWalk<'_>) {
    let explorer = walk.explorer;
    ui.spacing_mut().item_spacing.y = 0.0;
    let bookmarks = bookmark_places(explorer.bookmarks());
    let tops_key = ui.scope_id().with("group-tops");
    let last_tops: Vec<(PlaceGroup, f32)> = ui.data(|d| d.get_temp(tops_key)).unwrap_or_default();
    let mut tops = Vec::new();
    let rows_key = tops_key.with("bookmark-rows");
    walk.last_bookmark_rows = ui.data(|d| d.get_temp(rows_key)).unwrap_or_default();
    walk.bookmark_rows.clear();
    for group in explorer.settings().sidebar_groups() {
        let places = if group == PlaceGroup::Bookmarks {
            &bookmarks[..]
        } else {
            places
        };
        if places.iter().all(|p| p.group != group) {
            continue;
        }
        tops.push((group, ui.cursor().top()));
        group_heading(ui, group, &last_tops, walk.action);
        for (i, place) in places.iter().enumerate().filter(|(_, p)| p.group == group) {
            let title = place.title(explorer.settings().show_share_names);
            // Not yet checked counts as available.
            let bookmark = (group == PlaceGroup::Bookmarks).then(|| BookmarkRow {
                index: i,
                available: explorer.bookmark_available(i) != Some(false),
            });
            node(ui, walk, &title, &place.path, bookmark, 0);
        }
    }
    let rows = std::mem::take(&mut walk.bookmark_rows);
    ui.data_mut(|d| {
        d.insert_temp(tops_key, tops);
        d.insert_temp(rows_key, rows);
    });
}

/// The group heading. Dragging it onto another group's block moves the group there.
/// `tops` are the groups' top edges from the previous frame, as this frame's later groups are not laid out yet.
fn group_heading(
    ui: &mut Ui,
    group: PlaceGroup,
    tops: &[(PlaceGroup, f32)],
    action: &mut Option<Action>,
) {
    ui.add_space(8.0);
    let heading = RichText::new(group.title().to_uppercase())
        .small()
        .strong()
        .color(theme::dim());
    let resp = ui
        .add(Label::new(heading).sense(Sense::drag()))
        .on_hover_cursor(CursorIcon::Grab);
    ui.add_space(2.0);
    if !resp.dragged() && !resp.drag_stopped() {
        return;
    }
    let Some(target) = ui
        .input(|i| i.pointer.interact_pos())
        .and_then(|p| drop_target(tops, p.y))
    else {
        return;
    };
    if resp.drag_stopped() {
        *action = (target.0 != group).then_some(Action::MoveGroup(group, target.0));
        return;
    }
    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    let y = target.1;
    let line = Stroke::new(2.0, theme::accent());
    ui.painter().hline(ui.max_rect().x_range(), y, line);
}

/// The group whose block contains `y`, with the block's top edge.
fn drop_target(tops: &[(PlaceGroup, f32)], y: f32) -> Option<(PlaceGroup, f32)> {
    let hit = tops.iter().rev().find(|(_, top)| *top <= y);
    hit.or(tops.first()).copied()
}

fn apply(
    ui: &Ui,
    explorer: &mut Explorer,
    action: Action,
    actions: &mut Vec<crate::action::Action>,
) {
    match action {
        Action::Navigate(p) => explorer.navigate(p),
        Action::Toggle(p) => explorer.tree_toggle(p),
        Action::OpenInNewTab(p) => explorer.new_tab(p),
        Action::PasteInto(p) => explorer.paste(Some(p)),
        Action::CopyPath(p) => ui.ctx().copy_text(p.display().to_string()),
        Action::Delete(p) => actions.push(crate::action::Action::DeletePaths(vec![p])),
        Action::Drop(sources, dest) => {
            let copy = dnd::copying(ui.ctx(), explorer.settings().drag_copy_modifier);
            explorer.drop_into(sources, &dest, copy);
        }
        Action::NotFound(p) => {
            let text = tr(
                "notice.folder_not_found",
                &[("path", &p.display().to_string())],
            );
            explorer.notify_error(text);
        }
        Action::AddBookmark(p) => explorer.add_bookmark(p),
        Action::RemoveBookmark(i) => explorer.remove_bookmark(i),
        Action::MoveBookmark(from, to) => explorer.move_bookmark(from, to),
        Action::MoveGroup(group, target) => {
            let mut settings = explorer.settings().clone();
            settings.move_sidebar_group(group, target);
            explorer.set_settings(settings);
        }
    }
}

/// How a tree row looks; everything `paint_row` needs besides its rectangle.
struct RowLook<'a> {
    label: &'a str,
    path: &'a Path,
    depth: usize,
    selected: bool,
    /// A folder above the active one, on its path.
    ancestor: bool,
    hovered: bool,
    expanded: bool,
    has_chevron: bool,
    usable: bool,
}

fn node(
    ui: &mut Ui,
    walk: &mut TreeWalk<'_>,
    label: &str,
    path: &Path,
    bookmark: Option<BookmarkRow>,
    depth: usize,
) -> Rect {
    let explorer = walk.explorer;
    let tree = explorer.tree();
    let expanded = tree.is_expanded(path);
    let usable = bookmark.is_none_or(|b| b.available);
    // An unavailable bookmark is never listed: a dead mount would stall the tree.
    let children = if usable { tree.children(path) } else { None };
    let has_chevron = match children {
        None if !usable => false,
        None | Some(Children::Loading) => true,
        Some(Children::Loaded(c)) => !c.is_empty(),
        Some(Children::Failed(_)) => false,
    };

    let row_h = theme::row_height(22.0, explorer.settings().tree_font_size);
    // Only bookmark rows can be dragged (to reorder them).
    let sense = if bookmark.is_some() {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), sense);
    if let Some(b) = bookmark {
        bookmark_drag(ui, &resp, b.index, walk);
    }
    let selected = path == walk.current;
    if selected && walk.scroll {
        resp.scroll_to_me(None);
    }
    let look = RowLook {
        label,
        path,
        depth,
        selected,
        ancestor: !selected && walk.current.starts_with(path),
        hovered: resp.hovered(),
        expanded,
        has_chevron,
        usable,
    };
    let chevron = paint_row(ui, rect, row_h, &look);

    let on_chevron = has_chevron
        && resp
            .interact_pointer_pos()
            .is_some_and(|p| p.x < chevron.right() + 1.0);
    let single = explorer.settings().tree_single_click;
    if let Some(clicked) = click_action(&resp, path, usable, single, on_chevron) {
        *walk.action = Some(clicked);
    }

    if usable && let Some(sources) = dnd::target(&resp, path) {
        *walk.action = Some(Action::Drop(sources.paths.clone(), path.to_path_buf()));
    }

    resp.context_menu(|ui| {
        context_menu(
            ui,
            explorer,
            path,
            bookmark,
            usable,
            walk.action,
            walk.actions,
        );
    });

    if expanded && let Some(Children::Loaded(kids)) = children {
        let step = child_rows(ui, walk, kids, depth);
        if let Some(step) = step {
            paint_path_step(ui, rect, depth, &step);
        }
    }
    rect
}

/// The row below an ancestor that continues the path to the active folder.
struct PathStep {
    row: Rect,
    /// The row is the active folder itself.
    last: bool,
}

/// Lays out the child rows of an expanded folder; returns the one on the path to the active folder.
fn child_rows(
    ui: &mut Ui,
    walk: &mut TreeWalk<'_>,
    kids: &[TreeNode],
    depth: usize,
) -> Option<PathStep> {
    let mut step = None;
    for k in kids {
        let row = node(ui, walk, &k.name, &k.path, None, depth + 1);
        if walk.current.starts_with(&k.path) {
            step = Some(PathStep {
                row,
                last: walk.current == k.path,
            });
        }
    }
    step
}

/// Draws the trail from an ancestor row down to the row on the path: a vertical line at the
/// ancestor's chevron column that ends in an arrow pointing at the next folder.
fn paint_path_step(ui: &Ui, parent: Rect, depth: usize, step: &PathStep) {
    let x = parent.left() + 4.0 + depth as f32 * INDENT + CHEVRON_W / 2.0;
    let color = if step.last {
        theme::accent()
    } else {
        theme::accent().gamma_multiply(0.7)
    };
    // Where Lucide's corner-down-right has its vertical stroke and its horizontal bar.
    let (stem, bar) = (STEP_SIZE * 4.0 / 24.0, STEP_SIZE * 15.0 / 24.0);
    let corner = pos2(x - stem, step.row.center().y - bar);
    let stroke = Stroke::new(STEP_SIZE * 2.0 / 24.0, color);
    let from = pos2(x, parent.center().y + 6.0);
    ui.painter()
        .line_segment([from, pos2(x, corner.y + stem)], stroke);
    let glyph = Rect::from_min_size(corner, vec2(STEP_SIZE, STEP_SIZE));
    icons::glyph(ui, glyph, Glyph::PathStep, color);
}

/// Reorders bookmarks: dragging a row onto another row's slot moves it there.
fn bookmark_drag(ui: &Ui, resp: &Response, index: usize, walk: &mut TreeWalk<'_>) {
    walk.bookmark_rows
        .push((resp.rect.top(), resp.rect.bottom()));
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    if !resp.dragged() && !resp.drag_stopped() {
        return;
    }
    let Some(to) = ui
        .input(|i| i.pointer.interact_pos())
        .and_then(|p| nearest_row(&walk.last_bookmark_rows, p.y))
    else {
        return;
    };
    if resp.drag_stopped() {
        *walk.action = (to != index).then_some(Action::MoveBookmark(index, to));
        return;
    }
    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    let (top, bottom) = walk.last_bookmark_rows[to];
    let y = if to > index { bottom } else { top };
    ui.painter()
        .hline(resp.rect.x_range(), y, Stroke::new(2.0, theme::accent()));
}

/// Index of the row whose vertical span contains `y`, or the closest one.
fn nearest_row(rows: &[(f32, f32)], y: f32) -> Option<usize> {
    let gap = |(top, bottom): (f32, f32)| (top - y).max(y - bottom).max(0.0);
    (0..rows.len()).min_by(|a, b| gap(rows[*a]).total_cmp(&gap(rows[*b])))
}

/// Paints highlight, chevron, folder icon and label; returns the chevron's hit area.
fn paint_row(ui: &Ui, rect: Rect, row_h: f32, look: &RowLook) -> Rect {
    let fill = if look.selected {
        theme::selected()
    } else if look.hovered {
        theme::panel_hi()
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 6.0, fill);
    if look.selected {
        let ring = Stroke::new(1.0, theme::accent().gamma_multiply(0.45));
        painter.rect_stroke(rect.shrink(0.5), 6.0, ring, StrokeKind::Inside);
    }

    let x0 = rect.left() + 4.0 + look.depth as f32 * INDENT;
    let chevron = Rect::from_min_size(pos2(x0, rect.top()), vec2(CHEVRON_W, row_h));
    if look.has_chevron {
        let g = if look.expanded {
            Glyph::Chevron
        } else {
            Glyph::ChevronRight
        };
        icons::glyph(
            ui,
            Rect::from_center_size(chevron.center(), vec2(16.0, 16.0)),
            g,
            theme::dim(),
        );
    }
    let icon_x = x0 + CHEVRON_W + 2.0;
    let icon = Rect::from_center_size(pos2(icon_x + 8.0, rect.center().y), vec2(16.0, 16.0));
    let name = look
        .path
        .file_name()
        .map_or_else(|| "root".to_owned(), |n| n.to_string_lossy().into_owned());
    icons::paint_folder(ui, icon, &name, look.expanded);
    let text_x = icon_x + 16.0 + 6.0;
    let color = match (look.usable, look.ancestor) {
        (false, _) => theme::dim(),
        (true, true) => theme::header_text(),
        (true, false) => theme::text(),
    };
    let mut job = LayoutJob::simple_singleline(
        look.label.to_owned(),
        theme::tree_style().resolve(ui.style()),
        color,
    );
    job.wrap = TextWrapping::truncate_at_width((rect.right() - 4.0 - text_x).max(0.0));
    let galley = ui.painter().layout_job(job);
    painter.galley(
        pos2(text_x, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    chevron
}

/// What a click on the row does, if anything.
fn click_action(
    resp: &Response,
    path: &Path,
    usable: bool,
    single: bool,
    on_chevron: bool,
) -> Option<Action> {
    if !usable {
        return resp.clicked().then(|| Action::NotFound(path.to_path_buf()));
    }
    if resp.double_clicked() {
        // Single-click mode already navigated on the first click, so the second one expands.
        return Some(if single {
            Action::Toggle(path.to_path_buf())
        } else {
            Action::Navigate(path.to_path_buf())
        });
    }
    if !resp.clicked() {
        return None;
    }
    if on_chevron {
        Some(Action::Toggle(path.to_path_buf()))
    } else if single {
        Some(Action::Navigate(path.to_path_buf()))
    } else {
        None
    }
}

fn context_menu(
    ui: &mut Ui,
    explorer: &Explorer,
    path: &Path,
    bookmark: Option<BookmarkRow>,
    usable: bool,
    action: &mut Option<Action>,
    actions: &mut Vec<crate::action::Action>,
) {
    if menu::item(ui, t("cmd.open"), "", usable) {
        *action = Some(Action::Navigate(path.to_path_buf()));
    }
    if menu::item(ui, t("cmd.open_new_tab"), "", usable) {
        *action = Some(Action::OpenInNewTab(path.to_path_buf()));
    }
    ui.separator();
    let marked = explorer.bookmarks().iter().position(|b| b.path == path);
    if let Some(b) = bookmark
        && menu::item(ui, t("cmd.rename_bookmark"), "", true)
    {
        actions.push(crate::action::Action::RenameBookmark(b.index));
    }
    match marked {
        Some(i) if menu::item(ui, t("cmd.remove_bookmark"), "", true) => {
            *action = Some(Action::RemoveBookmark(i));
        }
        None if menu::item(ui, t("cmd.add_bookmark"), "", true) => {
            *action = Some(Action::AddBookmark(path.to_path_buf()));
        }
        _ => {}
    }
    ui.separator();
    if menu::item(
        ui,
        t("cmd.paste"),
        &hint(Command::Paste),
        usable && explorer.can_paste(),
    ) {
        *action = Some(Action::PasteInto(path.to_path_buf()));
    }
    if menu::item(ui, t("cmd.copy_path"), "", true) {
        *action = Some(Action::CopyPath(path.to_path_buf()));
    }
    ui.separator();
    if menu::item(ui, t("cmd.delete"), &hint(Command::Delete), true) {
        *action = Some(Action::Delete(path.to_path_buf()));
    }
}
