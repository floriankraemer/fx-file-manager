use super::icons::{self, Glyph};
use super::{dnd, menu};
use crate::command::{Command, hint};
use crate::theme;
use eframe::egui::text::{LayoutJob, TextWrapping};
use eframe::egui::{Color32, Rect, Response, RichText, ScrollArea, Sense, Ui, pos2, vec2};
use fx_app::{Children, Explorer, Place, PlaceGroup, bookmark_places, t, tr};
use std::path::{Path, PathBuf};

const GROUPS: [PlaceGroup; 3] = [
    PlaceGroup::Places,
    PlaceGroup::Bookmarks,
    PlaceGroup::Drives,
];
const INDENT: f32 = 16.0;
const CHEVRON_W: f32 = 12.0;

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
    action: &'a mut Option<Action>,
    actions: &'a mut Vec<crate::action::Action>,
}

/// Every non-empty group: a heading followed by its top-level folders.
fn place_groups(ui: &mut Ui, places: &[Place], walk: &mut TreeWalk<'_>) {
    let explorer = walk.explorer;
    ui.spacing_mut().item_spacing.y = 0.0;
    let bookmarks = bookmark_places(explorer.bookmarks());
    for group in GROUPS {
        let places = if group == PlaceGroup::Bookmarks {
            &bookmarks[..]
        } else {
            places
        };
        if places.iter().all(|p| p.group != group) {
            continue;
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new(group.title().to_uppercase())
                .small()
                .strong()
                .color(theme::dim()),
        );
        ui.add_space(2.0);
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
    }
}

/// How a tree row looks; everything `paint_row` needs besides its rectangle.
struct RowLook<'a> {
    label: &'a str,
    path: &'a Path,
    depth: usize,
    selected: bool,
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
) {
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
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
    let selected = path == walk.current;
    if selected && walk.scroll {
        resp.scroll_to_me(None);
    }
    let look = RowLook {
        label,
        path,
        depth,
        selected,
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
        *walk.action = Some(Action::Drop(sources.0.clone(), path.to_path_buf()));
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
        for k in kids {
            node(ui, walk, &k.name, &k.path, None, depth + 1);
        }
    }
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
    let color = if look.usable {
        theme::text()
    } else {
        theme::dim()
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
