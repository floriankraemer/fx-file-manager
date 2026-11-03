use crate::theme;
use eframe::egui::{Align, Layout, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};
use fx_app::{Explorer, LoadState, SortColumn, age, extension, format_size, format_time};
use fx_core::EntryKind;
use std::time::SystemTime;

const ROW_HEIGHT: f32 = 20.0;

pub fn show(ui: &mut Ui, explorer: &mut Explorer, scroll_to_selection: bool) {
    let tab = explorer.active();
    let view = match tab.state() {
        LoadState::Loading => {
            ui.centered_and_justified(|ui| {
                ui.spinner();
            });
            return;
        }
        LoadState::Failed(message) => {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.colored_label(
                    theme::ERROR,
                    format!("Cannot list {}", tab.path().display()),
                );
                ui.colored_label(theme::ERROR, message);
            });
            return;
        }
        LoadState::Loaded(view) => view.clone(),
    };

    let sort = tab.sort();
    let selected = tab.selected();
    let order = tab.order().to_vec();
    let now = SystemTime::now();
    let mut clicked_sort = None;
    let mut select = None;
    let mut activate = None;

    let mut builder = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::remainder().at_least(160.0).clip(true))
        .column(Column::initial(60.0).at_least(30.0))
        .column(Column::initial(80.0).at_least(50.0))
        .column(Column::initial(140.0).at_least(80.0))
        .column(Column::initial(56.0).at_least(40.0));
    if scroll_to_selection && let Some(row) = selected {
        builder = builder.scroll_to_row(row, None);
    }

    builder
        .header(22.0, |mut header| {
            let columns = [
                ("Name", Some(SortColumn::Name)),
                ("Ext", Some(SortColumn::Ext)),
                ("Size", Some(SortColumn::Size)),
                ("Modified", Some(SortColumn::Modified)),
                ("Age", None),
            ];
            for (title, column) in columns {
                header.col(|ui| {
                    let arrow = match column {
                        Some(c) if c == sort.column => {
                            if sort.ascending {
                                " ▲"
                            } else {
                                " ▼"
                            }
                        }
                        _ => "",
                    };
                    let text = RichText::new(format!("{title}{arrow}"))
                        .strong()
                        .color(theme::HEADER_TEXT);
                    let r = ui.max_rect();
                    ui.painter().hline(
                        r.x_range(),
                        r.bottom(),
                        egui::Stroke::new(1.5, theme::ACCENT.gamma_multiply(0.5)),
                    );
                    let resp = ui.add(egui::Label::new(text).sense(Sense::click()));
                    if let Some(column) = column
                        && resp.clicked()
                    {
                        clicked_sort = Some(column);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, order.len(), |mut row| {
                let i = row.index();
                let entry = &view.listing.entries[order[i] as usize];
                let name = view.listing.name(entry).to_string_lossy();
                row.set_selected(selected == Some(i));
                row.col(|ui| {
                    super::icons::entry(ui, entry.kind);
                    ui.add(egui::Label::new(name.as_ref()).truncate());
                });
                row.col(|ui| {
                    ui.colored_label(theme::DIM, extension(&name));
                });
                row.col(|ui| {
                    if entry.kind != EntryKind::Dir
                        && let Some(size) = entry.size
                    {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(format_size(size));
                        });
                    }
                });
                row.col(|ui| {
                    if let Some(time) = entry.modified {
                        ui.colored_label(theme::DIM, format_time(time));
                    }
                });
                row.col(|ui| {
                    let (bucket, text) = age(entry.modified, now);
                    if !text.is_empty() {
                        age_badge(ui, &text, theme::age_color(bucket));
                    }
                });
                let resp = row.response();
                if resp.clicked() {
                    select = Some(i);
                }
                if resp.double_clicked() {
                    activate = Some(i);
                }
            });
        });

    let tab = explorer.active_mut();
    if let Some(column) = clicked_sort {
        tab.set_sort_column(column);
    }
    if let Some(i) = select {
        tab.select(Some(i));
    }
    if let Some(i) = activate {
        explorer.activate(i);
    }
}

fn age_badge(ui: &mut Ui, text: &str, color: egui::Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.25))
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(5, 0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().color(color));
        });
}
