//! eframe::App: polls the explorer, maps input to its methods, lays out the views.

use crate::action::{Action, Pane};
use crate::command::{self, Command};
use crate::views::dialogs::{self, Dialog};
use crate::views::toast::{self, Toast};
use crate::{theme, views};
use eframe::egui::{
    self, Context, DragAndDrop, Pos2, ViewportBuilder, ViewportCommand, ViewportId,
};
use fx_app::{
    Explorer, Language, Notice, NoticeKind, Place, SearchView, Settings, State, WindowId, i18n,
    relabel_places,
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
    /// The last title sent to the window manager.
    title: String,
    path_edit: Option<views::toolbar::PathEdit>,
    search: views::search::State,
    /// A reveal asked for its item to be scrolled into view once the folder has loaded.
    reveal_scroll: bool,
    /// The pane the user clicked last; it receives typed characters.
    pane: Pane,
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
            title: String::new(),
            path_edit: None,
            search: views::search::State::default(),
            reveal_scroll: false,
            pane: Pane::default(),
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
            places: Vec::new(),
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
        app.explorer.refresh_places();
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

    fn open_properties(&mut self, index: usize) {
        if self.windows[index].dialog.is_some() {
            return;
        }
        let dialog = Dialog::properties(&mut self.explorer);
        self.open_dialog(index, dialog);
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
        match action {
            Action::OpenCommandPalette => self.open_palette(index),
            Action::NewFolder => self.open_dialog(index, Some(Dialog::new_folder(&self.explorer))),
            Action::Rename => self.open_dialog(index, Dialog::rename(&self.explorer)),
            Action::Delete { permanent } => self.delete_selection(index, permanent),
            Action::DeletePaths(paths) => self.delete(index, paths, false),
            Action::Properties => self.open_properties(index),
            Action::OpenSettings => self.open_settings(ctx, index),
            Action::NewWindow => self.new_window(ctx),
            Action::RenameBookmark(i) => {
                self.open_dialog(index, Dialog::rename_bookmark(&self.explorer, i));
            }
            Action::OpenAbout => self.open_about(ctx),
            Action::Exit => self.exit(ctx),
            Action::FocusPane(pane) => self.windows[index].pane = pane,
            Action::OpenSearch => self.windows[index].search.open(),
            Action::StartSearch(text) => self.start_search(index, text),
            Action::StopSearch => self.explorer.cancel_search(),
            Action::CloseSearch => self.close_search(index),
            Action::EscapeSearch => self.escape_search(index),
            Action::RevealSearchHit(row) => self.reveal_search_hit(index, row),
            Action::SetShowHidden(show) => self.update_settings(|s| s.show_hidden = show),
            Action::SetDensity(density) => self.update_settings(|s| s.density = density),
            action => dispatch_explorer(ctx, &mut self.explorer, action),
        }
    }

    fn delete_selection(&mut self, index: usize, permanent: bool) {
        let paths = selected_paths(&self.explorer);
        self.delete(index, paths, permanent);
    }

    fn open_about(&mut self, ctx: &Context) {
        if self.about {
            views::about::focus(ctx);
        }
        self.about = true;
    }

    fn exit(&mut self, ctx: &Context) {
        self.quitting = true;
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
    }

    fn reveal_search_hit(&mut self, index: usize, row: usize) {
        self.explorer.reveal_search_hit(row);
        self.windows[index].reveal_scroll = true;
    }

    /// Starts the search for `text`; the first one from a filesystem root asks for confirmation.
    fn start_search(&mut self, index: usize, text: String) {
        let ex = &mut self.explorer;
        let fresh = ex.search().is_none() && !text.trim().is_empty();
        if fresh && ex.search_needs_root_notice() {
            self.open_dialog(index, Some(Dialog::search_root(text)));
            return;
        }
        ex.start_search(&text);
    }

    fn close_search(&mut self, index: usize) {
        self.explorer.clear_search();
        self.windows[index].search.close();
    }

    /// Esc: a running search stops and keeps its results; otherwise the search closes.
    fn escape_search(&mut self, index: usize) {
        match self.explorer.search() {
            Some(search) if search.is_running() => self.explorer.cancel_search(),
            Some(_) => self.close_search(index),
            None if self.windows[index].search.is_open() => self.close_search(index),
            None => {}
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
        if ex.search().is_some() && command::needs_folder_listing(command) {
            return false;
        }
        if let Some(action) = command_action(command) {
            actions.push(action);
            return false;
        }
        run_on_explorer(ex, command, actions)
    }

    /// Consumes the bound shortcuts and runs their commands. Returns true if the selection moved.
    fn handle_keys(&mut self, ctx: &Context, index: usize, actions: &mut Vec<Action>) -> bool {
        let typing = ctx.egui_wants_keyboard_input();
        let fired: Vec<Command> = ctx.input_mut(|i| {
            self.keymap
                .bindings
                .iter()
                .filter(|b| may_fire(b, typing))
                .filter(|b| i.consume_key(b.shortcut.modifiers, b.shortcut.key))
                .map(|b| b.command)
                .collect()
        });
        let mut moved = false;
        for command in fired {
            moved |= self.run(command, actions);
        }
        if !typing && self.explorer.search().is_none() {
            moved |= self.type_ahead(ctx, index);
        }
        moved
    }

    /// Forwards typed characters to the focused pane. Returns true if the listing selection moved.
    fn type_ahead(&mut self, ctx: &Context, index: usize) -> bool {
        let typed: Vec<String> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(text) => Some(text.clone()),
                    _ => None,
                })
                .collect()
        });
        let now = std::time::Instant::now();
        let mut moved = false;
        for text in typed {
            moved |= match self.windows[index].pane {
                Pane::Listing => self.explorer.active_mut().type_ahead(&text, now),
                Pane::Tree => {
                    let roots = views::sidebar::roots(&self.explorer, &self.places);
                    self.explorer.tree_type_ahead(&text, &roots, now);
                    false
                }
            };
        }
        moved
    }
}

/// While a text field has the keyboard only bindings that opt in and carry Ctrl or Alt may fire.
fn may_fire(binding: &command::Binding, typing: bool) -> bool {
    let m = binding.shortcut.modifiers;
    !typing || (binding.command.def().while_typing && (m.ctrl || m.alt))
}

/// The action a command raises, for commands that act through the view-model of the window.
fn command_action(command: Command) -> Option<Action> {
    Some(match command {
        Command::Search => Action::OpenSearch,
        Command::CloseSearch => Action::EscapeSearch,
        Command::Rename => Action::Rename,
        Command::Delete => Action::Delete { permanent: false },
        Command::DeletePermanent => Action::Delete { permanent: true },
        Command::SelectAll => Action::SelectAll,
        Command::Copy => Action::Copy,
        Command::Cut => Action::Cut,
        Command::Paste => Action::Paste,
        Command::NewFolder => Action::NewFolder,
        Command::Properties => Action::Properties,
        Command::NewWindow => Action::NewWindow,
        Command::OpenSettings => Action::OpenSettings,
        Command::Palette => Action::OpenCommandPalette,
        Command::About => Action::OpenAbout,
        Command::Exit => Action::Exit,
        _ => return None,
    })
}

/// Runs the commands that act on the explorer directly. Returns true if the selection moved.
fn run_on_explorer(ex: &mut Explorer, command: Command, actions: &mut Vec<Action>) -> bool {
    match command {
        Command::NewTab => ex.new_tab(ex.active().path().to_path_buf()),
        Command::CloseTab => ex.close_tab(ex.active_index()),
        Command::NextTab => cycle_tab(ex, 1),
        Command::PrevTab => cycle_tab(ex, -1),
        Command::Refresh => ex.refresh(),
        Command::Back => ex.back(),
        Command::Forward => ex.forward(),
        Command::Up => ex.up(),
        Command::ToggleBookmark => ex.toggle_bookmark_active(),
        Command::MoveDown => return move_selection(ex, 1),
        Command::MoveUp => return move_selection(ex, -1),
        Command::OpenSelected => open_selected(ex, actions),
        Command::RevealSearchHit => {
            let row = ex.search().and_then(SearchView::selected);
            actions.extend(row.map(Action::RevealSearchHit));
        }
        _ => {}
    }
    false
}

fn cycle_tab(ex: &mut Explorer, step: isize) {
    let n = ex.tabs().len() as isize;
    if n == 0 {
        return;
    }
    ex.set_active((ex.active_index() as isize + step).rem_euclid(n) as usize);
}

/// Executes the actions that only touch the explorer.
fn dispatch_explorer(ctx: &Context, ex: &mut Explorer, action: Action) {
    match action {
        Action::SelectAll => ex.active_mut().select_all(),
        Action::Refresh => ex.refresh(),
        Action::Open(row) => ex.activate(row),
        Action::OpenInNewTab(row) => ex.open_in_new_tab(row),
        Action::ToggleBookmark(row) => toggle_bookmark(ex, row),
        Action::OpenWith(row) => ex.open_with(row),
        Action::CopyPath => copy_paths(ctx, ex),
        Action::Copy => ex.copy_to_clipboard(&ex.active().selection(), false),
        Action::Cut => ex.copy_to_clipboard(&ex.active().selection(), true),
        Action::Paste => ex.paste(None),
        Action::PasteInto(row) => {
            let folder = ex.entry_path(row);
            ex.paste(folder);
        }
        Action::OpenSearchHit(row) => ex.open_search_hit(row),
        Action::CopySearchHitPath(row) => copy_search_hit_path(ctx, ex, row),
        Action::SetSort(sort) => ex.active_mut().set_sort(sort),
        _ => {}
    }
}

fn toggle_bookmark(ex: &mut Explorer, row: usize) {
    if let Some(path) = ex.entry_path(row) {
        ex.toggle_bookmark(path);
    }
}

fn selected_paths(ex: &Explorer) -> Vec<PathBuf> {
    ex.active()
        .selection()
        .into_iter()
        .filter_map(|row| ex.entry_path(row))
        .collect()
}

/// Moves the selection of the search results when a search shows, else of the folder listing. Returns true.
fn move_selection(ex: &mut Explorer, delta: isize) -> bool {
    match ex.search() {
        Some(_) => ex.move_search_selection(delta),
        None => ex.active_mut().move_selection(delta),
    }
    true
}

fn open_selected(ex: &mut Explorer, actions: &mut Vec<Action>) {
    if let Some(search) = ex.search() {
        actions.extend(search.selected().map(Action::OpenSearchHit));
    } else if let Some(row) = ex.active().selected() {
        ex.activate(row);
    }
}

fn copy_search_hit_path(ctx: &Context, ex: &Explorer, row: usize) {
    let hit = ex.search().and_then(|s| s.hits().get(row));
    if let Some(hit) = hit {
        ctx.copy_text(hit.path.display().to_string());
    }
}

/// Puts the paths of the selected entries on the clipboard, one per line.
fn copy_paths(ctx: &Context, ex: &Explorer) {
    let paths: Vec<String> = selected_paths(ex)
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    if !paths.is_empty() {
        ctx.copy_text(paths.join("\n"));
    }
}

/// Raises a focus change to `pane` when the user presses the primary button over its panel.
fn focus_on_press(ui: &egui::Ui, panel: &egui::Response, pane: Pane, actions: &mut Vec<Action>) {
    if ui.input(|i| i.pointer.primary_pressed()) && panel.contains_pointer() {
        actions.push(Action::FocusPane(pane));
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
    /// The root window is being closed, not quit, while another window can take its place.
    fn root_close_hands_over(&self, ctx: &Context) -> bool {
        self.windows.len() > 1 && !self.quitting && ctx.input(|i| i.viewport().close_requested())
    }

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
        dnd::track_always(&ctx, self.windows.len() > 1);

        self.handle_input(&ctx, index);
        self.update_title(&ctx, index);

        let mut actions = Vec::new();
        self.panels(ui, index, &mut actions);
        self.overlays(&ctx, index);
        self.dispatch_all(&ctx, index, actions);
        self.drag_and_drop(&ctx, index);
    }

    fn update_title(&mut self, ctx: &Context, index: usize) {
        let title = format!("fx — {}", self.explorer.active().path().display());
        if self.windows[index].title == title {
            return;
        }
        ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
        self.windows[index].title = title;
    }

    fn dispatch_all(&mut self, ctx: &Context, index: usize, actions: Vec<Action>) {
        for action in actions {
            self.dispatch(ctx, index, action);
        }
    }

    /// Runs the palette and the shortcuts of window `index`, then the actions they raised.
    fn handle_input(&mut self, ctx: &Context, index: usize) {
        let mut actions = Vec::new();
        let palette_moved = self.palette_frame(ctx, index, &mut actions);
        let keys_moved = self.is_idle(index) && self.handle_keys(ctx, index, &mut actions);
        self.windows[index].scroll_to_selection = palette_moved || keys_moved;
        self.dispatch_all(ctx, index, actions);
        self.windows[index].scroll_to_selection |= self.take_reveal_scroll(index);
    }

    /// No dialog, settings or palette takes the keyboard from window `index`.
    fn is_idle(&self, index: usize) -> bool {
        let window = &self.windows[index];
        window.dialog.is_none() && window.palette.is_none() && self.settings.is_none()
    }

    fn drag_and_drop(&mut self, ctx: &Context, index: usize) {
        self.highlight_drop_target(ctx, index);
        self.drop_os_files(ctx, index);
        self.hand_drag_to_os(ctx, index);
        self.finish_drag(ctx, index);
    }

    /// True once, when a reveal's folder has loaded and its item is selected.
    fn take_reveal_scroll(&mut self, index: usize) -> bool {
        let ready =
            self.windows[index].reveal_scroll && self.explorer.active().selected().is_some();
        if ready {
            self.windows[index].reveal_scroll = false;
        }
        ready
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
        let area = if dnd::os_hovering(ctx) {
            ctx.request_repaint();
            self.external_area(ctx, index)
        } else {
            self.internal_drop_area(ctx, index)
        };
        if let Some(area) = area {
            dnd::highlight(ctx, area.rect);
        }
    }

    /// The area a drag from another window would land on, when that lies in window `index`.
    fn internal_drop_area(&self, ctx: &Context, index: usize) -> Option<DropArea> {
        let dragged = DragAndDrop::payload::<Dragged>(ctx)?;
        let source = self.windows.iter().position(|w| w.id == dragged.from)?;
        if source == index {
            return None;
        }
        self.drop_target(ctx, source, &dragged.paths)
            .filter(|target| target.viewport == self.viewport_at(index))
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
        self.windows[index].search.sync(&self.explorer);
        if self.windows[index].search.is_open() {
            egui::Panel::top("search").show(ui, |ui| {
                let state = &mut self.windows[index].search;
                views::search::bar(ui, &mut self.explorer, state, actions);
            });
        }
        egui::Panel::top("commands").show(ui, |ui| {
            views::commandbar::show(ui, &self.explorer, actions);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            views::status::show(ui, &self.explorer);
        });
        if self.explorer.has_transfers() {
            egui::Panel::bottom("transfers").show(ui, |ui| {
                let state = &mut self.windows[index].transfers;
                views::transfers::show(ui, &mut self.explorer, state);
            });
        }
        let tree = egui::Panel::left("places")
            .default_size(240.0)
            .show(ui, |ui| {
                views::sidebar::show(ui, &mut self.explorer, &self.places, actions);
            });
        focus_on_press(ui, &tree.response, Pane::Tree, actions);
        let listing = egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::bg()).inner_margin(4))
            .show(ui, |ui| {
                if self.explorer.search().is_some() {
                    let state = &mut self.windows[index].search;
                    views::search::results(
                        ui,
                        &mut self.explorer,
                        state,
                        scroll_to_selection,
                        actions,
                    );
                } else {
                    views::table::show(ui, &mut self.explorer, scroll_to_selection, actions);
                }
            });
        focus_on_press(ui, &listing.response, Pane::Listing, actions);
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
            log::info!("language changed to {:?}", current.4);
            self.apply_language(current.4);
        }
        if current.0 != self.applied_look.0 {
            log::info!("theme changed to {}", current.0);
        }
        theme::apply(ctx, self.explorer.settings());
        self.applied_look = current;
    }

    fn apply_language(&mut self, language: Language) {
        i18n::set_language(language);
        relabel_places(&mut self.places);
    }
}

impl eframe::App for App {
    fn on_exit(&mut self) {
        self.explorer.save_session();
    }

    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.explorer.poll();
        if let Some(mut places) = self.explorer.take_places() {
            relabel_places(&mut places);
            self.places = places;
        }
        // Keep polling while idle so background revalidation results show up.
        ctx.request_repaint_after(fx_app::REVALIDATE_INTERVAL);
        if self.explorer.transfers_active() {
            // Progress is sampled, not pushed: 10 Hz is smooth and costs nothing.
            ctx.request_repaint_after(TRANSFER_REFRESH);
        }
        if self.root_close_hands_over(ctx) {
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
