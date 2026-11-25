use super::{SearchAction, State};
use crate::command::{Command, hint};
use crate::views::icons::{self, Glyph};
use crate::{action::Action, theme};
use eframe::egui::{
    self, Align, Key, Layout, Modifiers, Rect, RichText, Role, Sense, Stroke, TextEdit, TextStyle,
    Ui, WidgetInfo,
    text::{CCursor, CCursorRange},
    vec2,
};
use fx_app::{Explorer, SearchView, t, tr};
use std::path::Path;

const BAR_HEIGHT: f32 = 28.0;
const ICON_SLOT: f32 = 20.0;

fn folder_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// The search panel under the toolbar; nothing while the window has no search bar.
pub fn bar(ui: &mut Ui, explorer: &Explorer, state: &mut State, actions: &mut Vec<Action>) {
    state.sync(explorer);
    if !state.open {
        return;
    }
    egui::Panel::top("search").show(ui, |ui| row(ui, explorer, state, actions));
}

fn row(ui: &mut Ui, explorer: &Explorer, state: &mut State, actions: &mut Vec<Action>) {
    let running = explorer.search().is_some_and(SearchView::is_running);
    ui.horizontal(|ui| {
        ui.spacing_mut().interact_size.y = BAR_HEIGHT;
        let (slot, _) = ui.allocate_exact_size(vec2(ICON_SLOT, BAR_HEIGHT), Sense::hover());
        icons::glyph(ui, slot, Glyph::Search, theme::dim());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            end_button(ui, running, actions);
            if explorer.search().is_some_and(SearchView::is_glob) {
                chip(ui, t("search.glob"));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                field(ui, explorer, state, actions);
            });
        });
    });
}

/// Stop while the walk runs, Clear once it ended.
fn end_button(ui: &mut Ui, running: bool, actions: &mut Vec<Action>) {
    let (glyph, key, action) = if running {
        (
            Glyph::Stop,
            "search.stop",
            Action::Search(SearchAction::Stop),
        )
    } else {
        (
            Glyph::Clear,
            "search.clear",
            Action::Search(SearchAction::Close),
        )
    };
    let label = format!("{} · {}", t(key), hint(Command::CloseSearch));
    if glyph_button(ui, glyph, &label).clicked() {
        actions.push(action);
    }
}

fn glyph_button(ui: &mut Ui, glyph: Glyph, label: &str) -> egui::Response {
    let color = theme::text();
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        TextStyle::Button.resolve(ui.style()),
        color,
    );
    let pad = 8.0;
    let width = pad * 2.0 + ICON_SLOT + 4.0 + galley.size().x;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, BAR_HEIGHT), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, theme::panel_hi());
    }
    let icon = Rect::from_min_size(rect.min + vec2(pad, 0.0), vec2(ICON_SLOT, BAR_HEIGHT));
    icons::glyph(ui, icon, glyph, color);
    let text_at = egui::pos2(icon.right() + 4.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_at, galley, color);
    resp
}

fn chip(ui: &mut Ui, text: &str) {
    let color = theme::accent();
    egui::Frame::new()
        .fill(color.gamma_multiply(0.2))
        .stroke(Stroke::new(1.0, color))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().color(color));
        });
}

/// The query field. Every change restarts the search; arrows move the result selection while it has focus.
fn field(ui: &mut Ui, explorer: &Explorer, state: &mut State, actions: &mut Vec<Action>) {
    let id = ui.make_persistent_id("search-field");
    let hint_text = tr(
        "search.placeholder",
        &[("folder", &folder_name(explorer.active().path()))],
    );
    let mut out = TextEdit::singleline(&mut state.text)
        .id(id)
        .hint_text(hint_text)
        .align(egui::Align2::LEFT_CENTER)
        .desired_width(ui.available_width())
        .show(ui);
    if std::mem::take(&mut state.focus) {
        out.response.request_focus();
        let end = CCursor::new(state.text.chars().count());
        out.state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(0), end)));
        out.state.store(ui.ctx(), id);
    }
    if out.response.changed() {
        actions.push(Action::Search(SearchAction::Start(state.text.clone())));
    }
    if out.response.has_focus() {
        arrows(ui, actions);
    }
    if out.response.lost_focus() {
        leave_field(ui, explorer, actions);
    }
}

fn arrows(ui: &Ui, actions: &mut Vec<Action>) {
    for (key, delta) in [(Key::ArrowDown, 1), (Key::ArrowUp, -1)] {
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
            actions.push(Action::Search(SearchAction::MoveSelection(delta)));
        }
    }
}

/// Enter opens the selected result (the first when none is selected); Esc stops or closes.
fn leave_field(ui: &Ui, explorer: &Explorer, actions: &mut Vec<Action>) {
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        actions.push(Action::Search(SearchAction::Escape));
    } else if ui.input(|i| i.key_pressed(Key::Enter))
        && let Some(search) = explorer.search().filter(|s| s.matches() > 0)
    {
        actions.push(Action::Search(SearchAction::OpenHit(
            search.selected().unwrap_or(0),
        )));
    }
}
