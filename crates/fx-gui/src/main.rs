//! fx-gui: egui presentation layer over `fx-app`. Renders state and forwards input; holds no logic.

mod app;
mod theme;
mod views;

use std::path::PathBuf;

fn main() -> eframe::Result {
    let start = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 750.0])
            .with_title("fx"),
        ..Default::default()
    };
    eframe::run_native(
        "fx",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, start)))),
    )
}
