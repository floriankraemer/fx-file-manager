//! IDE-style settings dialog: searchable category list on the left, option rows on the right.
//!
//! Every option is declared once in [`ROWS`]; pages and search both derive from that registry.

use crate::theme;
use eframe::egui::{
    self, Align, Context, Id, Key, Layout, Modal, Modifiers, RichText, ScrollArea, TextEdit, Ui,
};
use fx_app::{Density, Explorer, FolderOrder, MAX_FONT_SIZE, MIN_FONT_SIZE, Settings, ThemeChoice};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    General,
    Appearance,
    Fonts,
    Listing,
    Behavior,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::General,
        Category::Appearance,
        Category::Fonts,
        Category::Listing,
        Category::Behavior,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::General => "General",
            Category::Appearance => "Appearance",
            Category::Fonts => "Fonts",
            Category::Listing => "Listing",
            Category::Behavior => "Behavior",
        }
    }
}

/// Mutable state the option rows edit.
pub struct Form<'a> {
    draft: &'a mut Settings,
    start_text: &'a mut String,
    current_folder: String,
}

struct Row {
    category: Category,
    label: &'static str,
    show: fn(&mut Ui, &mut Form, &'static str),
}

const ROWS: &[Row] = &[
    Row {
        category: Category::General,
        label: "Start folder",
        show: |ui, f, l| {
            ui.label(l);
            ui.add(
                TextEdit::singleline(f.start_text)
                    .hint_text("Home folder")
                    .desired_width(220.0),
            );
            if ui.button("Use current folder").clicked() {
                *f.start_text = f.current_folder.clone();
            }
        },
    },
    Row {
        category: Category::General,
        label: "Show hidden items",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.show_hidden, l);
        },
    },
    Row {
        category: Category::Appearance,
        label: "Theme",
        show: |ui, f, l| {
            ui.label(l);
            ui.radio_value(&mut f.draft.theme, ThemeChoice::Dark, "Dark");
            ui.radio_value(&mut f.draft.theme, ThemeChoice::Light, "Light");
        },
    },
    Row {
        category: Category::Appearance,
        label: "Density",
        show: |ui, f, l| {
            ui.label(l);
            ui.radio_value(&mut f.draft.density, Density::Compact, "Compact");
            ui.radio_value(&mut f.draft.density, Density::Comfortable, "Comfortable");
        },
    },
    Row {
        category: Category::Appearance,
        label: "Show Ext column",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.show_ext_column, l);
        },
    },
    Row {
        category: Category::Appearance,
        label: "Show Age column",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.show_age_column, l);
        },
    },
    Row {
        category: Category::Fonts,
        label: "Font family",
        show: |ui, f, l| {
            ui.label(l);
            font_combo(ui, &mut f.draft.font_family);
        },
    },
    Row {
        category: Category::Fonts,
        label: "List font size",
        show: |ui, f, l| {
            ui.label(l);
            font_size(ui, &mut f.draft.list_font_size);
        },
    },
    Row {
        category: Category::Fonts,
        label: "Tree font size",
        show: |ui, f, l| {
            ui.label(l);
            font_size(ui, &mut f.draft.tree_font_size);
        },
    },
    Row {
        category: Category::Listing,
        label: "Folder order",
        show: |ui, f, l| {
            ui.label(l);
            let o = &mut f.draft.folder_order;
            let text = |o: FolderOrder| match o {
                FolderOrder::Top => "Folders on top",
                FolderOrder::Mixed => "Folders mixed with files",
                FolderOrder::Bottom => "Folders on bottom",
            };
            egui::ComboBox::from_id_salt("folder-order")
                .selected_text(text(*o))
                .show_ui(ui, |ui| {
                    for v in [FolderOrder::Top, FolderOrder::Mixed, FolderOrder::Bottom] {
                        ui.selectable_value(o, v, text(v));
                    }
                });
        },
    },
    Row {
        category: Category::Listing,
        label: "Preload subfolders in the background",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.preload_subfolders, l);
        },
    },
    Row {
        category: Category::Behavior,
        label: "Single click in the folder tree",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.tree_single_click, l);
        },
    },
    Row {
        category: Category::Behavior,
        label: "Single click in the file list",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.list_single_click, l);
        },
    },
    Row {
        category: Category::Behavior,
        label: "Move deleted items to Trash",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.use_trash, l);
        },
    },
    Row {
        category: Category::Behavior,
        label: "Confirm before deleting",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.confirm_delete, l);
            if !f.draft.use_trash {
                ui.label(
                    RichText::new("Without Trash, deleting is permanent and always asks first.")
                        .small()
                        .color(theme::dim()),
                );
            }
        },
    },
];

fn font_size(ui: &mut Ui, size: &mut f32) {
    ui.add(
        egui::DragValue::new(size)
            .range(MIN_FONT_SIZE..=MAX_FONT_SIZE)
            .speed(0.1)
            .suffix(" px"),
    );
}

fn font_combo(ui: &mut Ui, family: &mut Option<String>) {
    let filter_id = ui.scope_id().with("font-filter");
    egui::ComboBox::from_id_salt("font-family")
        .selected_text(family.as_deref().unwrap_or("Default"))
        .height(300.0)
        .show_ui(ui, |ui| {
            let mut filter: String = ui.data_mut(|d| d.get_temp(filter_id).unwrap_or_default());
            ui.add(TextEdit::singleline(&mut filter).hint_text("Filter fonts"));
            let needle = filter.to_lowercase();
            ui.data_mut(|d| d.insert_temp(filter_id, filter));
            if needle.is_empty() {
                ui.selectable_value(family, None, "Default");
            }
            for name in theme::system_font_families() {
                if name.to_lowercase().contains(&needle) {
                    ui.selectable_value(family, Some(name.clone()), name);
                }
            }
        });
}

/// Rows whose label or category label contains `query` (case-insensitive substring).
fn search(query: &str) -> Vec<&'static Row> {
    let q = query.trim().to_lowercase();
    ROWS.iter()
        .filter(|r| {
            r.label.to_lowercase().contains(&q) || r.category.label().to_lowercase().contains(&q)
        })
        .collect()
}

pub struct State {
    pub draft: Settings,
    pub start_text: String,
    pub category: Category,
    pub query: String,
    /// What the live preview last applied.
    previewed: Settings,
}

impl State {
    pub fn new(saved: &Settings) -> Self {
        Self {
            draft: saved.clone(),
            start_text: saved
                .start_folder
                .as_ref()
                .map_or_else(String::new, |p| p.display().to_string()),
            category: Category::General,
            query: String::new(),
            previewed: saved.clone(),
        }
    }
}

fn look_changed(a: &Settings, b: &Settings) -> bool {
    a.theme != b.theme
        || a.font_family != b.font_family
        || a.list_font_size != b.list_font_size
        || a.tree_font_size != b.tree_font_size
}

/// Draws the dialog. Returns true when it should be closed.
pub fn show(ctx: &Context, explorer: &mut Explorer, st: &mut State) -> bool {
    // Esc first clears a non-empty search instead of closing.
    let esc_clears =
        !st.query.is_empty() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    if esc_clears {
        st.query.clear();
    }
    if look_changed(&st.draft, &st.previewed) {
        theme::apply(ctx, &st.draft);
        st.previewed = st.draft.clone();
    }

    let current_folder = explorer.active().path().display().to_string();
    let mut done = false;
    let mut apply = false;
    let response = Modal::new(Id::unique("settings-dialog")).show(ctx, |ui| {
        ui.set_width(760.0);
        ui.set_height(520.0);
        ui.heading("Settings");
        ui.add_space(6.0);
        let body = ui.available_height() - 40.0;
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(180.0, body),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(180.0);
                    ui.add(
                        TextEdit::singleline(&mut st.query)
                            .hint_text("Search settings")
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(6.0);
                    for c in Category::ALL {
                        if ui
                            .selectable_label(st.category == c && st.query.is_empty(), c.label())
                            .clicked()
                        {
                            st.category = c;
                            st.query.clear();
                        }
                    }
                },
            );
            ui.separator();
            ScrollArea::vertical()
                .id_salt("settings-page")
                .max_height(body)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut form = Form {
                        draft: &mut st.draft,
                        start_text: &mut st.start_text,
                        current_folder,
                    };
                    let rows: Vec<&Row> = if st.query.trim().is_empty() {
                        ROWS.iter().filter(|r| r.category == st.category).collect()
                    } else {
                        search(&st.query)
                    };
                    if rows.is_empty() {
                        ui.label(RichText::new("No matching settings").color(theme::dim()));
                    }
                    let mut last = None;
                    for r in rows {
                        if !st.query.trim().is_empty() && last != Some(r.category) {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(r.category.label())
                                    .strong()
                                    .color(theme::accent()),
                            );
                            ui.separator();
                        }
                        last = Some(r.category);
                        ui.horizontal_wrapped(|ui| (r.show)(ui, &mut form, r.label));
                    }
                });
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Apply").clicked() {
                apply = true;
            }
            if ui.button("Cancel").clicked() {
                done = true;
                theme::apply(ctx, explorer.settings());
            }
            if ui.button("OK").clicked() {
                apply = true;
                done = true;
            }
        });
    });
    if response.should_close() && !esc_clears && !done {
        done = true;
        theme::apply(ctx, explorer.settings());
    }
    if apply {
        let folder = st.start_text.trim();
        st.draft.start_folder = (!folder.is_empty()).then(|| PathBuf::from(folder));
        explorer.set_settings(st.draft.clone());
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_label_or_category_case_insensitively() {
        assert_eq!(search("").len(), ROWS.len());
        assert_eq!(search("THEME").len(), 1);
        assert_eq!(search("fonts").len(), 3);
        assert!(search("zzz").is_empty());
    }

    #[test]
    fn every_category_has_rows() {
        for c in Category::ALL {
            assert!(ROWS.iter().any(|r| r.category == c));
        }
    }
}
