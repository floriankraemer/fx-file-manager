use super::entries::{self, EntryContext, EntryInput};
use super::menu;
use crate::{action::Action, theme};
use eframe::egui::{Align, CursorIcon, Layout, RichText, Sense, Stroke, Ui};
use egui_extras::{Column, TableBuilder, TableRow};
use fx_app::{
    Density, DirView, Explorer, ListColumn, LoadState, Settings, SortColumn, SortSpec, age,
    extension, format_size, format_time, tr,
};
use fx_core::{Entry, EntryKind};
use std::path::Path;
use std::time::SystemTime;

pub(super) fn row_height(density: Density) -> f32 {
    match density {
        Density::Compact => 20.0,
        Density::Comfortable => 28.0,
    }
}

/// Read-only state every row needs, snapshotted before the table borrows the UI.
struct RowContext<'a> {
    entries: EntryContext<'a>,
    settings: &'a Settings,
    now: SystemTime,
}

/// What the user did in the table this frame; applied once rendering is done.
#[derive(Default)]
struct Interaction {
    entries: EntryInput,
    sort: Option<SortColumn>,
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
        entries: EntryContext {
            explorer,
            view: &view,
            tab,
            folder: tab.path(),
            single_click: settings.list_single_click,
            can_paste,
        },
        settings,
        now: SystemTime::now(),
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
                |mut row| entry_row(&mut row, &rows, &mut out.entries, actions),
            );
        });
    if let Some(drag) = &out.column_drag {
        paint_column_drag(ui, drag, &spans, &settings.list_columns);
    }
    out.column_widths = store_header_layout(ui, layout_key, &last, settings, spans);
    let empty = entries::below_content(&scroll);
    entries::background(ui, empty, settings.show_hidden, can_paste, actions);
    apply_columns(explorer, &out);
    entries::apply(ui, explorer, out.entries, modifiers);
    if let Some(column) = out.sort {
        explorer.active_mut().set_sort_column(column);
    }
}

/// The loaded listing; while loading or after a failure it draws that state and returns `None`.
pub(super) fn loaded_view(ui: &mut Ui, tab: &fx_app::Tab) -> Option<std::sync::Arc<DirView>> {
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
    out: &mut EntryInput,
    actions: &mut Vec<Action>,
) {
    let i = row.index();
    let e = &ctx.entries;
    let entry = &e.view.listing.entries[e.tab.order()[i] as usize];
    let name = e.view.listing.name(entry).to_string_lossy();
    row.set_selected(e.tab.is_selected(i));
    entry_cells(row, entry, &name, ctx);
    entries::respond(&row.response(), i, &name, entry.kind, e, out, actions);
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
