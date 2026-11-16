//! eframe::App: polls the explorer, maps input to its methods, lays out the views.

use crate::action::Action;
use crate::command::{self, Command};
use crate::views::dialogs::{self, Dialog};
use crate::views::toast::{self, Toast};
use crate::{theme, views};
use eframe::egui::{
    self, Context, DragAndDrop, Pos2, ViewportBuilder, ViewportCommand, ViewportId,
};
use fx_app::{
    Explorer, Language, Notice, NoticeKind, Place, Settings, State, WindowId, i18n, places,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use views::dnd::{self, Dragged, DropArea};

const TRANSFER_REFRESH: std::time::Duration = std::time::Duration::from_millis(100);

/// UI state of one explorer window; the first one lives in the root viewport.
struct WindowUi {
    id: WindowId,
    dialog: Option<Dialog>,
    palette: Option<views::palette::State>,
    scroll_to_selection: bool,
    transfers: views::transfers::State,
    was_focused: bool,
    path_edit: Option<views::toolbar::PathEdit>,
}

impl WindowUi {
    fn new(id: WindowId, was_focused: bool) -> Self {
        Self {
            id,
            dialog: None,
            palette: None,
            scroll_to_selection: false,
            transfers: views::transfers::State::default(),
            was_focused,
            path_edit: None,
        }
    }
}

pub struct App {
    explorer: Explorer,
    places: Vec<Place>,
    windows: Vec<WindowUi>,
    /// The window that last had focus; it shows the toast and transfer conflicts.
    focused: WindowId,
    /// Set by Exit so closing the root window quits instead of promoting another window.
    quitting: bool,
    /// The settings window, shown natively while open.
    settings: Option<Box<views::settings::State>>,
    /// Whether the about window is open.
    about: bool,
    toast: Option<Toast>,
    applied_look: Look,
    keymap: command::Keymap,
    /// The `[keymap]` overrides `keymap` was resolved from.
    applied_keymap: BTreeMap<String, Vec<String>>,
}

/// What `theme::apply` and the translated texts depend on; a change re-applies them.
type Look = (String, Option<String>, f32, f32, Language);

fn look(s: &Settings) -> Look {
    (
        s.theme.clone(),
        s.font_family.clone(),
        s.list_font_size,
        s.tree_font_size,
        s.language,
    )
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        start: Option<PathBuf>,
        fallback: PathBuf,
        settings: Settings,
        state: State,
    ) -> Self {
        i18n::set_language(settings.language);
        fx_app::set_date_format(settings.date_format);
        let applied_look = look(&settings);
        theme::apply(&cc.egui_ctx, &settings);
        let ctx = cc.egui_ctx.clone();
        let explorer =
            Explorer::with_state(start, fallback, settings, state, State::path(), move || {
                ctx.request_repaint()
            });
        let ids = explorer.window_ids();
        let first = ids[0];
        let windows = ids
            .into_iter()
            .enumerate()
            .map(|(i, id)| WindowUi::new(id, i == 0))
            .collect();
        let mut app = Self {
            explorer,
            places: places(),
            windows,
            focused: first,
            quitting: false,
            settings: None,
            about: false,
            toast: None,
            applied_look,
            keymap: command::Keymap::default(),
            applied_keymap: BTreeMap::new(),
        };
        app.apply_keymap();
        app
    }

    fn update_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        let mut settings = self.explorer.settings().clone();
        change(&mut settings);
        self.explorer.set_settings(settings);
    }

    /// Opens `dialog` in window `index` unless one is already showing there.
    fn open_dialog(&mut self, index: usize, dialog: Option<Dialog>) {
        let slot = &mut self.windows[index].dialog;
        if slot.is_none() {
            *slot = dialog;
        }
    }

    /// Asks first when the settings or `permanent` require it, otherwise deletes right away.
    fn delete(&mut self, index: usize, paths: Vec<PathBuf>, permanent: bool) {
        let settings = self.explorer.settings();
        let permanent = permanent || !settings.use_trash;
        if permanent || settings.confirm_delete {
            let dialog = Dialog::delete_paths(paths, permanent);
            self.open_dialog(index, dialog);
        } else {
            self.explorer.delete_paths(paths, false);
        }
    }

    fn dispatch(&mut self, ctx: &Context, index: usize, action: Action) {
        let ex = &mut self.explorer;
        match action {
            Action::OpenCommandPalette => self.open_palette(index),
            Action::NewFolder => {
                let dialog = Dialog::new_folder(ex);
                self.open_dialog(index, Some(dialog));
            }
            Action::Rename => {
                let dialog = Dialog::rename(ex);
                self.open_dialog(index, dialog);
            }
            Action::Delete { permanent } => {
                let paths = ex
                    .active()
                    .selection()
                    .iter()
                    .filter_map(|&row| ex.entry_path(row))
                    .collect();
                self.delete(index, paths, permanent);
            }
            Action::DeletePaths(paths) => self.delete(index, paths, false),
            Action::Properties => {
                let dialog = Dialog::properties(ex);
                self.open_dialog(index, dialog);
            }
            Action::OpenSettings => self.open_settings(ctx, index),
            Action::NewWindow => self.new_window(ctx),
            Action::RenameBookmark(i) => {
                let dialog = Dialog::rename_bookmark(ex, i);
                self.open_dialog(index, dialog);
            }
            Action::OpenAbout => {
                if self.about {
                    views::about::focus(ctx);
                }
                self.about = true;
            }
            Action::Exit => {
                self.quitting = true;
                ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
            }
            Action::SelectAll => ex.active_mut().select_all(),
            Action::Refresh => ex.refresh(),
            Action::Open(row) => ex.activate(row),
            Action::OpenInNewTab(row) => ex.open_in_new_tab(row),
            Action::ToggleBookmark(row) => {
                if let Some(path) = ex.entry_path(row) {
                    ex.toggle_bookmark(path);
                }
            }
            #[cfg(windows)]
            Action::OpenWith(row) => {
                if let Some(path) = ex.entry_path(row) {
                    fx_app::open_with_chooser(&path);
                }
            }
            Action::CopyPath => copy_paths(ctx, ex),
            Action::Copy => ex.copy_to_clipboard(&ex.active().selection(), false),
            Action::Cut => ex.copy_to_clipboard(&ex.active().selection(), true),
            Action::Paste => ex.paste(None),
            Action::PasteInto(row) => {
                let folder = ex.entry_path(row);
                ex.paste(folder);
            }
            Action::SetSort(sort) => ex.active_mut().set_sort(sort),
            Action::SetShowHidden(show) => self.update_settings(|s| s.show_hidden = show),
            Action::SetDensity(density) => self.update_settings(|s| s.density = density),
        }
    }

    /// Opens the command palette in window `index` unless a dialog or the settings are showing.
    fn open_palette(&mut self, index: usize) {
        if self.settings.is_none() && self.windows[index].dialog.is_none() {
            self.windows[index].palette = Some(views::palette::State::new());
        }
    }

    /// Shows the palette of window `index` and runs the command the user picked. Returns true if the selection moved.
    fn palette_frame(&mut self, ctx: &Context, index: usize, actions: &mut Vec<Action>) -> bool {
        let Some(state) = self.windows[index].palette.as_mut() else {
            return false;
        };
        match views::palette::show(ctx, state, &self.explorer) {
            views::palette::Outcome::Pending => false,
            views::palette::Outcome::Close => {
                self.windows[index].palette = None;
                false
            }
            views::palette::Outcome::Run(command) => {
                self.windows[index].palette = None;
                self.run(command, actions)
            }
        }
    }

    /// Focuses the settings window, or opens it unless window `index` shows a dialog.
    fn open_settings(&mut self, ctx: &Context, index: usize) {
        if self.settings.is_some() {
            views::settings::focus(ctx);
        } else if self.windows[index].dialog.is_none() {
            let state = views::settings::State::new(self.explorer.settings());
            self.settings = Some(Box::new(state));
        }
    }

    /// Opens the active folder in a new window.
    fn new_window(&mut self, ctx: &Context) {
        let path = self.explorer.active().path().to_path_buf();
        if ctx.embed_viewports() {
            // No native multi-window support: a tab is the closest thing.
            self.explorer.new_tab(path);
            return;
        }
        let id = self.explorer.open_window(path);
        self.windows.push(WindowUi::new(id, false));
    }

    /// Rebuilds the resolved keymap and the shortcut hints. Entries that cannot be used raise a notice.
    fn apply_keymap(&mut self) {
        let overrides = &self.explorer.settings().keymap;
        let (keymap, problems) = command::resolve(overrides);
        command::install_hints(overrides);
        self.keymap = keymap;
        self.applied_keymap = overrides.clone();
        if !problems.is_empty() {
            let text = fx_app::tr("notice.keymap_ignored", &[("entry", &problems.join(", "))]);
            self.toast = Some(Toast::new(Notice {
                kind: NoticeKind::Error,
                text,
            }));
        }
    }

    /// Runs `command`. Returns true if the selection moved.
    fn run(&mut self, command: Command, actions: &mut Vec<Action>) -> bool {
        let ex = &mut self.explorer;
        match command {
            Command::NewTab => ex.new_tab(ex.active().path().to_path_buf()),
            Command::CloseTab => ex.close_tab(ex.active_index()),
            Command::NextTab => ex.set_active((ex.active_index() + 1) % ex.tabs().len()),
            Command::PrevTab => {
                let n = ex.tabs().len();
                ex.set_active((ex.active_index() + n - 1) % n);
            }
            Command::Refresh => ex.refresh(),
            Command::Back => ex.back(),
            Command::Forward => ex.forward(),
            Command::Up => ex.up(),
            Command::ToggleBookmark => ex.toggle_bookmark_active(),
            Command::MoveDown => {
                ex.active_mut().move_selection(1);
                return true;
            }
            Command::MoveUp => {
                ex.active_mut().move_selection(-1);
                return true;
            }
            Command::OpenSelected => {
                if let Some(row) = ex.active().selected() {
                    ex.activate(row);
                }
            }
            Command::Rename => actions.push(Action::Rename),
            Command::Delete => actions.push(Action::Delete { permanent: false }),
            Command::DeletePermanent => actions.push(Action::Delete { permanent: true }),
            Command::SelectAll => actions.push(Action::SelectAll),
            Command::Copy => actions.push(Action::Copy),
            Command::Cut => actions.push(Action::Cut),
            Command::Paste => actions.push(Action::Paste),
            Command::NewFolder => actions.push(Action::NewFolder),
            Command::Properties => actions.push(Action::Properties),
            Command::NewWindow => actions.push(Action::NewWindow),
            Command::OpenSettings => actions.push(Action::OpenSettings),
            Command::Palette => actions.push(Action::OpenCommandPalette),
            Command::About => actions.push(Action::OpenAbout),
            Command::Exit => actions.push(Action::Exit),
        }
        false
    }

    /// Consumes the bound shortcuts and runs their commands. Returns true if the selection moved.
    fn handle_keys(&mut self, ctx: &Context, actions: &mut Vec<Action>) -> bool {
        let typing = ctx.egui_wants_keyboard_input();
        let fired: Vec<Command> = ctx.input_mut(|i| {
            self.keymap
                .bindings
                .iter()
                .filter(|b| {
                    let m = b.shortcut.modifiers;
                    !typing || (b.command.def().while_typing && (m.ctrl || m.alt))
                })
                .filter(|b| i.consume_key(b.shortcut.modifiers, b.shortcut.key))
                .map(|b| b.command)
                .collect()
        });
        let mut moved = false;
        for command in fired {
            moved |= self.run(command, actions);
        }
        moved
    }
}

/// Puts the paths of the selected entries on the clipboard, one per line.
fn copy_paths(ctx: &Context, ex: &Explorer) {
    let paths: Vec<String> = ex
        .active()
        .selection()
        .into_iter()
        .filter_map(|row| ex.entry_path(row))
        .map(|path| path.display().to_string())
        .collect();
    if !paths.is_empty() {
        ctx.copy_text(paths.join("\n"));
    }
}

/// Covers the main window while settings is open; true when the user clicked it.
fn block_input(ctx: &Context) -> bool {
    let rect = ctx.content_rect();
    egui::Area::new(egui::Id::unique("settings-blocker"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .show(ctx, |ui| {
            ui.allocate_exact_size(rect.size(), egui::Sense::click()).1
        })
        .inner
        .clicked()
}

/// The viewport an extra window is shown in.
fn viewport_of(id: WindowId) -> ViewportId {
    ViewportId::from_hash_of(("window", id))
}

impl App {
    /// Handles the root window being closed while other windows exist: the next window takes its place.
    fn promote_next_window(&mut self, ctx: &Context) {
        let next = viewport_of(self.windows[1].id);
        let (outer, inner) =
            ctx.input_for(next, |i| (i.viewport().outer_rect, i.viewport().inner_rect));
        ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        let gone = self.windows.remove(0);
        self.explorer.close_window(gone.id);
        if let Some(outer) = outer {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(outer.min));
        }
        if let Some(inner) = inner {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(inner.size()));
        }
    }

    /// Draws and drives window `index`; `ui` belongs to the viewport that window is shown in.
    fn window_frame(&mut self, ui: &mut egui::Ui, index: usize) {
        let ctx = ui.ctx().clone();
        self.explorer.set_window(self.windows[index].id);
        self.track_focus(&ctx, index);
        dnd::forget_areas(&ctx);

        let mut actions = Vec::new();
        let palette_moved = self.palette_frame(&ctx, index, &mut actions);
        let idle = self.windows[index].dialog.is_none()
            && self.settings.is_none()
            && self.windows[index].palette.is_none();
        let keys_moved = idle && self.handle_keys(&ctx, &mut actions);
        self.windows[index].scroll_to_selection = palette_moved || keys_moved;
        for action in actions.drain(..) {
            self.dispatch(&ctx, index, action);
        }
        ctx.send_viewport_cmd(ViewportCommand::Title(format!(
            "fx — {}",
            self.explorer.active().path().display()
        )));

        self.panels(ui, index, &mut actions);
        self.overlays(&ctx, index);
        for action in actions {
            self.dispatch(&ctx, index, action);
        }
        self.highlight_drop_target(&ctx, index);
        self.drop_os_files(&ctx, index);
        self.hand_drag_to_os(&ctx, index);
        self.finish_drag(&ctx, index);
    }

    /// The viewport window `index` is shown in; the first window lives in the root viewport.
    fn viewport_at(&self, index: usize) -> ViewportId {
        if index == 0 {
            ViewportId::ROOT
        } else {
            viewport_of(self.windows[index].id)
        }
    }

    /// Where a drag from window `source` lands: a tree folder in any window, else a window's listing.
    fn drop_target(&self, ctx: &Context, source: usize, sources: &[PathBuf]) -> Option<DropArea> {
        let pointer = dnd::global_pointer(ctx, self.viewport_at(source))?;
        self.area_at(ctx, pointer, Some(self.viewport_at(source)), sources)
    }

    fn area_at(
        &self,
        ctx: &Context,
        pointer: Pos2,
        source: Option<ViewportId>,
        sources: &[PathBuf],
    ) -> Option<DropArea> {
        dnd::drop_target(
            pointer,
            &dnd::tree_folders(ctx),
            &self.window_areas(ctx),
            source,
            sources,
        )
    }

    /// Where files from outside fx land when they hover or drop on window `index`: the tree folder under
    /// the cursor if there is one, else the window's active folder.
    fn external_area(&self, ctx: &Context, index: usize) -> Option<DropArea> {
        let own = self.viewport_at(index);
        dnd::screen_pointer(ctx)
            .and_then(|pointer| self.area_at(ctx, pointer, None, &[]))
            .filter(|area| area.viewport == own)
            .or_else(|| {
                self.window_areas(ctx)
                    .into_iter()
                    .find(|area| area.viewport == own)
            })
    }

    /// Each window's active folder over its global content rectangle.
    fn window_areas(&self, ctx: &Context) -> Vec<DropArea> {
        (0..self.windows.len())
            .filter_map(|i| {
                let viewport = self.viewport_at(i);
                let rect = dnd::content_rect(ctx, viewport)?;
                let folder = self.explorer.window_folder(self.windows[i].id)?;
                Some(DropArea {
                    viewport,
                    folder,
                    rect,
                })
            })
            .collect()
    }

    /// Outlines what a drag would land on, while it hovers window `index`.
    fn highlight_drop_target(&self, ctx: &Context, index: usize) {
        if dnd::os_hovering(ctx) {
            ctx.request_repaint();
            if let Some(area) = self.external_area(ctx, index) {
                dnd::highlight(ctx, area.rect);
            }
            return;
        }
        let Some(dragged) = DragAndDrop::payload::<Dragged>(ctx) else {
            return;
        };
        let Some(source) = self.windows.iter().position(|w| w.id == dragged.from) else {
            return;
        };
        if source == index {
            return;
        }
        if let Some(target) = self.drop_target(ctx, source, &dragged.paths)
            && target.viewport == self.viewport_at(index)
        {
            dnd::highlight(ctx, target.rect);
        }
    }

    /// Copies files dropped from outside fx into the tree folder or listing they landed on.
    fn drop_os_files(&mut self, ctx: &Context, index: usize) {
        let paths = dnd::os_dropped_paths(ctx);
        if paths.is_empty() {
            return;
        }
        if let Some(area) = self.external_area(ctx, index) {
            self.explorer.drop_into(paths, &area.folder, true);
        }
    }

    /// Passes a drag that started in window `index` to the operating system once the pointer
    /// leaves every fx window, so other programs can receive the files.
    fn hand_drag_to_os(&self, ctx: &Context, index: usize) {
        let Some(dragged) = DragAndDrop::payload::<Dragged>(ctx) else {
            return;
        };
        if dragged.from != self.windows[index].id {
            return;
        }
        let Some(pointer) = dnd::screen_pointer(ctx) else {
            return;
        };
        if self
            .window_areas(ctx)
            .iter()
            .any(|area| area.rect.contains(pointer))
        {
            return;
        }
        DragAndDrop::clear_payload(ctx);
        dnd::start_external(dragged.paths.clone());
    }

    /// Ends a drag that started in window `index` once its button is released.
    /// Released over a tree folder or a listing of another window, the paths are copied or moved there.
    fn finish_drag(&mut self, ctx: &Context, index: usize) {
        let Some(dragged) = DragAndDrop::payload::<Dragged>(ctx) else {
            return;
        };
        if dragged.from != self.windows[index].id || !dnd::released(ctx, self.viewport_at(index)) {
            return;
        }
        if let Some(target) = self.drop_target(ctx, index, &dragged.paths) {
            let copy = dnd::copying(ctx, self.explorer.settings().drag_copy_modifier);
            self.explorer
                .drop_into(dragged.paths.clone(), &target.folder, copy);
        }
        DragAndDrop::clear_payload(ctx);
    }

    /// When window `index` gains focus, revalidates its folder and bookmarks and makes it the focused window.
    fn track_focus(&mut self, ctx: &Context, index: usize) {
        let focused = ctx.input(|i| i.focused);
        if focused && !self.windows[index].was_focused {
            self.explorer.revalidate();
            self.explorer.check_bookmarks();
            self.focused = self.windows[index].id;
        }
        self.windows[index].was_focused = focused;
    }

    /// The panels of window `index`: bars on top, status and transfers at the bottom, places and the file table.
    fn panels(&mut self, ui: &mut egui::Ui, index: usize, actions: &mut Vec<Action>) {
        let scroll_to_selection = self.windows[index].scroll_to_selection;
        egui::Panel::top("menubar").show(ui, |ui| {
            views::menubar::show(ui, &self.explorer, actions);
        });
        egui::Panel::top("tabs").show(ui, |ui| {
            views::tabs::show(ui, &mut self.explorer);
        });
        egui::Panel::top("toolbar").show(ui, |ui| {
            views::toolbar::show(ui, &mut self.explorer, &mut self.windows[index].path_edit);
        });
        egui::Panel::top("commands").show(ui, |ui| {
            views::commandbar::show(ui, &self.explorer, actions);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            views::status::show(ui, &self.explorer);
        });
        if !self.explorer.transfers().is_empty() {
            egui::Panel::bottom("transfers").show(ui, |ui| {
                let state = &mut self.windows[index].transfers;
                views::transfers::show(ui, &mut self.explorer, state);
            });
        }
        egui::Panel::left("places")
            .default_size(240.0)
            .show(ui, |ui| {
                views::sidebar::show(ui, &mut self.explorer, &self.places, actions);
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::bg()).inner_margin(4))
            .show(ui, |ui| {
                views::table::show(ui, &mut self.explorer, scroll_to_selection, actions);
            });
    }

    /// What floats over window `index`: drag feedback, its dialog, the settings blocker and the toast.
    fn overlays(&mut self, ctx: &Context, index: usize) {
        views::dnd::feedback(ctx, self.explorer.settings().drag_copy_modifier);
        if let Some(dialog) = &mut self.windows[index].dialog
            && dialogs::show(ctx, dialog, &mut self.explorer)
        {
            self.windows[index].dialog = None;
        }
        if self.settings.as_ref().is_some_and(|s| !s.embedded()) && block_input(ctx) {
            views::settings::focus(ctx);
        }
        if self.focused == self.windows[index].id
            && let Some(current) = &self.toast
            && !toast::show(ctx, current)
        {
            self.toast = None;
        }
    }

    /// Shows the extra windows in their own viewports. Returns the ones the user closed.
    fn show_extra_windows(&mut self, ctx: &Context) -> Vec<WindowId> {
        let mut closed = Vec::new();
        for index in 1..self.windows.len() {
            if self.show_extra_window(ctx, index) {
                closed.push(self.windows[index].id);
            }
        }
        closed
    }

    /// Shows window `index` in its viewport. Returns true when the user closed it.
    fn show_extra_window(&mut self, ctx: &Context, index: usize) -> bool {
        let builder = ViewportBuilder::default()
            .with_title("fx")
            .with_inner_size([1200.0, 750.0]);
        let viewport = viewport_of(self.windows[index].id);
        ctx.show_viewport_immediate(viewport, builder, |ui, _class| {
            self.window_frame(ui, index);
            ui.ctx().input(|i| i.viewport().close_requested())
        })
    }

    /// Falls back to the first window when the focused one is gone.
    fn keep_focus_valid(&mut self) {
        if !self.windows.iter().any(|w| w.id == self.focused) {
            self.focused = self.windows[0].id;
        }
    }

    /// Gives the focused window the pending transfer conflict, unless it shows a dialog already.
    fn offer_conflict(&mut self) {
        if let Some(window) = self.windows.iter_mut().find(|w| w.id == self.focused)
            && window.dialog.is_none()
        {
            window.dialog = Dialog::conflict(&self.explorer);
        }
    }

    /// Re-applies what depends on changed settings: date format, keymap, language, theme and fonts.
    fn apply_settings(&mut self, ctx: &Context) {
        fx_app::set_date_format(self.explorer.settings().date_format);
        if self.explorer.settings().keymap != self.applied_keymap {
            self.apply_keymap();
        }
        let current = look(self.explorer.settings());
        if current == self.applied_look {
            return;
        }
        if current.4 != self.applied_look.4 {
            i18n::set_language(current.4);
            // Place labels are translated when listed.
            self.places = places();
        }
        theme::apply(ctx, self.explorer.settings());
        self.applied_look = current;
    }
}

impl eframe::App for App {
    fn on_exit(&mut self) {
        self.explorer.save_session();
    }

    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.explorer.poll();
        // Keep polling while idle so background revalidation results show up.
        ctx.request_repaint_after(fx_app::REVALIDATE_INTERVAL);
        if self.explorer.transfers_active() {
            // Progress is sampled, not pushed: 10 Hz is smooth and costs nothing.
            ctx.request_repaint_after(TRANSFER_REFRESH);
        }
        if self.windows.len() > 1 && !self.quitting && ctx.input(|i| i.viewport().close_requested())
        {
            self.promote_next_window(ctx);
        }
        self.keep_focus_valid();
        self.explorer.set_window(self.focused);
        self.offer_conflict();
        self.apply_settings(ctx);
        if self.toast.is_none() {
            self.toast = self.explorer.take_notice().map(Toast::new);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.window_frame(ui, 0);

        for id in self.show_extra_windows(&ctx) {
            self.windows.retain(|w| w.id != id);
            self.explorer.close_window(id);
        }
        self.keep_focus_valid();

        // Settings acts on the folder of the window that was focused last.
        self.explorer.set_window(self.focused);
        if let Some(state) = &mut self.settings
            && views::settings::show(&ctx, &mut self.explorer, state)
        {
            self.settings = None;
        }
        if self.about && views::about::show(&ctx) {
            self.about = false;
        }
    }
}
