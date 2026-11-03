//! Transient notice shown bottom-right above the status bar.

use crate::theme;
use eframe::egui::{Align2, Area, Context, Frame, Id, Margin, Order, RichText, Sense, vec2};
use fx_app::{Notice, NoticeKind};
use std::time::{Duration, Instant};

const LIFETIME: Duration = Duration::from_secs(4);
/// Clears the status bar.
const BOTTOM_OFFSET: f32 = 40.0;

pub struct Toast {
    notice: Notice,
    shown_at: Instant,
}

impl Toast {
    pub fn new(notice: Notice) -> Self {
        Self {
            notice,
            shown_at: Instant::now(),
        }
    }
}

/// Draws the toast. Returns false once it expired or was clicked.
pub fn show(ctx: &Context, toast: &Toast) -> bool {
    let remaining = LIFETIME.saturating_sub(toast.shown_at.elapsed());
    if remaining.is_zero() {
        return false;
    }
    ctx.request_repaint_after(remaining);
    let color = match toast.notice.kind {
        NoticeKind::Info => theme::accent(),
        NoticeKind::Error => theme::error(),
    };
    let fill = match toast.notice.kind {
        NoticeKind::Info => theme::panel_hi(),
        NoticeKind::Error => theme::error().gamma_multiply(0.25),
    };
    let area = Area::new(Id::unique("toast"))
        .order(Order::Tooltip)
        .anchor(Align2::RIGHT_BOTTOM, vec2(-12.0, -BOTTOM_OFFSET))
        .show(ctx, |ui| {
            Frame::new()
                .fill(fill)
                .stroke(egui::Stroke::new(1.0, color))
                .corner_radius(4)
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_max_width(420.0);
                    ui.label(RichText::new(&toast.notice.text).color(theme::text()));
                })
                .response
                .interact(Sense::click())
        });
    !area.inner.clicked()
}
