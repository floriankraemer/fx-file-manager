//! eframe::App: polls the explorer, maps input to its methods, lays out the views.

use crate::{theme, views};
use eframe::egui::{self, Context, Key, Modifiers};
use fx_app::{Explorer, Place, places};
use std::path::PathBuf;

pub struct App {
    explorer: Explorer,
    places: Vec<Place>,
    scroll_to_selection: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, start: PathBuf) -> Self {
        theme::apply(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        Self {
            explorer: Explorer::new(start, move || ctx.request_repaint()),
            places: places(),
            scroll_to_selection: false,
        }
    }

    /// Maps shortcuts to explorer calls. Returns true if the selection moved.
    fn handle_keys(&mut self, ctx: &Context) -> bool {
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
        self.scroll_to_selection = self.handle_keys(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "fx — {}",
            self.explorer.active().path().display()
        )));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let scroll_to_selection = self.scroll_to_selection;
        egui::Panel::top("tabs").show(ui, |ui| {
            views::tabs::show(ui, &mut self.explorer);
        });
        egui::Panel::top("toolbar").show(ui, |ui| {
            views::toolbar::show(ui, &mut self.explorer);
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
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(4))
            .show(ui, |ui| {
                views::table::show(ui, &mut self.explorer, scroll_to_selection);
            });
    }
}
