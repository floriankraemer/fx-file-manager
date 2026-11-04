use super::commandbar::menu_item;
use super::icons::{self, Glyph};
use crate::theme;
use eframe::egui::text::{LayoutJob, TextWrapping};
use eframe::egui::{Color32, Rect, RichText, ScrollArea, Sense, Ui, pos2, vec2};
use fx_app::{Children, Explorer, Place, PlaceGroup, t};
use std::path::{Path, PathBuf};

const GROUPS: [PlaceGroup; 2] = [PlaceGroup::Places, PlaceGroup::Drives];
const INDENT: f32 = 16.0;
const CHEVRON_W: f32 = 12.0;

enum Action {
    Navigate(PathBuf),
    Toggle(PathBuf),
    OpenInNewTab(PathBuf),
    PasteInto(PathBuf),
    CopyPath(PathBuf),
}

pub fn show(ui: &mut Ui, explorer: &mut Explorer, places: &[Place]) {
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
            for group in GROUPS {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(group.title().to_uppercase())
                        .small()
                        .strong()
                        .color(theme::dim()),
                );
                ui.add_space(2.0);
                for place in places.iter().filter(|p| p.group == group) {
                    node(
                        ui,
                        explorer,
                        &place.label,
                        &place.path,
                        0,
                        &path,
                        scroll,
                        &mut action,
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
        None => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn node(
    ui: &mut Ui,
    explorer: &Explorer,
    label: &str,
    path: &Path,
    depth: usize,
    current: &Path,
    scroll: bool,
    action: &mut Option<Action>,
) {
    let tree = explorer.tree();
    let expanded = tree.is_expanded(path);
    let children = tree.children(path);
    let has_chevron = match children {
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
    let mut job = LayoutJob::simple_singleline(
        label.to_owned(),
        theme::tree_style().resolve(ui.style()),
        theme::text(),
    );
    job.wrap = TextWrapping::truncate_at_width((rect.right() - 4.0 - text_x).max(0.0));
    let galley = ui.painter().layout_job(job);
    painter.galley(
        pos2(text_x, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::text(),
    );

    let single = explorer.settings().tree_single_click;
    if resp.double_clicked() {
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

    resp.context_menu(|ui| {
        if menu_item(ui, t("cmd.open"), "", true) {
            *action = Some(Action::Navigate(path.to_path_buf()));
        }
        if menu_item(ui, t("cmd.open_new_tab"), "", true) {
            *action = Some(Action::OpenInNewTab(path.to_path_buf()));
        }
        ui.separator();
        if menu_item(ui, t("cmd.paste"), "Ctrl+V", explorer.can_paste()) {
            *action = Some(Action::PasteInto(path.to_path_buf()));
        }
        if menu_item(ui, t("cmd.copy_path"), "", true) {
            *action = Some(Action::CopyPath(path.to_path_buf()));
        }
    });

    if expanded && let Some(Children::Loaded(kids)) = children {
        for k in kids {
            node(
                ui,
                explorer,
                &k.name,
                &k.path,
                depth + 1,
                current,
                scroll,
                action,
            );
        }
    }
}
