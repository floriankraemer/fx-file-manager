use super::{SearchAction, State};
use crate::command::{Command, hint};
use crate::views::icons::{self, Glyph};
use crate::{action::Action, theme};
use eframe::egui::{
    self, Align, Color32, Id, Key, Layout, Modifiers, Rect, Role, Sense, Stroke, StrokeKind,
    TextEdit, TextStyle, Ui, WidgetInfo,
    text::{CCursor, CCursorRange},
    vec2,
};
use fx_app::{Explorer, SearchView, t, tr};
use std::path::Path;

const BAR_HEIGHT: f32 = 28.0;
const ICON_SLOT: f32 = 20.0;
const CHIP_HEIGHT: f32 = 20.0;
const TOGGLE_WIDTH: f32 = 30.0;
const LABEL_MAX_WIDTH: f32 = 220.0;
/// Below this width the label moves into the field's hint and the end button drops its text.
const NARROW: f32 = 520.0;

fn folder_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn field_id() -> Id {
    Id::unique("search-field")
}

fn raise(actions: &mut Vec<Action>, action: SearchAction) {
    actions.push(Action::Search(action));
}

/// The search panel under the toolbar; nothing while the window has no search bar.
pub fn bar(ui: &mut Ui, explorer: &Explorer, state: &mut State, actions: &mut Vec<Action>) {
    state.sync(explorer);
    if !state.open {
        return;
    }
    egui::Panel::top("search").show(ui, |ui| row(ui, explorer, state, actions));
}

/// One 28 px row: icon, label, the field, then the fixed-size chips and buttons on the right.
fn row(ui: &mut Ui, explorer: &Explorer, state: &mut State, actions: &mut Vec<Action>) {
    let view = explorer.search();
    let narrow = ui.available_width() < NARROW;
    ui.horizontal(|ui| {
        ui.spacing_mut().interact_size.y = BAR_HEIGHT;
        let (slot, _) = ui.allocate_exact_size(vec2(ICON_SLOT, BAR_HEIGHT), Sense::hover());
        icons::glyph(ui, slot, Glyph::Search, theme::dim());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            end_button(ui, view, narrow, actions);
            glob_chip(ui, view);
            case_toggle(ui, explorer.settings().search.case_sensitive, actions);
            if view.is_some_and(SearchView::on_network) {
                chip(
                    ui,
                    t("search.network"),
                    theme::dim(),
                    true,
                    t("search.network_tip"),
                );
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let label = tr(
                    "search.label",
                    &[("folder", &folder_name(explorer.active().path()))],
                );
                field(ui, &label, narrow, state, explorer, actions);
            });
        });
    });
}

/// Stop while the walk runs (in the error colour on a network drive, where stopping matters most),
/// Clear once it ended.
fn end_button(ui: &mut Ui, view: Option<&SearchView>, narrow: bool, actions: &mut Vec<Action>) {
    let running = view.is_some_and(SearchView::is_running);
    let (glyph, key, action, color) = if running {
        let network = view.is_some_and(SearchView::on_network);
        let color = if network {
            theme::error()
        } else {
            theme::text()
        };
        (Glyph::Stop, "search.stop", SearchAction::Stop, color)
    } else {
        (
            Glyph::Clear,
            "search.clear",
            SearchAction::Close,
            theme::text(),
        )
    };
    let label = format!("{} · {}", t(key), hint(Command::CloseSearch));
    let shown = if narrow { "" } else { label.as_str() };
    let response = glyph_button(ui, glyph, shown, &label, color);
    if response.on_hover_text(&label).clicked() {
        raise(actions, action);
    }
}

fn glyph_button(
    ui: &mut Ui,
    glyph: Glyph,
    shown: &str,
    label: &str,
    color: Color32,
) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        shown.to_owned(),
        TextStyle::Button.resolve(ui.style()),
        color,
    );
    let pad = 8.0;
    let gap = if shown.is_empty() { 0.0 } else { 4.0 };
    let width = pad * 2.0 + ICON_SLOT + gap + galley.size().x;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, BAR_HEIGHT), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, theme::panel_hi());
    }
    let icon = Rect::from_min_size(rect.min + vec2(pad, 0.0), vec2(ICON_SLOT, BAR_HEIGHT));
    icons::glyph(ui, icon, glyph, color);
    let text_at = egui::pos2(icon.right() + gap, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_at, galley, color);
    resp
}

/// Lit while the search runs as a glob; unlit before any search exists.
fn glob_chip(ui: &mut Ui, view: Option<&SearchView>) {
    let lit = view.is_some_and(SearchView::is_glob);
    chip(
        ui,
        t("search.glob_chip"),
        theme::accent(),
        lit,
        t("search.glob_tip"),
    );
}

fn chip(ui: &mut Ui, text: &str, color: Color32, lit: bool, tip: &str) {
    let color = if lit {
        color
    } else {
        theme::dim().gamma_multiply(0.7)
    };
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), TextStyle::Small.resolve(ui.style()), color);
    let size = vec2(galley.size().x + 14.0, CHIP_HEIGHT);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    if lit {
        painter.rect_filled(rect, 8.0, color.gamma_multiply(0.2));
    }
    painter.rect_stroke(rect, 8.0, Stroke::new(1.0, color), StrokeKind::Inside);
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    resp.on_hover_text(tip);
}

/// The `Aa` button: selected while searches match case.
fn case_toggle(ui: &mut Ui, on: bool, actions: &mut Vec<Action>) {
    let label = t("search.case_tip");
    let color = if on { theme::accent() } else { theme::dim() };
    let galley = ui.painter().layout_no_wrap(
        t("search.case_chip").to_owned(),
        TextStyle::Button.resolve(ui.style()),
        color,
    );
    let (rect, resp) = ui.allocate_exact_size(vec2(TOGGLE_WIDTH, BAR_HEIGHT), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(Role::Button, true, on, label));
    let painter = ui.painter();
    if on {
        painter.rect_filled(rect.shrink2(vec2(0.0, 2.0)), 4.0, color.gamma_multiply(0.2));
    } else if resp.hovered() {
        painter.rect_filled(rect, 4.0, theme::panel_hi());
    }
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    if resp.on_hover_text(label).clicked() {
        raise(actions, SearchAction::ToggleCase);
        ui.memory_mut(|m| m.request_focus(field_id()));
    }
}

/// The query field. Every change restarts the search; arrows move the result selection while it has focus.
/// Wide bars show `label` before it, narrow ones use it as the hint.
fn field(
    ui: &mut Ui,
    label: &str,
    narrow: bool,
    state: &mut State,
    explorer: &Explorer,
    actions: &mut Vec<Action>,
) {
    if !narrow {
        bar_label(ui, label);
    }
    let id = field_id();
    let hint_text = if narrow { label } else { "" };
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
        raise(actions, SearchAction::Start(state.text.clone()));
    }
    if out.response.has_focus() {
        arrows(ui, actions);
    }
    if out.response.lost_focus() {
        leave_field(ui, explorer, actions);
    }
}

fn bar_label(ui: &mut Ui, label: &str) {
    let font = TextStyle::Body.resolve(ui.style());
    let natural = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, theme::dim());
    let width = natural.size().x.min(LABEL_MAX_WIDTH);
    let text = egui::RichText::new(label).color(theme::dim());
    ui.add_sized([width, BAR_HEIGHT], egui::Label::new(text).truncate());
}

fn arrows(ui: &Ui, actions: &mut Vec<Action>) {
    for (key, delta) in [(Key::ArrowDown, 1), (Key::ArrowUp, -1)] {
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
            raise(actions, SearchAction::MoveSelection(delta));
        }
    }
}

/// Enter opens the selected result (the first when none is selected); Esc stops or closes.
fn leave_field(ui: &Ui, explorer: &Explorer, actions: &mut Vec<Action>) {
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        raise(actions, SearchAction::Escape);
    } else if ui.input(|i| i.key_pressed(Key::Enter))
        && let Some(search) = explorer.search().filter(|s| s.matches() > 0)
    {
        raise(
            actions,
            SearchAction::OpenHit(search.selected().unwrap_or(0)),
        );
    }
}
