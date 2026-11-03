use crate::theme;
use eframe::egui::{CursorIcon, Layout, RichText, Sense, Stroke, TextStyle, Ui, vec2};
use fx_app::{Explorer, breadcrumbs};

pub fn show(ui: &mut Ui, explorer: &mut Explorer) {
    let (can_back, can_forward, can_up) = {
        let tab = explorer.active();
        (tab.can_back(), tab.can_forward(), tab.can_up())
    };
    let mut goto = None;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(can_back, egui::Button::new("←"))
            .on_hover_text("Back (Alt+←)")
            .clicked()
        {
            explorer.back();
        }
        if ui
            .add_enabled(can_forward, egui::Button::new("→"))
            .on_hover_text("Forward (Alt+→)")
            .clicked()
        {
            explorer.forward();
        }
        if ui
            .add_enabled(can_up, egui::Button::new("↑"))
            .on_hover_text("Up (Alt+↑)")
            .clicked()
        {
            explorer.up();
        }
        if ui.button("⟳").on_hover_text("Refresh (F5)").clicked() {
            explorer.refresh();
        }
        ui.separator();

        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            let tab = explorer.active_mut();
            let mut hidden = tab.show_hidden();
            if ui.checkbox(&mut hidden, "Hidden").changed() {
                tab.set_show_hidden(hidden);
            }
            let mut filter = tab.filter().to_owned();
            let edit = egui::TextEdit::singleline(&mut filter)
                .hint_text("Filter…")
                .desired_width(160.0);
            if ui.add(edit).changed() {
                tab.set_filter(&filter);
            }
            ui.with_layout(Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let crumbs = breadcrumbs(explorer.active().path());
                let last = crumbs.len().saturating_sub(1);
                for (i, (label, path)) in crumbs.into_iter().enumerate() {
                    if crumb(ui, &label, i == last) {
                        goto = Some(path);
                    }
                    if i != last {
                        ui.label(RichText::new("›").color(theme::DIM));
                    }
                }
            });
        });
    });
    if let Some(path) = goto {
        explorer.navigate(path);
    }
}

/// One clickable breadcrumb: padded, highlighted and underlined on hover, accent when current.
fn crumb(ui: &mut Ui, label: &str, current: bool) -> bool {
    let font = TextStyle::Body.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, theme::TEXT);
    let size = vec2((galley.size().x + 12.0).max(22.0), 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    let color = match (current, hovered) {
        (true, _) => theme::ACCENT,
        (false, true) => theme::HEADER_TEXT,
        (false, false) => theme::TEXT,
    };
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 3.0, theme::PANEL_HI);
    }
    let text_pos = rect.center() - galley.size() / 2.0;
    painter.galley(text_pos, galley.clone(), color);
    if hovered {
        let y = text_pos.y + galley.size().y;
        painter.hline(
            text_pos.x..=text_pos.x + galley.size().x,
            y,
            Stroke::new(1.0, color),
        );
    }
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}
