use super::dnd::{self, Dragged};
use super::menu;
use crate::command::{Command, hint};
use crate::{action::Action, theme};
use eframe::egui::scroll_area::ScrollAreaOutput;
use eframe::egui::{Align, Layout, Modifiers, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder, TableRow};
use fx_app::{
    Density, DirView, Explorer, LoadState, Settings, SortColumn, SortSpec, age, extension,
    format_size, format_time, t, tr,
};
use fx_core::{Entry, EntryKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

fn row_height(density: Density) -> f32 {
    match density {
        Density::Compact => 20.0,
        Density::Comfortable => 28.0,
    }
}

/// Read-only state every row needs, snapshotted before the table borrows the UI.
struct RowContext<'a> {
    view: &'a DirView,
    order: &'a [u32],
    selection: &'a [usize],
    folder: &'a Path,
    settings: &'a Settings,
    now: SystemTime,
    can_paste: bool,
}

/// What the user did in the table this frame; applied once rendering is done.
#[derive(Default)]
struct Interaction {
    sort: Option<SortColumn>,
    select: Option<usize>,
    activate: Option<usize>,
    right_clicked: Option<usize>,
    drag_started: Option<usize>,
    dropped: Option<(Arc<Dragged>, PathBuf)>,
}

pub fn show(
    ui: &mut Ui,
    explorer: &mut Explorer,
    scroll_to_selection: bool,
    actions: &mut Vec<Action>,
) {
    // Selectable labels grab clicks and drags, so rows would never see double/right clicks.
    ui.style_mut().interaction.selectable_labels = false;
    let settings = explorer.settings().clone();
    let can_paste = explorer.can_paste();
    let tab = explorer.active();
    let view = match tab.state() {
        LoadState::Loading => {
            ui.centered_and_justified(|ui| {
                ui.spinner();
            });
            return;
        }
        LoadState::Failed(message) => {
            load_error(ui, tab.path(), message);
            return;
        }
        LoadState::Loaded(view) => view.clone(),
    };

    let sort = tab.sort();
    let scroll_row = tab.selected().filter(|_| scroll_to_selection);
    let selection = tab.selection();
    let order = tab.order().to_vec();
    let folder = tab.path().to_path_buf();
    let modifiers = ui.input(|input| input.modifiers);
    let rows = RowContext {
        view: &view,
        order: &order,
        selection: &selection,
        folder: &folder,
        settings: &settings,
        now: SystemTime::now(),
        can_paste,
    };
    let mut out = Interaction::default();

    let scroll = table_builder(ui, &settings, scroll_row)
        .header(22.0, |header| {
            out.sort = header_row(header, &settings, sort);
        })
        .body(|body| {
            body.rows(
                theme::row_height(row_height(settings.density), settings.list_font_size),
                order.len(),
                |mut row| entry_row(&mut row, &rows, explorer, &mut out, actions),
            );
        });
    background_area(ui, &scroll, settings.show_hidden, can_paste, actions);
    apply(ui, explorer, out, &selection, modifiers, &settings);
}

fn load_error(ui: &mut Ui, path: &Path, message: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.colored_label(
            theme::error(),
            tr(
                "table.cannot_list",
                &[("path", &path.display().to_string())],
            ),
        );
        ui.colored_label(theme::error(), message);
    });
}

fn table_builder<'a>(
    ui: &'a mut Ui,
    settings: &Settings,
    scroll_row: Option<usize>,
) -> TableBuilder<'a> {
    let mut builder = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(Sense::click_and_drag())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::remainder().at_least(160.0).clip(true));
    if settings.show_ext_column {
        builder = builder.column(Column::initial(60.0).at_least(30.0));
    }
    builder = builder
        .column(Column::initial(80.0).at_least(50.0))
        .column(Column::initial(140.0).at_least(80.0));
    if settings.show_age_column {
        builder = builder.column(Column::initial(56.0).at_least(40.0));
    }
    if let Some(row) = scroll_row {
        builder = builder.scroll_to_row(row, None);
    }
    builder
}

/// The column headers; returns the column whose header was clicked.
fn header_row(mut header: TableRow, settings: &Settings, sort: SortSpec) -> Option<SortColumn> {
    let columns = [
        (t("col.name"), Some(SortColumn::Name), true),
        (
            t("col_short.ext"),
            Some(SortColumn::Ext),
            settings.show_ext_column,
        ),
        (t("col.size"), Some(SortColumn::Size), true),
        (t("col.modified"), Some(SortColumn::Modified), true),
        (t("col.age"), None, settings.show_age_column),
    ];
    let mut clicked = None;
    for (title, column, _) in columns.into_iter().filter(|c| c.2) {
        header.col(|ui| {
            if let Some(column) = header_cell(ui, title, column, sort) {
                clicked = Some(column);
            }
        });
    }
    clicked
}

/// One header cell with its sort arrow; returns its column when clicked.
fn header_cell(
    ui: &mut Ui,
    title: &str,
    column: Option<SortColumn>,
    sort: SortSpec,
) -> Option<SortColumn> {
    let arrow = match column {
        Some(c) if c == sort.column => {
            if sort.ascending {
                " ▲"
            } else {
                " ▼"
            }
        }
        _ => "",
    };
    let text = RichText::new(format!("{title}{arrow}"))
        .strong()
        .color(theme::header_text());
    let r = ui.max_rect();
    ui.painter().hline(
        r.x_range(),
        r.bottom(),
        egui::Stroke::new(1.5, theme::accent().gamma_multiply(0.5)),
    );
    // The whole cell is the click target, not just the label.
    let resp = ui.interact(r, ui.scope_id().with("sort"), Sense::click());
    ui.add(egui::Label::new(text).selectable(false));
    column.filter(|_| resp.clicked())
}

fn entry_row(
    row: &mut TableRow,
    ctx: &RowContext,
    explorer: &Explorer,
    out: &mut Interaction,
    actions: &mut Vec<Action>,
) {
    let i = row.index();
    let entry = &ctx.view.listing.entries[ctx.order[i] as usize];
    let name = ctx.view.listing.name(entry).to_string_lossy();
    let is_selected = ctx.selection.binary_search(&i).is_ok();
    row.set_selected(is_selected);
    entry_cells(row, entry, &name, ctx);

    let resp = row.response();
    if resp.secondary_clicked() && !is_selected {
        out.right_clicked = Some(i);
    }
    let selected_count = if is_selected { ctx.selection.len() } else { 1 };
    let is_dir = entry.kind == EntryKind::Dir;
    resp.context_menu(|ui| {
        let bookmarked = is_dir && explorer.is_bookmarked(&ctx.folder.join(name.as_ref()));
        row_menu(
            ui,
            i,
            is_dir,
            bookmarked,
            selected_count,
            ctx.can_paste,
            actions,
        );
    });
    if resp.clicked() {
        out.select = Some(i);
    }
    if resp.drag_started() {
        out.drag_started = Some(i);
    }
    if is_dir {
        let dest = ctx.folder.join(name.as_ref());
        if let Some(sources) = dnd::target(&resp, &dest) {
            out.dropped = Some((sources, dest));
        }
    }
    let activated = if is_dir && ctx.settings.list_single_click {
        resp.clicked()
    } else {
        resp.double_clicked()
    };
    if activated {
        out.activate = Some(i);
    }
}

fn entry_cells(row: &mut TableRow, entry: &Entry, name: &str, ctx: &RowContext) {
    row.col(|ui| {
        super::icons::entry(ui, entry.kind, name);
        ui.add(egui::Label::new(RichText::new(name).text_style(theme::list_style())).truncate());
    });
    if ctx.settings.show_ext_column {
        row.col(|ui| {
            if entry.kind != EntryKind::Dir {
                ui.colored_label(
                    theme::dim(),
                    RichText::new(extension(name)).text_style(theme::list_style()),
                );
            }
        });
    }
    row.col(|ui| size_cell(ui, entry));
    row.col(|ui| {
        if let Some(time) = entry.modified {
            ui.colored_label(
                theme::dim(),
                RichText::new(format_time(time)).text_style(theme::list_style()),
            );
        }
    });
    if ctx.settings.show_age_column {
        row.col(|ui| {
            let (bucket, text) = age(entry.modified, ctx.now);
            if !text.is_empty() {
                age_badge(ui, &text, theme::age_color(bucket));
            }
        });
    }
}

fn size_cell(ui: &mut Ui, entry: &Entry) {
    if entry.kind == EntryKind::Dir {
        return;
    }
    let Some(size) = entry.size else {
        return;
    };
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.label(RichText::new(format_size(size)).text_style(theme::list_style()));
    });
}

/// The space below the last row; `body` reports the scroll area, so rows keep their own clicks.
fn background_area(
    ui: &mut Ui,
    scroll: &ScrollAreaOutput<()>,
    show_hidden: bool,
    can_paste: bool,
    actions: &mut Vec<Action>,
) {
    let rows_bottom =
        scroll.inner_rect.top() + scroll.content_size.y - scroll.state.unclamped_offset().y;
    let empty = egui::Rect::from_min_max(
        egui::pos2(
            scroll.inner_rect.left(),
            rows_bottom.max(scroll.inner_rect.top()),
        ),
        scroll.inner_rect.max,
    );
    if empty.height() > 1.0 {
        let background = ui.interact(
            empty,
            ui.scope_id().with("table-background"),
            Sense::click(),
        );
        background.context_menu(|ui| background_menu(ui, show_hidden, can_paste, actions));
    }
}

/// Applies the frame's interactions in a fixed order: selection first, then activation and drag and drop.
fn apply(
    ui: &Ui,
    explorer: &mut Explorer,
    out: Interaction,
    selection: &[usize],
    modifiers: Modifiers,
    settings: &Settings,
) {
    let tab = explorer.active_mut();
    if let Some(i) = out.right_clicked {
        tab.click(i, false, false);
    }
    if let Some(column) = out.sort {
        tab.set_sort_column(column);
    }
    if let Some(i) = out.select {
        tab.click(i, modifiers.command, modifiers.shift);
    }
    if let Some(i) = out.activate {
        explorer.activate(i);
    }
    if let Some(i) = out.drag_started {
        start_drag(ui, explorer, i, selection);
    }
    if let Some((sources, dest)) = out.dropped {
        let copy = dnd::copying(ui.ctx(), settings.drag_copy_modifier);
        explorer.drop_into(sources.paths.clone(), &dest, copy);
    }
}

fn start_drag(ui: &Ui, explorer: &mut Explorer, row: usize, selection: &[usize]) {
    // Dragging an unselected row drags just that row, as in other file managers.
    if selection.binary_search(&row).is_err() {
        explorer.active_mut().click(row, false, false);
    }
    let rows = explorer.active().selection().to_vec();
    let paths = rows
        .iter()
        .filter_map(|&r| explorer.entry_path(r))
        .collect();
    dnd::start(ui.ctx(), explorer.window(), paths);
}

fn age_badge(ui: &mut Ui, text: &str, color: egui::Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.25))
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(5, 0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().color(color));
        });
}

fn row_menu(
    ui: &mut Ui,
    row: usize,
    is_dir: bool,
    bookmarked: bool,
    selected: usize,
    can_paste: bool,
    actions: &mut Vec<Action>,
) {
    if menu::item(ui, t("cmd.open"), "Enter", true) {
        actions.push(Action::Open(row));
    }
    if is_dir && menu::item(ui, t("cmd.open_new_tab"), "", true) {
        actions.push(Action::OpenInNewTab(row));
    }
    #[cfg(windows)]
    if !is_dir && menu::item(ui, t("cmd.open_with"), "", true) {
        actions.push(Action::OpenWith(row));
    }
    ui.separator();
    if menu::item(ui, t("cmd.cut"), &hint(Command::Cut), true) {
        actions.push(Action::Cut);
    }
    if menu::item(ui, t("cmd.copy"), &hint(Command::Copy), true) {
        actions.push(Action::Copy);
    }
    if is_dir && menu::item(ui, t("cmd.paste_into"), "", can_paste) {
        actions.push(Action::PasteInto(row));
    }
    let bookmark = if bookmarked {
        "cmd.remove_bookmark"
    } else {
        "cmd.add_bookmark"
    };
    if is_dir && menu::item(ui, t(bookmark), "", true) {
        actions.push(Action::ToggleBookmark(row));
    }
    ui.separator();
    if menu::item(ui, t("cmd.rename"), &hint(Command::Rename), selected == 1) {
        actions.push(Action::Rename);
    }
    if menu::item(ui, t("cmd.delete"), &hint(Command::Delete), true) {
        actions.push(Action::Delete { permanent: false });
    }
    ui.separator();
    if menu::item(ui, t("cmd.copy_path"), "", true) {
        actions.push(Action::CopyPath);
    }
    if menu::item(
        ui,
        t("cmd.properties"),
        &hint(Command::Properties),
        selected == 1,
    ) {
        actions.push(Action::Properties);
    }
}

fn background_menu(ui: &mut Ui, show_hidden: bool, can_paste: bool, actions: &mut Vec<Action>) {
    if menu::item(ui, t("cmd.new_folder"), &hint(Command::NewFolder), true) {
        actions.push(Action::NewFolder);
    }
    if menu::item(ui, t("cmd.paste"), &hint(Command::Paste), can_paste) {
        actions.push(Action::Paste);
    }
    if menu::item(ui, t("cmd.refresh"), &hint(Command::Refresh), true) {
        actions.push(Action::Refresh);
    }
    if menu::item(ui, t("cmd.select_all"), &hint(Command::SelectAll), true) {
        actions.push(Action::SelectAll);
    }
    ui.separator();
    if menu::check(ui, t("settings.show_hidden"), show_hidden) {
        actions.push(Action::SetShowHidden(!show_hidden));
    }
    ui.separator();
    if menu::item(ui, t("cmd.settings"), &hint(Command::OpenSettings), true) {
        actions.push(Action::OpenSettings);
    }
}
