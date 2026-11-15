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

#[cfg(test)]
mod tests {
    use super::show;
    use egui::{PointerButton, Pos2};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use fx_app::Explorer;
    use std::path::PathBuf;

    /// An explorer with tabs `a`, `b`, `c` (last one active) on real directories.
    fn explorer() -> Explorer {
        let root = std::env::temp_dir().join(format!("fx-gui-tabs-{}", std::process::id()));
        let dir = |name: &str| -> PathBuf {
            let path = root.join(name);
            std::fs::create_dir_all(&path).unwrap();
            path
        };
        let mut explorer = Explorer::new(dir("a"), || {});
        explorer.new_tab(dir("b"));
        explorer.new_tab(dir("c"));
        explorer
    }

    /// Renders the strip inside a top panel, as the app does.
    fn harness() -> Harness<'static, Explorer> {
        let mut harness = Harness::new_ui_state(
            |ui, explorer| {
                egui::Panel::top("tabs").show(ui, |ui| show(ui, explorer));
            },
            explorer(),
        );
        crate::theme::apply(&harness.ctx, &fx_app::Settings::default());
        harness.run();
        harness
    }

    /// Like a real user: move, wait, press, wait, release, each in its own frame.
    fn click_at(harness: &mut Harness<'_, Explorer>, pos: Pos2, button: PointerButton) {
        harness.hover_at(pos);
        harness.run();
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Default::default(),
            });
            harness.run();
        }
    }

    fn center(harness: &Harness<'_, Explorer>, label: &str) -> Pos2 {
        harness.get_by_label(label).rect().center()
    }

    fn titles(explorer: &Explorer) -> Vec<String> {
        explorer.tabs().iter().map(|t| t.title()).collect()
    }

    #[test]
    fn close_button_closes_an_inactive_tab() {
        let mut harness = harness();
        let close = harness
            .get_all_by_label("×")
            .next()
            .unwrap()
            .rect()
            .center();
        click_at(&mut harness, close, PointerButton::Primary);
        assert_eq!(titles(harness.state()), ["b", "c"]);
        assert_eq!(harness.state().active_index(), 1);
    }

    #[test]
    fn close_button_closes_the_active_tab() {
        let mut harness = harness();
        let close = harness
            .get_all_by_label("×")
            .last()
            .unwrap()
            .rect()
            .center();
        click_at(&mut harness, close, PointerButton::Primary);
        assert_eq!(titles(harness.state()), ["a", "b"]);
    }

    #[test]
    fn clicking_a_title_selects_the_tab() {
        let mut harness = harness();
        let pos = center(&harness, "a");
        click_at(&mut harness, pos, PointerButton::Primary);
        assert_eq!(titles(harness.state()).len(), 3);
        assert_eq!(harness.state().active_index(), 0);
    }

    #[test]
    fn clicking_tab_padding_selects_the_tab() {
        let mut harness = harness();
        let pos = harness.get_by_label("a").rect().left_center() - egui::vec2(6.0, 0.0);
        click_at(&mut harness, pos, PointerButton::Primary);
        assert_eq!(harness.state().active_index(), 0);
    }

    #[test]
    fn middle_click_closes_the_tab() {
        let mut harness = harness();
        let pos = center(&harness, "a");
        click_at(&mut harness, pos, PointerButton::Middle);
        assert_eq!(titles(harness.state()), ["b", "c"]);
    }
}
