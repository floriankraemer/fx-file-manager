use super::icons::{self, Glyph};
use crate::command::{Command, tip};
use crate::theme;
use eframe::egui::text_selection::CCursorRange;
use eframe::egui::{
    CursorIcon, Key, Layout, Response, RichText, Role, Sense, Stroke, TextEdit, TextStyle, Ui,
    WidgetInfo, vec2,
};
use fx_app::{Explorer, Tab, breadcrumbs, t};
use std::path::{Path, PathBuf};

/// The path bar while the user types a path instead of clicking crumbs.
pub struct PathEdit {
    text: String,
    /// Tab and folder it was opened for; showing anything else cancels the edit.
    tab: usize,
    origin: PathBuf,
    fresh: bool,
}

/// The path a user typed or pasted: trimmed, surrounding quotes removed; `None` when empty.
pub fn parse_typed_path(text: &str) -> Option<PathBuf> {
    let text = text.trim();
    let unquoted = ["\"", "'"]
        .iter()
        .find_map(|q| text.strip_prefix(q)?.strip_suffix(q))
        .unwrap_or(text)
        .trim();
    (!unquoted.is_empty()).then(|| PathBuf::from(unquoted))
}

/// Height of every control in the toolbar row.
const CRUMB_HEIGHT: f32 = 28.0;

pub fn show(ui: &mut Ui, explorer: &mut Explorer, path_edit: &mut Option<PathEdit>) {
    if path_edit
        .as_ref()
        .is_some_and(|e| e.tab != explorer.active_index() || e.origin != explorer.active().path())
    {
        *path_edit = None;
    }
    let mut goto = None;
    ui.horizontal(|ui| {
        // Buttons, filter field and breadcrumbs share one height.
        ui.spacing_mut().interact_size.y = CRUMB_HEIGHT;
        nav_buttons(ui, explorer);
        ui.separator();

        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            filter_field(ui, explorer.active_mut());
            goto = ui
                .with_layout(Layout::left_to_right(egui::Align::Center), |ui| {
                    path_area(ui, explorer, path_edit)
                })
                .inner;
        });
    });
    if let Some(path) = goto {
        explorer.navigate(path);
    }
}

/// Back, forward, up and refresh.
fn nav_buttons(ui: &mut Ui, explorer: &mut Explorer) {
    let (can_back, can_forward, can_up) = {
        let tab = explorer.active();
        (tab.can_back(), tab.can_forward(), tab.can_up())
    };
    if nav_button(ui, Glyph::Back, can_back, t("tip.back"))
        .on_hover_text(tip("tip.back", Command::Back))
        .clicked()
    {
        explorer.back();
    }
    if nav_button(ui, Glyph::Forward, can_forward, t("tip.forward"))
        .on_hover_text(tip("tip.forward", Command::Forward))
        .clicked()
    {
        explorer.forward();
    }
    if nav_button(ui, Glyph::Up, can_up, t("tip.up"))
        .on_hover_text(tip("tip.up", Command::Up))
        .clicked()
    {
        explorer.up();
    }
    if nav_button(ui, Glyph::Refresh, true, t("tip.refresh"))
        .on_hover_text(tip("tip.refresh", Command::Refresh))
        .clicked()
    {
        explorer.refresh();
    }
}

/// A flat icon button; `label` names it for assistive tech.
fn nav_button(ui: &mut Ui, glyph: Glyph, enabled: bool, label: &str) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, CRUMB_HEIGHT), sense);
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, theme::panel_hi());
    }
    let color = if enabled {
        theme::text()
    } else {
        theme::dim().gamma_multiply(0.6)
    };
    icons::glyph(ui, rect, glyph, color);
    resp
}

fn filter_field(ui: &mut Ui, tab: &mut Tab) {
    let mut filter = tab.filter().to_owned();
    let edit = egui::TextEdit::singleline(&mut filter)
        .hint_text(t("filter.hint"))
        .align(egui::Align2::LEFT_CENTER)
        .desired_width(160.0);
    if ui.add(edit).changed() {
        tab.set_filter(&filter);
    }
}

/// Breadcrumbs, or the path field while the user types; returns the folder to go to.
fn path_area(
    ui: &mut Ui,
    explorer: &Explorer,
    path_edit: &mut Option<PathEdit>,
) -> Option<PathBuf> {
    ui.spacing_mut().item_spacing.x = 2.0;
    if let Some(edit) = path_edit {
        let result = edit_path(ui, edit);
        if !matches!(result, EditResult::Editing) {
            *path_edit = None;
        }
        return match result {
            EditResult::Go(path) => Some(path),
            EditResult::Editing | EditResult::Cancel => None,
        };
    }
    let goto = crumbs(ui, explorer.active().path());
    // The rest of the breadcrumb area: click to type a path.
    let rest = vec2(ui.available_width().max(0.0), CRUMB_HEIGHT);
    let (_, resp) = ui.allocate_exact_size(rest, Sense::click());
    let resp = resp
        .on_hover_cursor(CursorIcon::Text)
        .on_hover_text(t("tip.edit_path"));
    if resp.clicked() {
        *path_edit = Some(PathEdit {
            text: explorer.active().path().display().to_string(),
            tab: explorer.active_index(),
            origin: explorer.active().path().to_path_buf(),
            fresh: true,
        });
    }
    goto
}

/// The clickable path segments; returns the folder of the one clicked.
fn crumbs(ui: &mut Ui, path: &Path) -> Option<PathBuf> {
    let crumbs = breadcrumbs(path);
    let last = crumbs.len().saturating_sub(1);
    let mut goto = None;
    for (i, (label, path)) in crumbs.into_iter().enumerate() {
        if crumb(ui, &label, i == last) {
            goto = Some(path);
        }
        if i != last {
            ui.label(RichText::new("›").color(theme::dim()));
        }
    }
    goto
}

enum EditResult {
    Editing,
    Cancel,
    Go(PathBuf),
}

/// A focused single-line field filling the breadcrumb area. Enter goes, Escape or losing focus cancels.
fn edit_path(ui: &mut Ui, edit: &mut PathEdit) -> EditResult {
    let id = ui.make_persistent_id("path-edit");
    if edit.fresh {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let out = TextEdit::singleline(&mut edit.text)
        .id(id)
        .align(egui::Align2::LEFT_CENTER)
        .desired_width(ui.available_width())
        .show(ui);
    if edit.fresh {
        edit.fresh = false;
        let mut state = out.state;
        state
            .cursor
            .set_char_range(Some(CCursorRange::select_all_str(&edit.text)));
        state.store(ui.ctx(), id);
        return EditResult::Editing;
    }
    let resp = out.response.response;
    let escape =
        resp.has_focus() && ui.input_mut(|i| i.consume_key(Default::default(), Key::Escape));
    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
        return parse_typed_path(&edit.text).map_or(EditResult::Cancel, EditResult::Go);
    }
    if escape || resp.lost_focus() {
        EditResult::Cancel
    } else {
        EditResult::Editing
    }
}

/// One clickable breadcrumb: padded, highlighted and underlined on hover, accent when current.
fn crumb(ui: &mut Ui, label: &str, current: bool) -> bool {
    let font = TextStyle::Body.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, theme::text());
    let size = vec2((galley.size().x + 12.0).max(CRUMB_HEIGHT), CRUMB_HEIGHT);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    let color = match (current, hovered) {
        (true, _) => theme::accent(),
        (false, true) => theme::header_text(),
        (false, false) => theme::text(),
    };
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 3.0, theme::panel_hi());
    }
    let text_pos = rect.center() - galley.size() / 2.0;
    painter.galley(text_pos, galley.clone(), color);
    if hovered {
        let y = text_pos.y + galley.size().y;
        painter.hline(
            text_pos.x..=text_pos.x + galley.size().x,
            y,
            Stroke::new(1.0, color),
        );
    }
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn toolbar_renders_without_text_glyphs() {
        let mut explorer = Explorer::new(std::env::temp_dir(), || {});
        let mut path_edit = None;
        let mut harness = Harness::new_ui(|ui| show(ui, &mut explorer, &mut path_edit));
        harness.run();
        harness.get_by_label(t("tip.back"));
        harness.get_by_label(t("tip.refresh"));
    }

    #[test]
    fn typed_paths_are_trimmed_and_unquoted() {
        assert_eq!(parse_typed_path("  /usr/bin \n"), Some("/usr/bin".into()));
        assert_eq!(
            parse_typed_path("\"C:\\My Dir\""),
            Some("C:\\My Dir".into())
        );
        assert_eq!(parse_typed_path(" ' /tmp ' "), Some("/tmp".into()));
        assert_eq!(parse_typed_path("  "), None);
        assert_eq!(parse_typed_path("\"\""), None);
    }
}
