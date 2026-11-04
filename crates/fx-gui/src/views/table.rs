use super::commandbar::menu_item;
use crate::{action::Action, theme};
use eframe::egui::{Align, Layout, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};
use fx_app::{
    Density, Explorer, LoadState, SortColumn, age, extension, format_size, format_time, t, tr,
};
use fx_core::EntryKind;
use std::time::SystemTime;

fn row_height(density: Density) -> f32 {
    match density {
        Density::Compact => 20.0,
        Density::Comfortable => 28.0,
    }
}

pub fn show(
    ui: &mut Ui,
    explorer: &mut Explorer,
    scroll_to_selection: bool,
    actions: &mut Vec<Action>,
) {
    // Selectable labels grab clicks and drags, so rows would never see double/right clicks.
    ui.style_mut().interaction.selectable_labels = false;
    let settings = explorer.settings().clone();
    let can_paste = explorer.can_paste();
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
                    theme::error(),
                    tr(
                        "table.cannot_list",
                        &[("path", &tab.path().display().to_string())],
                    ),
                );
                ui.colored_label(theme::error(), message);
            });
            return;
        }
        LoadState::Loaded(view) => view.clone(),
    };

    let sort = tab.sort();
    let selected = tab.selected();
    let selection = tab.selection();
    let order = tab.order().to_vec();
    let modifiers = ui.input(|input| input.modifiers);
    let now = SystemTime::now();
    let mut clicked_sort = None;
    let mut select = None;
    let mut activate = None;
    let mut right_clicked = None;

    let mut builder = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::remainder().at_least(160.0).clip(true));
    if settings.show_ext_column {
        builder = builder.column(Column::initial(60.0).at_least(30.0));
    }
    builder = builder
        .column(Column::initial(80.0).at_least(50.0))
        .column(Column::initial(140.0).at_least(80.0));
    if settings.show_age_column {
        builder = builder.column(Column::initial(56.0).at_least(40.0));
    }
    if scroll_to_selection && let Some(row) = selected {
        builder = builder.scroll_to_row(row, None);
    }

    let scroll = builder
        .header(22.0, |mut header| {
            let columns = [
                (t("col.name"), Some(SortColumn::Name), true),
                (
                    t("col_short.ext"),
                    Some(SortColumn::Ext),
                    settings.show_ext_column,
                ),
                (t("col.size"), Some(SortColumn::Size), true),
                (t("col.modified"), Some(SortColumn::Modified), true),
                (t("col.age"), None, settings.show_age_column),
            ];
            for (title, column, _) in columns.into_iter().filter(|c| c.2) {
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
                        .color(theme::header_text());
                    let r = ui.max_rect();
                    ui.painter().hline(
                        r.x_range(),
                        r.bottom(),
                        egui::Stroke::new(1.5, theme::accent().gamma_multiply(0.5)),
                    );
                    // The whole cell is the click target, not just the label.
                    let resp = ui.interact(r, ui.scope_id().with("sort"), Sense::click());
                    ui.add(egui::Label::new(text).selectable(false));
                    if let Some(column) = column
                        && resp.clicked()
                    {
                        clicked_sort = Some(column);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(
                theme::row_height(row_height(settings.density), settings.list_font_size),
                order.len(),
                |mut row| {
                    let i = row.index();
                    let entry = &view.listing.entries[order[i] as usize];
                    let name = view.listing.name(entry).to_string_lossy();
                    row.set_selected(selection.binary_search(&i).is_ok());
                    row.col(|ui| {
                        super::icons::entry(ui, entry.kind, &name);
                        ui.add(
                            egui::Label::new(
                                RichText::new(name.as_ref()).text_style(theme::list_style()),
                            )
                            .truncate(),
                        );
                    });
                    if settings.show_ext_column {
                        row.col(|ui| {
                            if entry.kind != EntryKind::Dir {
                                ui.colored_label(
                                    theme::dim(),
                                    RichText::new(extension(&name)).text_style(theme::list_style()),
                                );
                            }
                        });
                    }
                    row.col(|ui| {
                        if entry.kind != EntryKind::Dir
                            && let Some(size) = entry.size
                        {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format_size(size))
                                        .text_style(theme::list_style()),
                                );
                            });
                        }
                    });
                    row.col(|ui| {
                        if let Some(time) = entry.modified {
                            ui.colored_label(
                                theme::dim(),
                                RichText::new(format_time(time)).text_style(theme::list_style()),
                            );
                        }
                    });
                    if settings.show_age_column {
                        row.col(|ui| {
                            let (bucket, text) = age(entry.modified, now);
                            if !text.is_empty() {
                                age_badge(ui, &text, theme::age_color(bucket));
                            }
                        });
                    }
                    let resp = row.response();
                    if resp.secondary_clicked() && selection.binary_search(&i).is_err() {
                        right_clicked = Some(i);
                    }
                    let selected_count = if selection.binary_search(&i).is_ok() {
                        selection.len()
                    } else {
                        1
                    };
                    resp.context_menu(|ui| {
                        row_menu(
                            ui,
                            i,
                            entry.kind == EntryKind::Dir,
                            selected_count,
                            can_paste,
                            actions,
                        );
                    });
                    if resp.clicked() {
                        select = Some(i);
                    }
                    let is_dir = entry.kind == EntryKind::Dir;
                    if if is_dir && settings.list_single_click {
                        resp.clicked()
                    } else {
                        resp.double_clicked()
                    } {
                        activate = Some(i);
                    }
                },
            );
        });

    // The space below the last row; `body` reports the scroll area, so rows keep their own clicks.
    let rows_bottom =
        scroll.inner_rect.top() + scroll.content_size.y - scroll.state.unclamped_offset().y;
    let empty = egui::Rect::from_min_max(
        egui::pos2(
            scroll.inner_rect.left(),
            rows_bottom.max(scroll.inner_rect.top()),
        ),
        scroll.inner_rect.max,
    );
    if empty.height() > 1.0 {
        let background = ui.interact(
            empty,
            ui.scope_id().with("table-background"),
            Sense::click(),
        );
        background.context_menu(|ui| background_menu(ui, settings.show_hidden, can_paste, actions));
    }

    let tab = explorer.active_mut();
    if let Some(i) = right_clicked {
        tab.click(i, false, false);
    }
    if let Some(column) = clicked_sort {
        tab.set_sort_column(column);
    }
    if let Some(i) = select {
        tab.click(i, modifiers.command, modifiers.shift);
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

fn row_menu(
    ui: &mut Ui,
    row: usize,
    is_dir: bool,
    selected: usize,
    can_paste: bool,
    actions: &mut Vec<Action>,
) {
    if menu_item(ui, t("cmd.open"), "Enter", true) {
        actions.push(Action::Open(row));
    }
    if is_dir && menu_item(ui, t("cmd.open_new_tab"), "", true) {
        actions.push(Action::OpenInNewTab(row));
    }
    ui.separator();
    if menu_item(ui, t("cmd.cut"), "Ctrl+X", true) {
        actions.push(Action::Cut);
    }
    if menu_item(ui, t("cmd.copy"), "Ctrl+C", true) {
        actions.push(Action::Copy);
    }
    if is_dir && menu_item(ui, t("cmd.paste_into"), "", can_paste) {
        actions.push(Action::PasteInto(row));
    }
    ui.separator();
    if menu_item(ui, t("cmd.rename"), "F2", selected == 1) {
        actions.push(Action::Rename);
    }
    if menu_item(ui, t("cmd.delete"), "Del", true) {
        actions.push(Action::Delete { permanent: false });
    }
    ui.separator();
    if menu_item(ui, t("cmd.copy_path"), "", true) {
        actions.push(Action::CopyPath);
    }
    if menu_item(ui, t("cmd.properties"), "Alt+Enter", selected == 1) {
        actions.push(Action::Properties);
    }
}

fn background_menu(ui: &mut Ui, show_hidden: bool, can_paste: bool, actions: &mut Vec<Action>) {
    if menu_item(ui, t("cmd.new_folder"), "Ctrl+Shift+N", true) {
        actions.push(Action::NewFolder);
    }
    if menu_item(ui, t("cmd.paste"), "Ctrl+V", can_paste) {
        actions.push(Action::Paste);
    }
    if menu_item(ui, t("cmd.refresh"), "F5", true) {
        actions.push(Action::Refresh);
    }
    if menu_item(ui, t("cmd.select_all"), "Ctrl+A", true) {
        actions.push(Action::SelectAll);
    }
    ui.separator();
    let mut hidden = show_hidden;
    if ui
        .checkbox(&mut hidden, t("settings.show_hidden"))
        .changed()
    {
        actions.push(Action::SetShowHidden(hidden));
    }
    ui.separator();
    if menu_item(ui, t("cmd.settings"), "Ctrl+,", true) {
        actions.push(Action::OpenSettings);
    }
}
