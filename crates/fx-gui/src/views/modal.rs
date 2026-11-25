//! Building blocks shared by every modal: the frame, its button row and the default-button focus.

use eframe::egui::{self, Align, Context, Id, Layout, Modal, Ui};

const WIDTH: f32 = 380.0;

pub fn modal(ctx: &Context, id: &str, content: impl FnOnce(&mut Ui) -> bool) -> bool {
    let response = Modal::new(Id::unique(id)).show(ctx, |ui| {
        ui.set_width(WIDTH);
        content(ui)
    });
    response.inner || response.should_close()
}

/// Right-aligned button row; the closure adds buttons in right-to-left order (primary first).
pub fn button_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.add_space(8.0);
    ui.with_layout(Layout::right_to_left(Align::Center), add);
}

/// Focuses `response` once, while nothing else holds the focus, so Enter triggers the default button.
pub fn default_focus(ui: &Ui, response: &egui::Response) {
    if ui.memory(|m| m.focused().is_none()) {
        response.request_focus();
    }
}
