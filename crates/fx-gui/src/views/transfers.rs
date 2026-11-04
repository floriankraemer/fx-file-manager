//! Transfer panel above the status bar: one row per copy/move with progress, ETA, pause/resume/cancel,
//! the disk-full prompt and the error report of finished transfers.

use crate::theme;
use eframe::egui::{Align, Layout, ProgressBar, RichText, ScrollArea, Ui};
use fx_app::{Explorer, TransferInfo, TransferState, format_size};
use std::collections::HashSet;
use std::time::Duration;

/// Error rows shown before the list scrolls.
const REPORT_HEIGHT: f32 = 140.0;

/// Which finished transfers have their error report expanded.
#[derive(Default)]
pub struct State {
    expanded: HashSet<u64>,
}

/// `1:05` or `1:02:03`.
pub fn format_eta(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// `120 of at least 400 files · 1.2 GB of 3.4 GB · 85 MB/s · 0:42 left`
pub fn progress_text(t: &TransferInfo) -> String {
    let Some(s) = &t.snapshot else {
        return String::new();
    };
    let at_least = if s.walk_complete { "" } else { "at least " };
    let mut parts = vec![
        format!("{} of {at_least}{} files", s.files_done, s.files_total),
        format!(
            "{} of {at_least}{}",
            format_size(s.bytes_done),
            format_size(s.bytes_total)
        ),
    ];
    if t.rate >= 1.0 {
        parts.push(format!("{}/s", format_size(t.rate as u64)));
    }
    if let Some(eta) = t.eta {
        parts.push(format!("{} left", format_eta(eta)));
    }
    parts.join(" · ")
}

pub fn show(ui: &mut Ui, explorer: &mut Explorer, st: &mut State) {
    for t in explorer.transfers() {
        ui.add_space(2.0);
        row(ui, explorer, st, &t);
    }
    ui.add_space(2.0);
}

fn row(ui: &mut Ui, explorer: &mut Explorer, st: &mut State, t: &TransferInfo) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(&t.label).strong().color(theme::text()));
        let status = match t.state {
            TransferState::Queued => Some(("Waiting for the disk", theme::dim())),
            TransferState::Paused => Some(("Paused", theme::dim())),
            TransferState::DiskFull => Some(("The destination is full", theme::error())),
            TransferState::Running | TransferState::Finished => None,
        };
        if let Some((text, color)) = status {
            ui.colored_label(color, text);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            buttons(ui, explorer, st, t)
        });
    });
    if let Some(s) = &t.snapshot {
        let fraction = if s.bytes_total > 0 {
            s.bytes_done as f32 / s.bytes_total as f32
        } else if s.files_total > 0 {
            s.files_done as f32 / s.files_total as f32
        } else {
            0.0
        };
        ui.add(ProgressBar::new(fraction.clamp(0.0, 1.0)).desired_height(6.0));
        ui.horizontal(|ui| {
            ui.colored_label(theme::dim(), progress_text(t));
            if let Some(current) = &s.current {
                ui.colored_label(theme::dim(), format!("· {}", current.display()));
            }
        });
    }
    if let Some(summary) = &t.summary {
        let errors = summary.error_count();
        let mut parts = Vec::new();
        if summary.cancelled {
            parts.push(format!("Cancelled after {} files", summary.files));
        } else if errors > 0 {
            parts.push(format!("Finished with {}", plural_of(errors, "error")));
        }
        if !summary.kept_sources.is_empty() {
            parts.push(format!(
                "{} left in place because not everything was moved",
                plural_of(summary.kept_sources.len(), "source")
            ));
        }
        let color = if errors > 0 {
            theme::error()
        } else {
            theme::dim()
        };
        ui.colored_label(color, parts.join(" · "));
    }
    if st.expanded.contains(&t.id) {
        report(ui, t);
    }
}

fn buttons(ui: &mut Ui, explorer: &mut Explorer, st: &mut State, t: &TransferInfo) {
    match t.state {
        TransferState::Finished => {
            if ui.button("Dismiss").clicked() {
                st.expanded.remove(&t.id);
                explorer.dismiss_transfer(t.id);
            }
        }
        TransferState::DiskFull => {
            if ui.button("Cancel").clicked() {
                explorer.cancel_transfer(t.id);
            }
            if ui.button("Skip files that do not fit").clicked() {
                explorer.skip_disk_full(t.id);
            }
            if ui.button("Retry").clicked() {
                explorer.resume_transfer(t.id);
            }
        }
        TransferState::Queued | TransferState::Running | TransferState::Paused => {
            if ui.button("Cancel").clicked() {
                explorer.cancel_transfer(t.id);
            }
            if t.state == TransferState::Paused {
                if ui.button("Resume").clicked() {
                    explorer.resume_transfer(t.id);
                }
            } else if t.state == TransferState::Running && ui.button("Pause").clicked() {
                explorer.pause_transfer(t.id);
            }
        }
    }
    if !t.errors.is_empty() {
        let open = st.expanded.contains(&t.id);
        let label = format!(
            "{} {}",
            if open { "Hide" } else { "Show" },
            plural_of(t.errors.len(), "problem")
        );
        if ui.button(label).clicked() && !st.expanded.remove(&t.id) {
            st.expanded.insert(t.id);
        }
    }
}

fn plural_of(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// The per-item report, with a button that copies it as text for a bug report or a retry list.
fn report(ui: &mut Ui, t: &TransferInfo) {
    let text: Vec<String> = t
        .errors
        .iter()
        .map(|e| {
            let tag = if e.warning { "warning" } else { "error" };
            format!("{tag}: {e}")
        })
        .collect();
    ScrollArea::vertical()
        .id_salt(("transfer-report", t.id))
        .max_height(REPORT_HEIGHT)
        .show(ui, |ui| {
            for (e, line) in t.errors.iter().zip(&text) {
                let color = if e.warning {
                    theme::dim()
                } else {
                    theme::error()
                };
                ui.colored_label(color, line);
            }
        });
    if ui.button("Copy report").clicked() {
        ui.ctx().copy_text(text.join("\n"));
    }
    ui.separator();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eta_formats_minutes_and_hours() {
        assert_eq!(format_eta(Duration::from_secs(65)), "1:05");
        assert_eq!(format_eta(Duration::from_secs(3723)), "1:02:03");
        assert_eq!(format_eta(Duration::ZERO), "0:00");
    }
}
