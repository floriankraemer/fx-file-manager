//! fx-gui: egui presentation layer over `fx-app`. Renders state and forwards input; holds no logic.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod action;
mod app;
mod command;
mod theme;
mod views;

use std::path::PathBuf;

fn main() -> eframe::Result {
    prefer_x11();
    log_startup(fx_app::logging::init());
    let settings = fx_app::Settings::load();
    let start = std::env::args_os().nth(1).map(PathBuf::from);
    let fallback = fx_app::start_dir(None, &settings);
    let state = fx_app::State::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 750.0])
            .with_title("fx")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))
                    .expect("embedded app icon is a valid PNG"),
            ),
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

fn log_startup(log_file: Option<PathBuf>) {
    log::info!(
        "fx {} ({}, built {}) on {} {}, log file {}",
        env!("CARGO_PKG_VERSION"),
        env!("FX_GIT_HASH"),
        env!("FX_BUILD_DATE"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        log_file
            .as_deref()
            .map_or("unavailable".into(), |p| p.display().to_string()),
    );
}

/// Dragging files out of fx to other programs is implemented for X11 only, so Linux sessions that
/// offer both Wayland and X11 run fx through X11.
#[cfg(all(unix, not(target_os = "macos")))]
fn prefer_x11() {
    views::dnd::note_wayland_session(std::env::var_os("WAYLAND_DISPLAY").is_some());
    if std::env::var_os("DISPLAY").is_some() {
        // SAFETY: runs first in `main`, before any other thread exists.
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn prefer_x11() {}
