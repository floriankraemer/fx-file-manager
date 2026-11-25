use super::{SearchAction, State};
use crate::command::{Command, hint};
use crate::views::icons;
use crate::views::{menu, table};
use crate::{action::Action, theme};
use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Stroke, Ui};
use egui_extras::{Column, TableBuilder, TableRow};
use fx_app::{Explorer, SearchStatus, SearchView, t, tr};
use fx_core::{EntryKind, Hit};

const NAME_WIDTH: f32 = 340.0;
const FOLDER_WIDTH: f32 = 300.0;
const KIND_MIN_WIDTH: f32 = 90.0;

/// What the user did in the result list this frame.
#[derive(Default)]
struct Interaction {
    select: Option<usize>,
    open: Option<usize>,
    reveal: Option<usize>,
    copy_path: Option<usize>,
}

pub fn results(ui: &mut Ui, explorer: &mut Explorer, state: &mut State, actions: &mut Vec<Action>) {
    ui.style_mut().interaction.selectable_labels = false;
    let scroll = std::mem::take(&mut state.scroll_to_selection);
    let mut out = Interaction::default();
    {
        let Some(view) = explorer.search() else {
            return;
        };
        banners(ui, view);
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

fn banners(ui: &mut Ui, view: &SearchView) {
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
        SearchStatus::Running if view.on_network() => {
            banner(ui, theme::dim(), t("search.network_slow"));
        }
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
        out.open
            .map(|row| Action::Search(SearchAction::OpenHit(row))),
        out.reveal
            .map(|row| Action::Search(SearchAction::RevealHit(row))),
        out.copy_path
            .map(|row| Action::Search(SearchAction::CopyHitPath(row))),
    ];
    actions.extend(raised.into_iter().flatten());
}
