use super::icons::{self, Glyph};
use super::{dnd, menu};
use crate::command::{Command, hint};
use crate::theme;
use eframe::egui::text::{LayoutJob, TextWrapping};
use eframe::egui::{Color32, Rect, RichText, ScrollArea, Sense, Ui, pos2, vec2};
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
    // Reveal the active path whenever it changes; scroll to it on the following frame.
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

    let mut action = None;
    // A reserved (non-floating) bar keeps the row highlight from running under it.
    ui.spacing_mut().scroll.floating = false;
    ui.spacing_mut().scroll.foreground_color = false;
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
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
                for (i, place) in places.iter().enumerate() {
                    if place.group != group {
                        continue;
                    }
                    let title = place.title(explorer.settings().show_share_names);
                    // Not yet checked counts as available.
                    let bookmark = (group == PlaceGroup::Bookmarks).then(|| BookmarkRow {
                        index: i,
                        available: explorer.bookmark_available(i) != Some(false),
                    });
                    node(
                        ui,
                        explorer,
                        &title,
                        &place.path,
                        bookmark,
                        0,
                        &path,
                        scroll,
                        &mut action,
                        actions,
                    );
                }
            }
        });
    match action {
        Some(Action::Navigate(p)) => explorer.navigate(p),
        Some(Action::Toggle(p)) => explorer.tree_toggle(p),
        Some(Action::OpenInNewTab(p)) => explorer.new_tab(p),
        Some(Action::PasteInto(p)) => explorer.paste(Some(p)),
        Some(Action::CopyPath(p)) => ui.ctx().copy_text(p.display().to_string()),
        Some(Action::Delete(p)) => actions.push(crate::action::Action::DeletePaths(vec![p])),
        Some(Action::Drop(sources, dest)) => {
            let copy = dnd::copying(ui.ctx(), explorer.settings().drag_copy_modifier);
            explorer.drop_into(sources, &dest, copy);
        }
        Some(Action::NotFound(p)) => {
            let text = tr(
                "notice.folder_not_found",
                &[("path", &p.display().to_string())],
            );
            explorer.notify_error(text);
        }
        Some(Action::AddBookmark(p)) => explorer.add_bookmark(p),
        Some(Action::RemoveBookmark(i)) => explorer.remove_bookmark(i),
        None => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn node(
    ui: &mut Ui,
    explorer: &Explorer,
    label: &str,
    path: &Path,
    bookmark: Option<BookmarkRow>,
    depth: usize,
    current: &Path,
    scroll: bool,
    action: &mut Option<Action>,
    actions: &mut Vec<crate::action::Action>,
) {
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
    let selected = path == current;
    if selected && scroll {
        resp.scroll_to_me(None);
    }
    let fill = if selected {
        theme::selected()
    } else if resp.hovered() {
        theme::panel_hi()
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 6.0, fill);

    let x0 = rect.left() + 4.0 + depth as f32 * INDENT;
    let chevron = Rect::from_min_size(pos2(x0, rect.top()), vec2(CHEVRON_W, row_h));
    if has_chevron {
        let g = if expanded {
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
    let name = path
        .file_name()
        .map_or_else(|| "root".to_owned(), |n| n.to_string_lossy().into_owned());
    icons::paint_folder(ui, icon, &name, expanded);
    let text_x = icon_x + 16.0 + 6.0;
    let color = if usable { theme::text() } else { theme::dim() };
    let mut job = LayoutJob::simple_singleline(
        label.to_owned(),
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

    let single = explorer.settings().tree_single_click;
    if !usable {
        if resp.clicked() {
            *action = Some(Action::NotFound(path.to_path_buf()));
        }
    } else if resp.double_clicked() {
        // Single-click mode already navigated on the first click, so the second one expands.
        *action = Some(if single {
            Action::Toggle(path.to_path_buf())
        } else {
            Action::Navigate(path.to_path_buf())
        });
    } else if resp.clicked() {
        let on_chevron = has_chevron
            && resp
                .interact_pointer_pos()
                .is_some_and(|p| p.x < chevron.right() + 1.0);
        if on_chevron {
            *action = Some(Action::Toggle(path.to_path_buf()));
        } else if single {
            *action = Some(Action::Navigate(path.to_path_buf()));
        }
    }

    if usable && let Some(sources) = dnd::target(&resp, path) {
        *action = Some(Action::Drop(sources.0.clone(), path.to_path_buf()));
    }

    resp.context_menu(|ui| {
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
            Some(i) => {
                if menu::item(ui, t("cmd.remove_bookmark"), "", true) {
                    *action = Some(Action::RemoveBookmark(i));
                }
            }
            None => {
                if menu::item(ui, t("cmd.add_bookmark"), "", true) {
                    *action = Some(Action::AddBookmark(path.to_path_buf()));
                }
            }
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
    });

    if expanded && let Some(Children::Loaded(kids)) = children {
        for k in kids {
            node(
                ui,
                explorer,
                &k.name,
                &k.path,
                None,
                depth + 1,
                current,
                scroll,
                action,
                actions,
            );
        }
    }
}
