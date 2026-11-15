use super::menu::bar_button;
use crate::command::{Command, tip};
use crate::theme;
use eframe::egui::{Sense, Ui};
use fx_app::Explorer;

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
                .inner_margin(egui::Margin {
                    left: 12,
                    right: 6,
                    top: 4,
                    bottom: 4,
                });
            let resp = ui
                .scope_builder(egui::UiBuilder::new().sense(Sense::click()), |ui| {
                    frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            // Not selectable: a selectable label senses clicks and would swallow the tab's own click.
                            let title = egui::RichText::new(tab.title()).color(if selected {
                                theme::accent()
                            } else {
                                theme::text()
                            });
                            ui.add(egui::Label::new(title).selectable(false));
                            if ui
                                .scope(|ui| {
                                    ui.style_mut().spacing.button_padding = egui::vec2(4.0, 2.0);
                                    ui.add(egui::Button::new("×").frame_when_inactive(false))
                                })
                                .inner
                                .on_hover_text(tip("tip.close_tab", Command::CloseTab))
                                .clicked()
                            {
                                close = Some(i);
                            }
                        });
                    });
                })
                .response;
            if resp.clicked() {
                activate = Some(i);
            }
            if resp.middle_clicked() {
                close = Some(i);
            }
        }
        if bar_button(ui, "+", true)
            .on_hover_text(tip("tip.new_tab", Command::NewTab))
            .clicked()
        {
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
