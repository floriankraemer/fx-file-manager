//! What the Details and Icons views share: clicks, drag and drop and context menus on entries.

use super::dnd;
use super::menu;
use crate::action::Action;
use crate::command::{Command, hint};
use eframe::egui::scroll_area::ScrollAreaOutput;
use eframe::egui::{self, Modifiers, Sense, Ui};
use fx_app::{DirView, Explorer, Tab, t};
use fx_core::EntryKind;
use std::path::{Path, PathBuf};

/// What the user did to the entries this frame; applied once rendering is done.
#[derive(Default)]
pub(super) struct EntryInput {
    pub(super) select: Option<usize>,
    pub(super) activate: Option<usize>,
    pub(super) right_clicked: Option<usize>,
    pub(super) drag_started: Option<usize>,
    /// Dragged paths released on a folder entry, with that folder.
    pub(super) drop: Option<(Vec<PathBuf>, PathBuf)>,
}

/// Read-only state every entry needs, snapshotted before the view borrows the UI.
pub(super) struct EntryContext<'a> {
    pub(super) explorer: &'a Explorer,
    pub(super) view: &'a DirView,
    pub(super) tab: &'a Tab,
    pub(super) folder: &'a Path,
    pub(super) single_click: bool,
    pub(super) can_paste: bool,
}

/// Wires one entry's response: context menu, drop onto folders, clicks.
pub(super) fn respond(
    resp: &egui::Response,
    row: usize,
    name: &str,
    kind: EntryKind,
    ctx: &EntryContext,
    out: &mut EntryInput,
    actions: &mut Vec<Action>,
) {
    let is_selected = ctx.tab.is_selected(row);
    if resp.secondary_clicked() && !is_selected {
        out.right_clicked = Some(row);
    }
    let is_dir = kind == EntryKind::Dir;
    resp.context_menu(|ui| {
        let menu = RowMenu {
            row,
            is_dir,
            bookmarked: is_dir && ctx.explorer.is_bookmarked(&ctx.folder.join(name)),
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
        && let Some(drop) = drop_on_folder(resp, ctx.folder.join(name))
    {
        out.drop = Some(drop);
    }
    record_clicks(resp, row, is_dir && ctx.single_click, out);
}

fn record_clicks(resp: &egui::Response, row: usize, single_click: bool, out: &mut EntryInput) {
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

/// Highlights a folder while entries hover it; returns them with the folder once released there.
fn drop_on_folder(resp: &egui::Response, folder: PathBuf) -> Option<(Vec<PathBuf>, PathBuf)> {
    let dragged = dnd::target(resp, &folder)?;
    Some((dragged.paths.clone(), folder))
}

/// The space `empty` not covered by entries gets the background menu.
pub(super) fn background(
    ui: &mut Ui,
    empty: egui::Rect,
    show_hidden: bool,
    can_paste: bool,
    actions: &mut Vec<Action>,
) {
    if empty.height() <= 1.0 {
        return;
    }
    let background = ui.interact(
        empty,
        ui.scope_id().with("entries-background"),
        Sense::click(),
    );
    background.context_menu(|ui| background_menu(ui, show_hidden, can_paste, actions));
}

/// The part of the scroll area below the last entry.
pub(super) fn below_content(scroll: &ScrollAreaOutput<()>) -> egui::Rect {
    let bottom =
        scroll.inner_rect.top() + scroll.content_size.y - scroll.state.unclamped_offset().y;
    egui::Rect::from_min_max(
        egui::pos2(
            scroll.inner_rect.left(),
            bottom.max(scroll.inner_rect.top()),
        ),
        scroll.inner_rect.max,
    )
}

/// Applies the frame's input in a fixed order: selection first, then activation and drag and drop.
pub(super) fn apply(ui: &Ui, explorer: &mut Explorer, out: EntryInput, modifiers: Modifiers) {
    let tab = explorer.active_mut();
    if let Some(i) = out.right_clicked {
        tab.click(i, false, false);
    }
    if let Some(i) = out.select {
        tab.click(i, modifiers.command, modifiers.shift);
    }
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

fn start_drag(ui: &Ui, explorer: &mut Explorer, row: usize) {
    // Dragging an unselected entry drags just that entry, as in other file managers.
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

/// What the entry context menu offers.
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
