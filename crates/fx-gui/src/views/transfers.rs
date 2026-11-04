//! Transfer panel above the status bar: one row per copy/move with progress, ETA, pause/resume/cancel,
//! the disk-full prompt and the error report of finished transfers.

use crate::theme;
use eframe::egui::{Align, Layout, ProgressBar, RichText, ScrollArea, Ui};
use fx_app::{Explorer, TransferInfo, TransferState, format_size, t, tn, tr};
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
pub fn progress_text(info: &TransferInfo) -> String {
    let Some(s) = &info.snapshot else {
        return String::new();
    };
    let (files, bytes) = if s.walk_complete {
        ("transfer.files", "transfer.bytes")
    } else {
        ("transfer.files_at_least", "transfer.bytes_at_least")
    };
    let (done, total) = (s.files_done.to_string(), s.files_total.to_string());
    let mut parts = vec![
        tr(files, &[("done", &done), ("total", &total)]),
        tr(
            bytes,
            &[
                ("done", &format_size(s.bytes_done)),
                ("total", &format_size(s.bytes_total)),
            ],
        ),
    ];
    if info.rate >= 1.0 {
        parts.push(tr(
            "transfer.rate",
            &[("rate", &format_size(info.rate as u64))],
        ));
    }
    if let Some(eta) = info.eta {
        parts.push(tr("transfer.left", &[("time", &format_eta(eta))]));
    }
    parts.join(" · ")
}

pub fn show(ui: &mut Ui, explorer: &mut Explorer, st: &mut State) {
    for info in explorer.transfers() {
        ui.add_space(2.0);
        row(ui, explorer, st, &info);
    }
    ui.add_space(2.0);
}

fn row(ui: &mut Ui, explorer: &mut Explorer, st: &mut State, info: &TransferInfo) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(&info.label).strong().color(theme::text()));
        let status = match info.state {
            TransferState::Queued => Some((t("transfer.waiting"), theme::dim())),
            TransferState::Paused => Some((t("transfer.paused"), theme::dim())),
            TransferState::DiskFull => Some((t("transfer.disk_full"), theme::error())),
            TransferState::Running | TransferState::Finished => None,
        };
        if let Some((text, color)) = status {
            ui.colored_label(color, text);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            buttons(ui, explorer, st, info)
        });
    });
    if let Some(s) = &info.snapshot {
        let fraction = if s.bytes_total > 0 {
            s.bytes_done as f32 / s.bytes_total as f32
        } else if s.files_total > 0 {
            s.files_done as f32 / s.files_total as f32
        } else {
            0.0
        };
        ui.add(ProgressBar::new(fraction.clamp(0.0, 1.0)).desired_height(6.0));
        ui.horizontal(|ui| {
            ui.colored_label(theme::dim(), progress_text(info));
            if let Some(current) = &s.current {
                ui.colored_label(theme::dim(), format!("· {}", current.display()));
            }
        });
    }
    if let Some(summary) = &info.summary {
        let errors = summary.error_count();
        let mut parts = Vec::new();
        if summary.cancelled {
            parts.push(tn("transfer.cancelled_after", summary.files, &[]));
        } else if errors > 0 {
            parts.push(tn("transfer.finished_errors", errors as u64, &[]));
        }
        if !summary.kept_sources.is_empty() {
            parts.push(tn("transfer.kept", summary.kept_sources.len() as u64, &[]));
        }
        let color = if errors > 0 {
            theme::error()
        } else {
            theme::dim()
        };
        ui.colored_label(color, parts.join(" · "));
    }
    if st.expanded.contains(&info.id) {
        report(ui, info);
    }
}

fn buttons(ui: &mut Ui, explorer: &mut Explorer, st: &mut State, info: &TransferInfo) {
    match info.state {
        TransferState::Finished => {
            if ui.button(t("transfer.dismiss")).clicked() {
                st.expanded.remove(&info.id);
                explorer.dismiss_transfer(info.id);
            }
        }
        TransferState::DiskFull => {
            if ui.button(t("button.cancel")).clicked() {
                explorer.cancel_transfer(info.id);
            }
            if ui.button(t("transfer.skip_full")).clicked() {
                explorer.skip_disk_full(info.id);
            }
            if ui.button(t("transfer.retry")).clicked() {
                explorer.resume_transfer(info.id);
            }
        }
        TransferState::Queued | TransferState::Running | TransferState::Paused => {
            if ui.button(t("button.cancel")).clicked() {
                explorer.cancel_transfer(info.id);
            }
            if info.state == TransferState::Paused {
                if ui.button(t("transfer.resume")).clicked() {
                    explorer.resume_transfer(info.id);
                }
            } else if info.state == TransferState::Running
                && ui.button(t("transfer.pause")).clicked()
            {
                explorer.pause_transfer(info.id);
            }
        }
    }
    if !info.errors.is_empty() {
        let open = st.expanded.contains(&info.id);
        let key = if open {
            "transfer.hide_problems"
        } else {
            "transfer.show_problems"
        };
        let label = tn(key, info.errors.len() as u64, &[]);
        if ui.button(label).clicked() && !st.expanded.remove(&info.id) {
            st.expanded.insert(info.id);
        }
    }
}

/// The per-item report, with a button that copies it as text for a bug report or a retry list.
fn report(ui: &mut Ui, info: &TransferInfo) {
    let text: Vec<String> = info
        .errors
        .iter()
        .map(|e| {
            let tag = t(if e.warning {
                "transfer.warning"
            } else {
                "transfer.error"
            });
            format!("{tag}: {e}")
        })
        .collect();
    ScrollArea::vertical()
        .id_salt(("transfer-report", info.id))
        .max_height(REPORT_HEIGHT)
        .show(ui, |ui| {
            for (e, line) in info.errors.iter().zip(&text) {
                let color = if e.warning {
                    theme::dim()
                } else {
                    theme::error()
                };
                ui.colored_label(color, line);
            }
        });
    if ui.button(t("transfer.copy_report")).clicked() {
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
