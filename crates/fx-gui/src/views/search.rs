//! The search bar under the toolbar and the result list that replaces the folder table while a search exists.

use super::icons::{self, Glyph};
use super::{menu, table};
use crate::command::{Command, hint};
use crate::{action::Action, theme};
use eframe::egui::{
    self, Align, Color32, Key, Layout, Modifiers, Rect, RichText, Role, Sense, Stroke, TextEdit,
    TextStyle, Ui, WidgetInfo,
    text::{CCursor, CCursorRange},
    vec2,
};
use egui_extras::{Column, TableBuilder, TableRow};
use fx_app::{Explorer, LoadState, SearchStatus, SearchView, t, tr};
use fx_core::{EntryKind, Hit, StorageKind};
use std::path::{Path, PathBuf};

const BAR_HEIGHT: f32 = 28.0;
const ICON_SLOT: f32 = 20.0;
const NAME_WIDTH: f32 = 340.0;
const FOLDER_WIDTH: f32 = 300.0;
const KIND_MIN_WIDTH: f32 = 90.0;

/// What the bar of one window remembers between frames.
#[derive(Default)]
pub struct State {
    open: bool,
    text: String,
    focus: bool,
    /// Tab and folder the bar belongs to; showing anything else resets it to that tab's search.
    key: Option<(u64, PathBuf)>,
    scroll_to_selection: bool,
    /// The failure already logged, so the banner does not log it again every frame.
    logged_failure: Option<String>,
}

impl State {
    fn note_failure(&mut self, status: &SearchStatus) {
        let failure = match status {
            SearchStatus::Failed(error) => Some(error),
            _ => None,
        };
        if self.logged_failure.as_ref() == failure {
            return;
        }
        if let Some(error) = failure {
            log::warn!("search failed: {error}");
        }
        self.logged_failure = failure.cloned();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Shows the bar and focuses its field, selecting what is typed.
    pub fn open(&mut self) {
        self.open = true;
        self.focus = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.text.clear();
    }

    /// Follows the active tab: another tab or folder shows that tab's own search, or no bar.
    pub fn sync(&mut self, explorer: &Explorer) {
        let tab = explorer.active();
        let key = (tab.id(), tab.path().to_path_buf());
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.key = Some(key);
        self.open = explorer.search().is_some();
        self.text = explorer
            .search()
            .map(|s| s.query().to_owned())
            .unwrap_or_default();
    }
}

fn is_glob(text: &str) -> bool {
    text.contains(['*', '?'])
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

pub fn bar(ui: &mut Ui, explorer: &mut Explorer, state: &mut State, actions: &mut Vec<Action>) {
    let running = explorer.search().is_some_and(SearchView::is_running);
    ui.horizontal(|ui| {
        ui.spacing_mut().interact_size.y = BAR_HEIGHT;
        let (slot, _) = ui.allocate_exact_size(vec2(ICON_SLOT, BAR_HEIGHT), Sense::hover());
        icons::glyph(ui, slot, Glyph::Search, theme::dim());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            end_button(ui, running, actions);
            if is_glob(&state.text) {
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
        (Glyph::Stop, "search.stop", Action::StopSearch)
    } else {
        (Glyph::Clear, "search.clear", Action::CloseSearch)
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
fn field(ui: &mut Ui, explorer: &mut Explorer, state: &mut State, actions: &mut Vec<Action>) {
    let id = ui.make_persistent_id("search-field");
    let hint_text = tr(
        "search.placeholder",
        &[("folder", &folder_name(explorer.active().path()))],
    );
    let mut out = TextEdit::singleline(&mut state.text)
        .id(id)
        .hint_text(hint_text)
        .vertical_align(Align::Center)
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
        actions.push(Action::StartSearch(state.text.clone()));
    }
    if out.response.has_focus() {
        arrows(ui, explorer, state);
    }
    if out.response.lost_focus() {
        leave_field(ui, explorer, actions);
    }
}

fn arrows(ui: &Ui, explorer: &mut Explorer, state: &mut State) {
    for (key, delta) in [(Key::ArrowDown, 1), (Key::ArrowUp, -1)] {
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
            explorer.move_search_selection(delta);
            state.scroll_to_selection = true;
        }
    }
}

/// Enter opens the selected result (the first when none is selected); Esc stops or closes.
fn leave_field(ui: &Ui, explorer: &Explorer, actions: &mut Vec<Action>) {
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        actions.push(Action::EscapeSearch);
    } else if ui.input(|i| i.key_pressed(Key::Enter))
        && let Some(search) = explorer.search().filter(|s| s.matches() > 0)
    {
        actions.push(Action::OpenSearchHit(search.selected().unwrap_or(0)));
    }
}

/// What the user did in the result list this frame.
#[derive(Default)]
struct Interaction {
    select: Option<usize>,
    open: Option<usize>,
    reveal: Option<usize>,
    copy_path: Option<usize>,
}

pub fn results(
    ui: &mut Ui,
    explorer: &mut Explorer,
    state: &mut State,
    scroll_from_keys: bool,
    actions: &mut Vec<Action>,
) {
    ui.style_mut().interaction.selectable_labels = false;
    let scroll = scroll_from_keys || std::mem::take(&mut state.scroll_to_selection);
    let mut out = Interaction::default();
    {
        let Some(view) = explorer.search() else {
            return;
        };
        let network = matches!(
            explorer.active().state(),
            LoadState::Loaded(v) if v.storage == StorageKind::Network
        );
        state.note_failure(view.status());
        banners(ui, view, network);
        if view.matches() == 0 && !view.is_running() {
            empty(ui, view);
            return;
        }
        let rows = Rows {
            view,
            height: row_height(explorer),
            scroll_row: view.selected().filter(|_| scroll),
        };
        list(ui, &rows, &mut out);
    }
    apply(explorer, out, actions);
}

struct Rows<'a> {
    view: &'a SearchView,
    height: f32,
    scroll_row: Option<usize>,
}

fn row_height(explorer: &Explorer) -> f32 {
    let settings = explorer.settings();
    theme::row_height(table::row_height(settings.density), settings.list_font_size)
}

fn banners(ui: &mut Ui, view: &SearchView, network: bool) {
    match view.status() {
        SearchStatus::StoppedAtCap => banner(
            ui,
            theme::error(),
            &tr("search.cap_note", &[("n", &view.matches().to_string())]),
        ),
        SearchStatus::Failed(error) => {
            banner(
                ui,
                theme::error(),
                &tr("search.failed", &[("error", error)]),
            );
        }
        SearchStatus::Running if network => banner(ui, theme::dim(), t("search.network_slow")),
        _ => {}
    }
}

fn banner(ui: &mut Ui, color: Color32, text: &str) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.15))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(color));
        });
}

fn empty(ui: &mut Ui, view: &SearchView) {
    if matches!(view.status(), SearchStatus::Failed(_)) {
        return;
    }
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        let text = tr("search.no_matches", &[("query", view.query())]);
        ui.label(RichText::new(text).color(theme::dim()));
    });
}

fn list(ui: &mut Ui, rows: &Rows, out: &mut Interaction) {
    let mut builder = TableBuilder::new(ui)
        .id_salt("search-results")
        .striped(true)
        .resizable(true)
        .sense(Sense::click_and_drag())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::initial(NAME_WIDTH).at_least(80.0).clip(true))
        .column(Column::initial(FOLDER_WIDTH).at_least(80.0).clip(true))
        .column(Column::remainder().at_least(KIND_MIN_WIDTH).clip(true));
    if let Some(row) = rows.scroll_row {
        builder = builder.scroll_to_row(row, None);
    }
    builder.header(22.0, header).body(|body| {
        body.rows(rows.height, rows.view.matches(), |mut row| {
            result_row(&mut row, rows.view, out);
        });
    });
}

fn header(mut header: TableRow) {
    for title in [t("col.name"), t("search.col_folder"), t("prop.type")] {
        header.col(|ui| {
            let r = ui.max_rect();
            ui.painter().hline(
                r.x_range(),
                r.bottom(),
                Stroke::new(1.5, theme::accent().gamma_multiply(0.5)),
            );
            ui.add(
                egui::Label::new(RichText::new(title).strong().color(theme::header_text()))
                    .selectable(false),
            );
        });
    }
}

fn result_row(row: &mut TableRow, view: &SearchView, out: &mut Interaction) {
    let i = row.index();
    let hit = &view.hits()[i];
    let selected = view.selected() == Some(i);
    row.set_selected(selected);
    let name = hit
        .path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    row.col(|ui| name_cell(ui, hit.kind, &name));
    row.col(|ui| dim_cell(ui, &view.relative_folder(hit)));
    row.col(|ui| dim_cell(ui, kind_label(hit)));

    let resp = row.response();
    if resp.clicked() || (resp.secondary_clicked() && !selected) {
        out.select = Some(i);
    }
    if resp.double_clicked() {
        out.open = Some(i);
    }
    resp.context_menu(|ui| row_menu(ui, i, out));
}

fn name_cell(ui: &mut Ui, kind: EntryKind, name: &str) {
    icons::entry(ui, kind, name);
    ui.add(egui::Label::new(RichText::new(name).text_style(theme::list_style())).truncate());
}

fn dim_cell(ui: &mut Ui, text: &str) {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .color(theme::dim())
                .text_style(theme::list_style()),
        )
        .truncate(),
    );
}

fn kind_label(hit: &Hit) -> &'static str {
    match hit.kind {
        EntryKind::Dir => t("kind.folder"),
        EntryKind::File => t("kind.file"),
        EntryKind::Symlink => t("kind.symlink"),
        EntryKind::Other => t("kind.misc"),
    }
}

fn row_menu(ui: &mut Ui, row: usize, out: &mut Interaction) {
    if menu::item(ui, t("cmd.open"), "Enter", true) {
        out.open = Some(row);
    }
    if menu::item(
        ui,
        t("search.reveal"),
        &hint(Command::RevealSearchHit),
        true,
    ) {
        out.reveal = Some(row);
    }
    ui.separator();
    if menu::item(ui, t("cmd.copy_path"), "", true) {
        out.copy_path = Some(row);
    }
}

fn apply(explorer: &mut Explorer, out: Interaction, actions: &mut Vec<Action>) {
    if let Some(row) = out.select {
        explorer.select_search_hit(Some(row));
    }
    let raised = [
        out.open.map(Action::OpenSearchHit),
        out.reveal.map(Action::RevealSearchHit),
        out.copy_path.map(Action::CopySearchHitPath),
    ];
    actions.extend(raised.into_iter().flatten());
}
