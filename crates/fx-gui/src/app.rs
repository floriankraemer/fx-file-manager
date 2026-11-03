//! eframe::App: polls the explorer, maps input to its methods, lays out the views.

use crate::action::Action;
use crate::views::dialogs::{self, Dialog};
use crate::views::toast::{self, Toast};
use crate::{theme, views};
use eframe::egui::{self, Context, Key, Modifiers};
use fx_app::{Explorer, Place, Settings, places};
use std::path::PathBuf;

pub struct App {
    explorer: Explorer,
    places: Vec<Place>,
    scroll_to_selection: bool,
    dialog: Option<Dialog>,
    toast: Option<Toast>,
    applied_theme: fx_app::ThemeChoice,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, start: PathBuf, settings: Settings) -> Self {
        let applied_theme = settings.theme;
        theme::apply(&cc.egui_ctx, applied_theme);
        let ctx = cc.egui_ctx.clone();
        Self {
            explorer: Explorer::with_settings(start, settings, move || ctx.request_repaint()),
            places: places(),
            scroll_to_selection: false,
            dialog: None,
            toast: None,
            applied_theme,
        }
    }

    fn update_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        let mut settings = self.explorer.settings().clone();
        change(&mut settings);
        self.explorer.set_settings(settings);
    }

    /// Opens `dialog` unless one is already showing.
    fn open_dialog(&mut self, dialog: Option<Dialog>) {
        if self.dialog.is_none() {
            self.dialog = dialog;
        }
    }

    fn dispatch(&mut self, ctx: &Context, action: Action) {
        let ex = &mut self.explorer;
        match action {
            Action::NewFolder => {
                let dialog = Dialog::new_folder(ex);
                self.open_dialog(Some(dialog));
            }
            Action::Rename => {
                let dialog = Dialog::rename(ex);
                self.open_dialog(dialog);
            }
            Action::Delete { permanent } => {
                let settings = ex.settings();
                let permanent = permanent || !settings.use_trash;
                if permanent || settings.confirm_delete {
                    let dialog = Dialog::delete(ex, permanent);
                    self.open_dialog(dialog);
                } else {
                    let rows = ex.active().selection();
                    ex.delete(&rows, false);
                }
            }
            Action::Properties => {
                let dialog = Dialog::properties(ex);
                self.open_dialog(dialog);
            }
            Action::OpenSettings => {
                let dialog = Dialog::settings(ex);
                self.open_dialog(Some(dialog));
            }
            Action::SelectAll => ex.active_mut().select_all(),
            Action::Refresh => ex.refresh(),
            Action::Open(row) => ex.activate(row),
            Action::OpenInNewTab(row) => ex.open_in_new_tab(row),
            Action::CopyPath => {
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
            Action::SetSort(sort) => ex.active_mut().set_sort(sort),
            Action::SetShowHidden(show) => self.update_settings(|s| s.show_hidden = show),
            Action::SetDensity(density) => self.update_settings(|s| s.density = density),
        }
    }

    /// Maps shortcuts to explorer calls or actions. Returns true if the selection moved.
    fn handle_keys(&mut self, ctx: &Context, actions: &mut Vec<Action>) -> bool {
        let typing = ctx.egui_wants_keyboard_input();
        let mut moved = false;
        ctx.input_mut(|i| {
            let ex = &mut self.explorer;
            if i.consume_key(Modifiers::CTRL, Key::T) {
                ex.new_tab(ex.active().path().to_path_buf());
            }
            if i.consume_key(Modifiers::CTRL, Key::W) {
                ex.close_tab(ex.active_index());
            }
            if i.consume_key(Modifiers::CTRL, Key::Tab) {
                ex.set_active((ex.active_index() + 1) % ex.tabs().len());
            }
            if i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab) {
                let n = ex.tabs().len();
                ex.set_active((ex.active_index() + n - 1) % n);
            }
            if i.consume_key(Modifiers::NONE, Key::F5) {
                ex.refresh();
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowLeft) {
                ex.back();
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowRight) {
                ex.forward();
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowUp) {
                ex.up();
            }
            if typing {
                return;
            }
            if i.consume_key(Modifiers::NONE, Key::Backspace) {
                ex.back();
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                ex.active_mut().move_selection(1);
                moved = true;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                ex.active_mut().move_selection(-1);
                moved = true;
            }
            let bindings = [
                (Modifiers::NONE, Key::F2, Action::Rename),
                (
                    Modifiers::SHIFT,
                    Key::Delete,
                    Action::Delete { permanent: true },
                ),
                (
                    Modifiers::NONE,
                    Key::Delete,
                    Action::Delete { permanent: false },
                ),
                (Modifiers::CTRL, Key::A, Action::SelectAll),
                (
                    Modifiers::CTRL | Modifiers::SHIFT,
                    Key::N,
                    Action::NewFolder,
                ),
                (Modifiers::ALT, Key::Enter, Action::Properties),
                (Modifiers::CTRL, Key::Comma, Action::OpenSettings),
            ];
            for (modifiers, key, action) in bindings {
                if i.consume_key(modifiers, key) {
                    actions.push(action);
                }
            }
            if i.consume_key(Modifiers::NONE, Key::Enter)
                && let Some(row) = ex.active().selected()
            {
                ex.activate(row);
            }
        });
        moved
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.explorer.poll();
        let mut actions = Vec::new();
        self.scroll_to_selection = self.dialog.is_none() && self.handle_keys(ctx, &mut actions);
        for action in actions {
            self.dispatch(ctx, action);
        }
        let theme = self.explorer.settings().theme;
        if theme != self.applied_theme {
            theme::apply(ctx, theme);
            self.applied_theme = theme;
        }
        if self.toast.is_none() {
            self.toast = self.explorer.take_notice().map(Toast::new);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "fx — {}",
            self.explorer.active().path().display()
        )));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let scroll_to_selection = self.scroll_to_selection;
        let mut actions = Vec::new();
        egui::Panel::top("tabs").show(ui, |ui| {
            views::tabs::show(ui, &mut self.explorer);
        });
        egui::Panel::top("toolbar").show(ui, |ui| {
            views::toolbar::show(ui, &mut self.explorer);
        });
        egui::Panel::top("commands").show(ui, |ui| {
            views::commandbar::show(ui, &self.explorer, &mut actions);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            views::status::show(ui, &self.explorer);
        });
        egui::Panel::left("places")
            .default_size(160.0)
            .show(ui, |ui| {
                views::sidebar::show(ui, &mut self.explorer, &self.places);
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::bg()).inner_margin(4))
            .show(ui, |ui| {
                views::table::show(ui, &mut self.explorer, scroll_to_selection, &mut actions);
            });

        let ctx = ui.ctx().clone();
        if let Some(dialog) = &mut self.dialog
            && dialogs::show(&ctx, dialog, &mut self.explorer)
        {
            self.dialog = None;
        }
        if let Some(current) = &self.toast
            && !toast::show(&ctx, current)
        {
            self.toast = None;
        }
        for action in actions {
            self.dispatch(&ctx, action);
        }
    }
}
