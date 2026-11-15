//! fx-gui: egui presentation layer over `fx-app`. Renders state and forwards input; holds no logic.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod action;
mod app;
mod command;
mod theme;
mod views;

use std::path::PathBuf;

fn main() -> eframe::Result {
    let settings = fx_app::Settings::load();
    let start = std::env::args_os().nth(1).map(PathBuf::from);
    let fallback = fx_app::start_dir(None, &settings);
    let state = fx_app::State::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 750.0])
            .with_title("fx"),
        ..Default::default()
    };
    eframe::run_native(
        "fx",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::App::new(
                cc, start, fallback, settings, state,
            )))
        }),
    )
}
