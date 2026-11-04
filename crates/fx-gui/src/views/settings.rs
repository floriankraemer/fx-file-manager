//! IDE-style settings dialog: searchable category list on the left, option rows on the right.
//!
//! Every option is declared once in [`ROWS`]; pages and search both derive from that registry.

use crate::theme;
use eframe::egui::{
    self, Align, Context, Id, Key, Layout, Modal, Modifiers, RichText, ScrollArea, Sense, TextEdit,
    Ui,
};
use fx_app::{
    ConflictChoice, Density, Explorer, FolderOrder, Language, MAX_FONT_SIZE, MIN_FONT_SIZE,
    Settings, open_with_system, t,
};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    General,
    Appearance,
    Fonts,
    Listing,
    Behavior,
    Transfers,
}

/// The worker override goes up to robocopy's `/MT` maximum.
const MAX_COPY_WORKERS: u32 = 128;

impl Category {
    pub const ALL: [Category; 6] = [
        Category::General,
        Category::Appearance,
        Category::Fonts,
        Category::Listing,
        Category::Behavior,
        Category::Transfers,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::General => t("settings.cat.general"),
            Category::Appearance => t("settings.cat.appearance"),
            Category::Fonts => t("settings.cat.fonts"),
            Category::Listing => t("settings.cat.listing"),
            Category::Behavior => t("settings.cat.behavior"),
            Category::Transfers => t("settings.cat.transfers"),
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
    /// Catalog key of the label.
    label: &'static str,
    show: fn(&mut Ui, &mut Form, &'static str),
}

const ROWS: &[Row] = &[
    Row {
        category: Category::General,
        label: "settings.start_folder",
        show: |ui, f, l| {
            ui.label(l);
            ui.add(
                TextEdit::singleline(f.start_text)
                    .hint_text(t("settings.home_folder"))
                    .desired_width(220.0),
            );
            if ui.button(t("settings.use_current")).clicked() {
                *f.start_text = f.current_folder.clone();
            }
        },
    },
    Row {
        category: Category::General,
        label: "settings.show_hidden",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.show_hidden, l);
        },
    },
    Row {
        category: Category::General,
        label: "settings.language",
        show: |ui, f, l| {
            ui.label(l);
            language_combo(ui, &mut f.draft.language);
        },
    },
    Row {
        category: Category::Appearance,
        label: "settings.theme",
        show: |ui, f, l| {
            ui.label(l);
            theme_picker(ui, &mut f.draft.theme);
            if ui.button(t("settings.open_themes")).clicked()
                && let Some(dir) = Settings::themes_dir()
                && let Err(err) =
                    std::fs::create_dir_all(&dir).and_then(|()| open_with_system(&dir))
            {
                eprintln!("fx: cannot open {}: {err}", dir.display());
            }
            if ui.button(t("settings.reload_themes")).clicked() {
                theme::reload();
                theme::apply(ui.ctx(), f.draft);
            }
        },
    },
    Row {
        category: Category::Appearance,
        label: "settings.density",
        show: |ui, f, l| {
            ui.label(l);
            ui.radio_value(&mut f.draft.density, Density::Compact, t("density.compact"));
            ui.radio_value(
                &mut f.draft.density,
                Density::Comfortable,
                t("density.comfortable"),
            );
        },
    },
    Row {
        category: Category::Appearance,
        label: "settings.ext_column",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.show_ext_column, l);
        },
    },
    Row {
        category: Category::Appearance,
        label: "settings.age_column",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.show_age_column, l);
        },
    },
    Row {
        category: Category::Fonts,
        label: "settings.font_family",
        show: |ui, f, l| {
            ui.label(l);
            font_combo(ui, &mut f.draft.font_family);
        },
    },
    Row {
        category: Category::Fonts,
        label: "settings.list_font_size",
        show: |ui, f, l| {
            ui.label(l);
            font_size(ui, &mut f.draft.list_font_size);
        },
    },
    Row {
        category: Category::Fonts,
        label: "settings.tree_font_size",
        show: |ui, f, l| {
            ui.label(l);
            font_size(ui, &mut f.draft.tree_font_size);
        },
    },
    Row {
        category: Category::Listing,
        label: "settings.folder_order",
        show: |ui, f, l| {
            ui.label(l);
            let o = &mut f.draft.folder_order;
            let text = |o: FolderOrder| match o {
                FolderOrder::Top => t("folder_order.top"),
                FolderOrder::Mixed => t("folder_order.mixed"),
                FolderOrder::Bottom => t("folder_order.bottom"),
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
        label: "settings.preload",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.preload_subfolders, l);
        },
    },
    Row {
        category: Category::Behavior,
        label: "settings.tree_single_click",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.tree_single_click, l);
        },
    },
    Row {
        category: Category::Behavior,
        label: "settings.list_single_click",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.list_single_click, l);
        },
    },
    Row {
        category: Category::Behavior,
        label: "settings.use_trash",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.use_trash, l);
        },
    },
    Row {
        category: Category::Behavior,
        label: "settings.confirm_delete",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.confirm_delete, l);
            if !f.draft.use_trash {
                ui.label(
                    RichText::new(t("settings.confirm_delete_note"))
                        .small()
                        .color(theme::dim()),
                );
            }
        },
    },
    Row {
        category: Category::Transfers,
        label: "settings.conflict_policy",
        show: |ui, f, l| {
            ui.label(l);
            let policy = &mut f.draft.conflict_policy;
            egui::ComboBox::from_id_salt("conflict-policy")
                .selected_text(policy.label())
                .show_ui(ui, |ui| {
                    for choice in ConflictChoice::ALL {
                        ui.selectable_value(policy, choice, choice.label());
                    }
                });
        },
    },
    Row {
        category: Category::Transfers,
        label: "settings.copy_workers",
        show: |ui, f, l| {
            ui.label(l);
            ui.add(
                egui::DragValue::new(&mut f.draft.copy_workers)
                    .range(0..=MAX_COPY_WORKERS)
                    .custom_formatter(|n, _| {
                        if n == 0.0 {
                            t("settings.copy_workers_auto").to_owned()
                        } else {
                            format!("{n}")
                        }
                    }),
            );
            ui.label(
                RichText::new(t("settings.copy_workers_note"))
                    .small()
                    .color(theme::dim()),
            );
        },
    },
    Row {
        category: Category::Transfers,
        label: "settings.verify_copies",
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.verify_copies, l);
            ui.label(
                RichText::new(t("settings.verify_copies_note"))
                    .small()
                    .color(theme::dim()),
            );
        },
    },
];

/// Each language is listed under its own name, so it stays findable when the UI language is unreadable.
fn language_combo(ui: &mut Ui, language: &mut Language) {
    let name = |l: Language| match l {
        Language::System => t("language.system"),
        other => other.native_name(),
    };
    egui::ComboBox::from_id_salt("language")
        .selected_text(name(*language))
        .show_ui(ui, |ui| {
            for l in Language::ALL {
                ui.selectable_value(language, l, name(l));
            }
        });
}

/// Combo box of all themes, each with a strip of swatches.
fn theme_picker(ui: &mut Ui, id: &mut String) {
    let themes = theme::themes();
    let selected = themes
        .iter()
        .find(|t| t.id == *id)
        .map_or_else(|| id.clone(), |th| theme::display_name(th));
    egui::ComboBox::from_id_salt("theme")
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for t in &themes {
                ui.horizontal(|ui| {
                    let p = &t.palette;
                    for c in [p.bg, p.panel_hi, p.text, p.accent, p.error] {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 14.0), Sense::hover());
                        ui.painter().rect_filled(r, 2.0, c);
                    }
                    ui.selectable_value(id, t.id.clone(), theme::display_name(t));
                });
            }
        });
}

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
        .selected_text(family.as_deref().unwrap_or(t("settings.font_default")))
        .height(300.0)
        .show_ui(ui, |ui| {
            let mut filter: String = ui.data_mut(|d| d.get_temp(filter_id).unwrap_or_default());
            ui.add(TextEdit::singleline(&mut filter).hint_text(t("settings.font_filter")));
            let needle = filter.to_lowercase();
            ui.data_mut(|d| d.insert_temp(filter_id, filter));
            if needle.is_empty() {
                ui.selectable_value(family, None, t("settings.font_default"));
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
            t(r.label).to_lowercase().contains(&q)
                || r.category.label().to_lowercase().contains(&q)
                || (r.label == "settings.theme"
                    && ["color", "colour", "dark", "light"].contains(&q.as_str()))
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
        ui.heading(t("settings.title"));
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
                            .hint_text(t("settings.search"))
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
                        ui.label(RichText::new(t("settings.none")).color(theme::dim()));
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
                        ui.horizontal_wrapped(|ui| (r.show)(ui, &mut form, t(r.label)));
                    }
                });
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(t("button.apply")).clicked() {
                apply = true;
            }
            if ui.button(t("button.cancel")).clicked() {
                done = true;
                theme::apply(ctx, explorer.settings());
            }
            if ui.button(t("button.ok")).clicked() {
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
    fn row_labels_are_catalog_keys() {
        for r in ROWS {
            assert_ne!(t(r.label), r.label, "missing text for {}", r.label);
        }
        for c in Category::ALL {
            assert!(!c.label().starts_with("settings."), "{c:?}");
        }
    }

    #[test]
    fn every_category_has_rows() {
        for c in Category::ALL {
            assert!(ROWS.iter().any(|r| r.category == c));
        }
    }
}
