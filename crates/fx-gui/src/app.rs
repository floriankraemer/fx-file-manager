//! eframe::App: polls the explorer, maps input to its methods, lays out the views.

use crate::action::Action;
use crate::command::{self, Command};
use crate::views::dialogs::{self, Dialog};
use crate::views::toast::{self, Toast};
use crate::{theme, views};
use eframe::egui::{self, Context};
use fx_app::{Explorer, Language, Notice, NoticeKind, Place, Settings, i18n, places};
use std::collections::BTreeMap;
use std::path::PathBuf;

const TRANSFER_REFRESH: std::time::Duration = std::time::Duration::from_millis(100);

pub struct App {
    explorer: Explorer,
    places: Vec<Place>,
    scroll_to_selection: bool,
    dialog: Option<Dialog>,
    /// The settings window, shown natively while open.
    settings: Option<Box<views::settings::State>>,
    toast: Option<Toast>,
    transfers: views::transfers::State,
    applied_look: Look,
    was_focused: bool,
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
    pub fn new(cc: &eframe::CreationContext<'_>, start: PathBuf, settings: Settings) -> Self {
        i18n::set_language(settings.language);
        fx_app::set_date_format(settings.date_format);
        let applied_look = look(&settings);
        theme::apply(&cc.egui_ctx, &settings);
        let ctx = cc.egui_ctx.clone();
        let mut app = Self {
            explorer: Explorer::with_settings(start, settings, move || ctx.request_repaint()),
            places: places(),
            scroll_to_selection: false,
            dialog: None,
            settings: None,
            toast: None,
            transfers: views::transfers::State::default(),
            applied_look,
            was_focused: true,
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
                if self.settings.is_some() {
                    views::settings::focus(ctx);
                } else if self.dialog.is_none() {
                    let state = views::settings::State::new(ex.settings());
                    self.settings = Some(Box::new(state));
                }
            }
            Action::OpenAbout => self.open_dialog(Some(Dialog::About)),
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
            Command::OpenSettings => actions.push(Action::OpenSettings),
            Command::About => actions.push(Action::OpenAbout),
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

impl eframe::App for App {
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.explorer.poll();
        // Keep polling while idle so background revalidation results show up.
        ctx.request_repaint_after(fx_app::REVALIDATE_INTERVAL);
        if self.explorer.transfers_active() {
            // Progress is sampled, not pushed: 10 Hz is smooth and costs nothing.
            ctx.request_repaint_after(TRANSFER_REFRESH);
        }
        if self.dialog.is_none() {
            self.dialog = Dialog::conflict(&self.explorer);
        }
        let focused = ctx.input(|i| i.focused);
        if focused && !self.was_focused {
            self.explorer.revalidate();
        }
        self.was_focused = focused;
        let mut actions = Vec::new();
        self.scroll_to_selection =
            self.dialog.is_none() && self.settings.is_none() && self.handle_keys(ctx, &mut actions);
        for action in actions {
            self.dispatch(ctx, action);
        }
        fx_app::set_date_format(self.explorer.settings().date_format);
        if self.explorer.settings().keymap != self.applied_keymap {
            self.apply_keymap();
        }
        let current = look(self.explorer.settings());
        if current != self.applied_look {
            if current.4 != self.applied_look.4 {
                i18n::set_language(current.4);
                // Place labels are translated when listed.
                self.places = places();
            }
            theme::apply(ctx, self.explorer.settings());
            self.applied_look = current;
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
        egui::Panel::top("menubar").show(ui, |ui| {
            views::menubar::show(ui, &self.explorer, &mut actions);
        });
        egui::Panel::top("tabs").show(ui, |ui| {
            views::tabs::show(ui, &mut self.explorer);
        });
        egui::Panel::top("toolbar").show(ui, |ui| {
            views::toolbar::show(ui, &mut self.explorer, &mut actions);
        });
        egui::Panel::top("commands").show(ui, |ui| {
            views::commandbar::show(ui, &self.explorer, &mut actions);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            views::status::show(ui, &self.explorer);
        });
        if !self.explorer.transfers().is_empty() {
            egui::Panel::bottom("transfers").show(ui, |ui| {
                views::transfers::show(ui, &mut self.explorer, &mut self.transfers);
            });
        }
        egui::Panel::left("places")
            .default_size(240.0)
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
        if let Some(state) = &mut self.settings {
            if views::settings::show(&ctx, &mut self.explorer, state) {
                self.settings = None;
            } else if !state.embedded() && block_input(&ctx) {
                views::settings::focus(&ctx);
            }
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
