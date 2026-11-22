use super::dnd;
use super::menu;
use crate::command::{Command, hint};
use crate::{action::Action, theme};
use eframe::egui::scroll_area::ScrollAreaOutput;
use eframe::egui::{Align, CursorIcon, Layout, Modifiers, RichText, Sense, Stroke, Ui};
use egui_extras::{Column, TableBuilder, TableRow};
use fx_app::{
    Density, DirView, Explorer, ListColumn, LoadState, Settings, SortColumn, SortSpec, age,
    extension, format_size, format_time, t, tr,
};
use fx_core::{Entry, EntryKind};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub(super) fn row_height(density: Density) -> f32 {
    match density {
        Density::Compact => 20.0,
        Density::Comfortable => 28.0,
    }
}

/// Read-only state every row needs, snapshotted before the table borrows the UI.
struct RowContext<'a> {
    view: &'a DirView,
    tab: &'a fx_app::Tab,
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
    /// Dragged paths released on a folder row, with that folder.
    drop: Option<(Vec<PathBuf>, PathBuf)>,
    /// The header being dragged this frame.
    column_drag: Option<ColumnDrag>,
    show_column: Option<(ListColumn, bool)>,
    /// A header dragged onto another column's slot.
    move_column: Option<(ListColumn, ListColumn)>,
    /// Widths of the columns the user finished resizing.
    column_widths: Option<Vec<(ListColumn, f32)>>,
}

/// A header drag in progress: positions in the header row, and the header's own rectangle.
struct ColumnDrag {
    from: usize,
    to: usize,
    header: egui::Rect,
}

/// Header cell extents of the previous frame. Drops resolve against them because later cells are not laid out yet.
#[derive(Clone, Default)]
struct HeaderLayout {
    columns: Vec<ListColumn>,
    spans: Vec<(f32, f32)>,
    /// A resize happened and has not been saved yet.
    resizing: bool,
}

impl HeaderLayout {
    fn width(&self, i: usize) -> f32 {
        self.spans[i].1 - self.spans[i].0
    }

    /// The layout after this frame, and the widths to save once a resize has settled.
    fn advance(
        &self,
        columns: &[ListColumn],
        spans: Vec<(f32, f32)>,
        pointer_down: bool,
    ) -> (Self, Option<Vec<(ListColumn, f32)>>) {
        let mut next = Self {
            columns: columns.to_vec(),
            spans,
            resizing: false,
        };
        // After a reorder or first frame there is nothing to compare, so no resize is assumed.
        if self.columns != next.columns || self.spans.len() != next.spans.len() {
            return (next, None);
        }
        let changed = (0..self.spans.len()).any(|i| (self.width(i) - next.width(i)).abs() > 0.5);
        next.resizing = self.resizing || changed;
        if !next.resizing || changed || pointer_down {
            return (next, None);
        }
        next.resizing = false;
        // The last column only fills what the others leave.
        let fixed = columns.len() - 1;
        let widths = (0..fixed).map(|i| (columns[i], next.width(i))).collect();
        (next, Some(widths))
    }
}

/// What the header cells of one frame read and report.
struct HeaderContext<'a> {
    sort: SortSpec,
    settings: &'a Settings,
    last: &'a HeaderLayout,
}

pub fn show(
    ui: &mut Ui,
    explorer: &mut Explorer,
    scroll_to_selection: bool,
    actions: &mut Vec<Action>,
) {
    // Selectable labels grab clicks and drags, so rows would never see double/right clicks.
    ui.style_mut().interaction.selectable_labels = false;
    let can_paste = explorer.can_paste();
    let settings = explorer.settings();
    let tab = explorer.active();
    let Some(view) = loaded_view(ui, tab) else {
        return;
    };

    let sort = tab.sort();
    let scroll_row = tab.selected().filter(|_| scroll_to_selection);
    let order_len = tab.order().len();
    let list_font_size = settings.list_font_size;
    let density = settings.density;
    let modifiers = ui.input(|input| input.modifiers);
    let rows = RowContext {
        view: &view,
        tab,
        folder: tab.path(),
        settings,
        now: SystemTime::now(),
        can_paste,
    };
    let mut out = Interaction::default();
    let layout_key = ui.scope_id().with("header-layout");
    let last: HeaderLayout = ui.data(|d| d.get_temp(layout_key)).unwrap_or_default();
    let mut spans = Vec::new();
    let header = HeaderContext {
        sort,
        settings,
        last: &last,
    };

    let scroll = table_builder(ui, settings, scroll_row)
        .header(22.0, |row| header_row(row, &header, &mut spans, &mut out))
        .body(|body| {
            body.rows(
                theme::row_height(row_height(density), list_font_size),
                order_len,
                |mut row| entry_row(&mut row, &rows, explorer, &mut out, actions),
            );
        });
    if let Some(drag) = &out.column_drag {
        paint_column_drag(ui, drag, &spans, &settings.list_columns);
    }
    out.column_widths = store_header_layout(ui, layout_key, &last, settings, spans);
    background_area(ui, &scroll, settings.show_hidden, can_paste, actions);
    apply(ui, explorer, out, modifiers);
}

/// The loaded listing; while loading or after a failure it draws that state and returns `None`.
fn loaded_view(ui: &mut Ui, tab: &fx_app::Tab) -> Option<std::sync::Arc<DirView>> {
    match tab.state() {
        LoadState::Loading => {
            ui.centered_and_justified(|ui| {
                ui.spinner();
            });
            None
        }
        LoadState::Failed(message) => {
            load_error(ui, tab.path(), message);
            None
        }
        LoadState::Loaded(view) => Some(view.clone()),
    }
}

/// Remembers this frame's header extents; returns the widths to save once a resize has settled.
fn store_header_layout(
    ui: &Ui,
    key: egui::Id,
    last: &HeaderLayout,
    settings: &Settings,
    spans: Vec<(f32, f32)>,
) -> Option<Vec<(ListColumn, f32)>> {
    let pointer_down = ui.input(|i| i.pointer.any_down());
    let (layout, widths) = last.advance(&settings.list_columns, spans, pointer_down);
    ui.data_mut(|d| d.insert_temp(key, layout));
    widths
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
    let columns = &settings.list_columns;
    // Column widths are kept by position, so a new order must start from the saved widths again.
    let mut builder = TableBuilder::new(ui)
        .id_salt(("list-columns", columns))
        .striped(true)
        .resizable(true)
        .sense(Sense::click_and_drag())
        .cell_layout(Layout::left_to_right(Align::Center));
    for (i, column) in columns.iter().enumerate() {
        let spec = if i + 1 == columns.len() {
            Column::remainder()
        } else {
            Column::initial(settings.column_width(*column))
        };
        builder = builder.column(spec.at_least(column.min_width()).clip(true));
    }
    if let Some(row) = scroll_row {
        builder = builder.scroll_to_row(row, None);
    }
    builder
}

fn sort_column(column: ListColumn) -> Option<SortColumn> {
    match column {
        ListColumn::Name => Some(SortColumn::Name),
        ListColumn::Ext => Some(SortColumn::Ext),
        ListColumn::Size => Some(SortColumn::Size),
        ListColumn::Modified => Some(SortColumn::Modified),
        ListColumn::Age => None,
    }
}

/// The column headers; records each cell's extent in `spans`.
fn header_row(
    mut header: TableRow,
    ctx: &HeaderContext,
    spans: &mut Vec<(f32, f32)>,
    out: &mut Interaction,
) {
    for (i, column) in ctx.settings.list_columns.iter().enumerate() {
        header.col(|ui| {
            let r = ui.max_rect();
            spans.push((r.left(), r.right()));
            header_cell(ui, i, *column, ctx, out);
        });
    }
}

fn sort_arrow(column: Option<SortColumn>, sort: SortSpec) -> &'static str {
    if column != Some(sort.column) {
        return "";
    }
    if sort.ascending { " ▲" } else { " ▼" }
}

/// One header cell: click sorts, dragging moves the column, right click lists the columns to show.
fn header_cell(
    ui: &mut Ui,
    index: usize,
    column: ListColumn,
    ctx: &HeaderContext,
    out: &mut Interaction,
) {
    let sort_by = sort_column(column);
    let arrow = sort_arrow(sort_by, ctx.sort);
    let text = RichText::new(format!("{}{arrow}", column.header()))
        .strong()
        .color(theme::header_text());
    let r = ui.max_rect();
    ui.painter().hline(
        r.x_range(),
        r.bottom(),
        Stroke::new(1.5, theme::accent().gamma_multiply(0.5)),
    );
    // The whole cell is the click target, not just the label.
    let resp = ui.interact(r, ui.scope_id().with("sort"), Sense::click_and_drag());
    ui.add(egui::Label::new(text).selectable(false));
    if resp.clicked() {
        out.sort = sort_by;
    }
    resp.context_menu(|ui| column_menu(ui, ctx.settings, out));
    column_drag(ui, &resp, index, ctx, out);
}

/// Dragging a header onto another column's slot moves it there; a line marks the slot while dragging.
fn column_drag(
    ui: &Ui,
    resp: &egui::Response,
    index: usize,
    ctx: &HeaderContext,
    out: &mut Interaction,
) {
    if !resp.dragged() && !resp.drag_stopped() {
        return;
    }
    let Some(to) = ui
        .input(|i| i.pointer.interact_pos())
        .and_then(|p| nearest_span(&ctx.last.spans, p.x))
    else {
        return;
    };
    let columns = &ctx.settings.list_columns;
    if resp.drag_stopped() {
        out.move_column = (to != index && to < columns.len() && index < columns.len())
            .then(|| (columns[index], columns[to]));
        return;
    }
    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    out.column_drag = Some(ColumnDrag {
        from: index,
        to,
        header: ui.max_rect(),
    });
}

/// Index of the span containing `x`, or the closest one.
fn nearest_span(spans: &[(f32, f32)], x: f32) -> Option<usize> {
    let gap = |(left, right): (f32, f32)| (left - x).max(x - right).max(0.0);
    (0..spans.len()).min_by(|a, b| gap(spans[*a]).total_cmp(&gap(spans[*b])))
}

/// Marks the drag over the whole table: the dragged column tinted, the target slot as a line, the name following the pointer.
fn paint_column_drag(ui: &Ui, drag: &ColumnDrag, spans: &[(f32, f32)], columns: &[ListColumn]) {
    let (Some(&(left, right)), Some(pointer)) =
        (spans.get(drag.from), ui.input(|i| i.pointer.interact_pos()))
    else {
        return;
    };
    let bottom = ui.max_rect().bottom();
    let y = drag.header.top()..=bottom;
    let accent = theme::accent();
    let painter = ui.painter();
    let source = egui::Rect::from_x_y_ranges(left..=right, y.clone());
    painter.rect_filled(source, 0.0, accent.gamma_multiply(0.14));
    let to = spans[drag.to.min(spans.len() - 1)];
    let edge = if drag.to > drag.from { to.1 } else { to.0 };
    painter.vline(edge, y, Stroke::new(3.0, accent));
    painter.circle_filled(egui::pos2(edge, drag.header.top() + 1.0), 4.0, accent);
    paint_drag_chip(ui, pointer, columns[drag.from].header());
}

/// A small label that follows the pointer so the dragged column stays identifiable.
fn paint_drag_chip(ui: &Ui, pointer: egui::Pos2, title: &str) {
    let layer = egui::LayerId::new(egui::Order::Tooltip, ui.scope_id().with("column-chip"));
    let painter = ui.ctx().layer_painter(layer);
    let galley = painter.layout_no_wrap(
        title.to_owned(),
        egui::TextStyle::Body.resolve(ui.style()),
        theme::on_accent(),
    );
    let rect = egui::Rect::from_min_size(pointer + egui::vec2(12.0, 10.0), galley.size())
        .expand2(egui::vec2(8.0, 3.0));
    painter.rect_filled(rect, 6.0, theme::accent());
    painter.galley(rect.min + egui::vec2(8.0, 3.0), galley, theme::on_accent());
}

/// The header context menu: a check entry per column except Name, which cannot be hidden.
fn column_menu(ui: &mut Ui, settings: &Settings, out: &mut Interaction) {
    for column in ListColumn::ALL.into_iter().filter(|c| c.hideable()) {
        let shown = settings.column_shown(column);
        if menu::check(ui, column.label(), shown) {
            out.show_column = Some((column, !shown));
        }
    }
}

fn entry_row(
    row: &mut TableRow,
    ctx: &RowContext,
    explorer: &Explorer,
    out: &mut Interaction,
    actions: &mut Vec<Action>,
) {
    let i = row.index();
    let entry = &ctx.view.listing.entries[ctx.tab.order()[i] as usize];
    let name = ctx.view.listing.name(entry).to_string_lossy();
    let is_selected = ctx.tab.is_selected(i);
    row.set_selected(is_selected);
    entry_cells(row, entry, &name, ctx);

    let resp = row.response();
    if resp.secondary_clicked() && !is_selected {
        out.right_clicked = Some(i);
    }
    let is_dir = entry.kind == EntryKind::Dir;
    resp.context_menu(|ui| {
        let menu = RowMenu {
            row: i,
            is_dir,
            bookmarked: is_dir && explorer.is_bookmarked(&ctx.folder.join(name.as_ref())),
            selected: if is_selected {
                ctx.tab.selection_len()
            } else {
                1
            },
            can_paste: ctx.can_paste,
        };
        row_menu(ui, &menu, actions);
    });
    if is_dir
        && dnd::recording(&resp.ctx)
        && let Some(drop) = drop_on_folder(&resp, ctx.folder.join(name.as_ref()))
    {
        out.drop = Some(drop);
    }
    record_clicks(&resp, i, is_dir && ctx.settings.list_single_click, out);
}

fn record_clicks(resp: &egui::Response, row: usize, single_click: bool, out: &mut Interaction) {
    if resp.clicked() {
        out.select = Some(row);
    }
    if resp.drag_started() {
        out.drag_started = Some(row);
    }
    let activated = if single_click {
        resp.clicked()
    } else {
        resp.double_clicked()
    };
    if activated {
        out.activate = Some(row);
    }
}

/// Highlights a folder row while entries hover it; returns them with the folder once released there.
fn drop_on_folder(resp: &egui::Response, folder: PathBuf) -> Option<(Vec<PathBuf>, PathBuf)> {
    let dragged = dnd::target(resp, &folder)?;
    Some((dragged.paths.clone(), folder))
}

fn entry_cells(row: &mut TableRow, entry: &Entry, name: &str, ctx: &RowContext) {
    for column in &ctx.settings.list_columns {
        row.col(|ui| match column {
            ListColumn::Name => name_cell(ui, entry, name),
            ListColumn::Ext => ext_cell(ui, entry, name),
            ListColumn::Size => size_cell(ui, entry),
            ListColumn::Modified => modified_cell(ui, entry),
            ListColumn::Age => age_cell(ui, entry, ctx.now),
        });
    }
}

fn name_cell(ui: &mut Ui, entry: &Entry, name: &str) {
    super::icons::entry(ui, entry.kind, name);
    ui.add(egui::Label::new(RichText::new(name).text_style(theme::list_style())).truncate());
}

fn ext_cell(ui: &mut Ui, entry: &Entry, name: &str) {
    if entry.kind == EntryKind::Dir {
        return;
    }
    ui.colored_label(
        theme::dim(),
        RichText::new(extension(name)).text_style(theme::list_style()),
    );
}

fn modified_cell(ui: &mut Ui, entry: &Entry) {
    if let Some(time) = entry.modified {
        ui.colored_label(
            theme::dim(),
            RichText::new(format_time(time)).text_style(theme::list_style()),
        );
    }
}

fn age_cell(ui: &mut Ui, entry: &Entry, now: SystemTime) {
    let (bucket, text) = age(entry.modified, now);
    if !text.is_empty() {
        age_badge(ui, &text, theme::age_color(bucket));
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
fn apply(ui: &Ui, explorer: &mut Explorer, out: Interaction, modifiers: Modifiers) {
    apply_columns(explorer, &out);
    apply_selection(explorer, &out, modifiers);
    if let Some(i) = out.activate {
        explorer.activate(i);
    }
    if let Some(i) = out.drag_started {
        start_drag(ui, explorer, i);
    }
    if let Some((paths, folder)) = out.drop {
        let copy = dnd::copying(ui.ctx(), explorer.settings().drag_copy_modifier);
        explorer.drop_into(paths, &folder, copy);
    }
}

fn apply_selection(explorer: &mut Explorer, out: &Interaction, modifiers: Modifiers) {
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
}

/// Persists the column changes of this frame in one settings update.
fn apply_columns(explorer: &mut Explorer, out: &Interaction) {
    if out.show_column.is_none() && out.move_column.is_none() && out.column_widths.is_none() {
        return;
    }
    let mut settings = explorer.settings().clone();
    if let Some((column, shown)) = out.show_column {
        settings.set_column_shown(column, shown);
    }
    if let Some((column, target)) = out.move_column {
        settings.move_column(column, target);
    }
    for (column, width) in out.column_widths.iter().flatten() {
        settings.list_column_widths.insert(*column, *width);
    }
    explorer.set_settings(settings);
}

fn start_drag(ui: &Ui, explorer: &mut Explorer, row: usize) {
    // Dragging an unselected row drags just that row, as in other file managers.
    if !explorer.active().is_selected(row) {
        explorer.active_mut().click(row, false, false);
    }
    let rows = explorer.active().selection();
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

/// What the row context menu offers.
struct RowMenu {
    row: usize,
    is_dir: bool,
    bookmarked: bool,
    selected: usize,
    can_paste: bool,
}

fn row_menu(ui: &mut Ui, menu: &RowMenu, actions: &mut Vec<Action>) {
    open_items(ui, menu, actions);
    ui.separator();
    clipboard_items(ui, menu, actions);
    ui.separator();
    if menu::item(
        ui,
        t("cmd.rename"),
        &hint(Command::Rename),
        menu.selected == 1,
    ) {
        actions.push(Action::Rename);
    }
    if menu::item(ui, t("cmd.delete"), &hint(Command::Delete), true) {
        actions.push(Action::Delete { permanent: false });
    }
    ui.separator();
    if menu::item(ui, t("cmd.copy_path"), "", true) {
        actions.push(Action::CopyPath);
    }
    let properties = hint(Command::Properties);
    if menu::item(ui, t("cmd.properties"), &properties, menu.selected == 1) {
        actions.push(Action::Properties);
    }
}

fn open_items(ui: &mut Ui, menu: &RowMenu, actions: &mut Vec<Action>) {
    if menu::item(ui, t("cmd.open"), "Enter", true) {
        actions.push(Action::Open(menu.row));
    }
    if menu.is_dir && menu::item(ui, t("cmd.open_new_tab"), "", true) {
        actions.push(Action::OpenInNewTab(menu.row));
    }
    if !menu.is_dir && menu::item(ui, t("cmd.open_with"), "", true) {
        actions.push(Action::OpenWith(menu.row));
    }
}

fn clipboard_items(ui: &mut Ui, menu: &RowMenu, actions: &mut Vec<Action>) {
    if menu::item(ui, t("cmd.cut"), &hint(Command::Cut), true) {
        actions.push(Action::Cut);
    }
    if menu::item(ui, t("cmd.copy"), &hint(Command::Copy), true) {
        actions.push(Action::Copy);
    }
    if !menu.is_dir {
        return;
    }
    if menu::item(ui, t("cmd.paste_into"), "", menu.can_paste) {
        actions.push(Action::PasteInto(menu.row));
    }
    let bookmark = if menu.bookmarked {
        "cmd.remove_bookmark"
    } else {
        "cmd.add_bookmark"
    };
    if menu::item(ui, t(bookmark), "", true) {
        actions.push(Action::ToggleBookmark(menu.row));
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
