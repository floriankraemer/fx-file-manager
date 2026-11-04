use crate::theme;
use eframe::egui::{Sense, Ui};
use fx_app::{Explorer, t};

pub fn show(ui: &mut Ui, explorer: &mut Explorer) {
    let mut activate = None;
    let mut close = None;
    let mut add = false;
    ui.horizontal(|ui| {
        let active = explorer.active_index();
        for (i, tab) in explorer.tabs().iter().enumerate() {
            let selected = i == active;
            let frame = egui::Frame::new()
                .fill(if selected {
                    theme::bg()
                } else {
                    theme::panel_hi()
                })
                .stroke(if selected {
                    egui::Stroke::new(1.0, theme::accent())
                } else {
                    egui::Stroke::NONE
                })
                .corner_radius(egui::CornerRadius {
                    nw: 4,
                    ne: 4,
                    sw: 0,
                    se: 0,
                })
                .inner_margin(egui::Margin::symmetric(8, 2));
            let resp = frame
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(egui::RichText::new(tab.title()).color(if selected {
                            theme::accent()
                        } else {
                            theme::text()
                        }));
                        if ui
                            .small_button("×")
                            .on_hover_text(t("tip.close_tab"))
                            .clicked()
                        {
                            close = Some(i);
                        }
                    });
                })
                .response
                .interact(Sense::click());
            if resp.clicked() {
                activate = Some(i);
            }
            if resp.middle_clicked() {
                close = Some(i);
            }
        }
        if ui.button("+").on_hover_text(t("tip.new_tab")).clicked() {
            add = true;
        }
    });
    if add {
        explorer.new_tab(explorer.active().path().to_path_buf());
    }
    if let Some(i) = activate {
        explorer.set_active(i);
    }
    if let Some(i) = close {
        explorer.close_tab(i);
    }
}
