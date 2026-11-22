//! The option registry: every option row of the form pages, and the controls they draw.

use super::{CHECK_INDENT, COMBO_WIDTH, Category, FIELD_WIDTH};
use crate::theme;
use eframe::egui::{self, RichText, Sense, TextEdit, Ui};
use fx_app::{
    ConflictChoice, DateFormat, Density, DragModifier, FolderOrder, GlobScopeChoice, Language,
    ListColumn, MAX_FONT_SIZE, MIN_FONT_SIZE, MIN_RESULT_CAP, PlaceGroup, Settings,
    open_with_system, t,
};

/// The worker override goes up to robocopy's `/MT` maximum.
const MAX_COPY_WORKERS: u32 = 128;
/// Upper bound of the result limit field; a list this long already holds every file of a large disk.
const MAX_RESULT_CAP: usize = 100_000_000;

/// Mutable state the option rows edit.
pub struct Form<'a> {
    pub(super) draft: &'a mut Settings,
    pub(super) start_text: &'a mut String,
    pub(super) current_folder: String,
}

/// How a row is laid out in the two-column form.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// Label in the left column, control in the right.
    Field,
    /// A checkbox carrying its own label, in the right column.
    Check,
}

pub(super) struct Row {
    pub(super) category: Category,
    /// Catalog key of the group heading the row sits under.
    pub(super) group: &'static str,
    /// Catalog key of the label.
    pub(super) label: &'static str,
    pub(super) kind: Kind,
    /// Draws the control (and any note under it). Field rows get the label for accessibility only.
    pub(super) show: fn(&mut Ui, &mut Form, &'static str),
}

pub(super) const ROWS: &[Row] = &[
    Row {
        category: Category::General,
        group: "settings.group.interface",
        label: "settings.language",
        kind: Kind::Field,
        show: |ui, f, _| language_row(ui, &mut f.draft.language),
    },
    Row {
        category: Category::General,
        group: "settings.group.startup",
        label: "settings.start_folder",
        kind: Kind::Field,
        show: |ui, f, _| start_folder_row(ui, f),
    },
    Row {
        category: Category::General,
        group: "settings.group.startup",
        label: "settings.restore_session",
        kind: Kind::Check,
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.restore_session, l);
        },
    },
    Row {
        category: Category::General,
        group: "settings.group.files",
        label: "settings.show_hidden",
        kind: Kind::Check,
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.show_hidden, l);
        },
    },
    Row {
        category: Category::Appearance,
        group: "settings.group.theme",
        label: "settings.theme",
        kind: Kind::Field,
        show: |ui, f, _| theme_row(ui, f.draft),
    },
    Row {
        category: Category::Appearance,
        group: "settings.group.theme",
        label: "settings.density",
        kind: Kind::Field,
        show: |ui, f, _| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                ui.radio_value(&mut f.draft.density, Density::Compact, t("density.compact"));
                ui.radio_value(
                    &mut f.draft.density,
                    Density::Comfortable,
                    t("density.comfortable"),
                );
            });
        },
    },
    Row {
        category: Category::Appearance,
        group: "settings.group.columns",
        label: "settings.ext_column",
        kind: Kind::Check,
        show: |ui, f, l| column_check(ui, f, l, ListColumn::Ext),
    },
    Row {
        category: Category::Appearance,
        group: "settings.group.columns",
        label: "settings.size_column",
        kind: Kind::Check,
        show: |ui, f, l| column_check(ui, f, l, ListColumn::Size),
    },
    Row {
        category: Category::Appearance,
        group: "settings.group.columns",
        label: "settings.modified_column",
        kind: Kind::Check,
        show: |ui, f, l| column_check(ui, f, l, ListColumn::Modified),
    },
    Row {
        category: Category::Appearance,
        group: "settings.group.columns",
        label: "settings.age_column",
        kind: Kind::Check,
        show: |ui, f, l| column_check(ui, f, l, ListColumn::Age),
    },
    // Only Windows maps network shares to drive letters.
    #[cfg(windows)]
    Row {
        category: Category::Appearance,
        group: "settings.group.sidebar",
        label: "settings.share_names",
        kind: Kind::Check,
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.show_share_names, l);
        },
    },
    Row {
        category: Category::Appearance,
        group: "settings.group.sidebar",
        label: "settings.show_places",
        kind: Kind::Check,
        show: |ui, f, l| {
            let mut shown = f.draft.sidebar_group_shown(PlaceGroup::Places);
            if ui.checkbox(&mut shown, l).changed() {
                f.draft.set_sidebar_group_shown(PlaceGroup::Places, shown);
            }
        },
    },
    Row {
        category: Category::Appearance,
        group: "settings.group.sidebar",
        label: "settings.show_bookmarks",
        kind: Kind::Check,
        show: |ui, f, l| {
            let mut shown = f.draft.sidebar_group_shown(PlaceGroup::Bookmarks);
            if ui.checkbox(&mut shown, l).changed() {
                f.draft
                    .set_sidebar_group_shown(PlaceGroup::Bookmarks, shown);
            }
        },
    },
    Row {
        category: Category::Appearance,
        group: "settings.group.sidebar",
        label: "settings.show_drives",
        kind: Kind::Check,
        show: |ui, f, l| {
            let mut shown = f.draft.sidebar_group_shown(PlaceGroup::Drives);
            if ui.checkbox(&mut shown, l).changed() {
                f.draft.set_sidebar_group_shown(PlaceGroup::Drives, shown);
            }
        },
    },
    #[cfg(windows)]
    Row {
        category: Category::Appearance,
        group: "settings.group.sidebar",
        label: "settings.show_wsl",
        kind: Kind::Check,
        show: |ui, f, l| {
            let mut shown = f.draft.sidebar_group_shown(PlaceGroup::Wsl);
            if ui.checkbox(&mut shown, l).changed() {
                f.draft.set_sidebar_group_shown(PlaceGroup::Wsl, shown);
            }
        },
    },
    Row {
        category: Category::Fonts,
        group: "settings.group.font",
        label: "settings.font_family",
        kind: Kind::Field,
        show: |ui, f, _| font_combo(ui, &mut f.draft.font_family),
    },
    Row {
        category: Category::Fonts,
        group: "settings.group.sizes",
        label: "settings.list_font_size",
        kind: Kind::Field,
        show: |ui, f, _| font_size(ui, &mut f.draft.list_font_size),
    },
    Row {
        category: Category::Fonts,
        group: "settings.group.sizes",
        label: "settings.tree_font_size",
        kind: Kind::Field,
        show: |ui, f, _| font_size(ui, &mut f.draft.tree_font_size),
    },
    Row {
        category: Category::Listing,
        group: "settings.group.sorting",
        label: "settings.folder_order",
        kind: Kind::Field,
        show: |ui, f, _| {
            combo(
                ui,
                "folder-order",
                &mut f.draft.folder_order,
                &[FolderOrder::Top, FolderOrder::Mixed, FolderOrder::Bottom],
                folder_order_text,
            );
        },
    },
    Row {
        category: Category::Listing,
        group: "settings.group.display",
        label: "settings.date_format",
        kind: Kind::Field,
        show: |ui, f, _| {
            combo(
                ui,
                "date-format",
                &mut f.draft.date_format,
                &[DateFormat::Locale, DateFormat::Iso],
                date_format_text,
            );
        },
    },
    Row {
        category: Category::Listing,
        group: "settings.group.performance",
        label: "settings.preload",
        kind: Kind::Check,
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.preload_subfolders, l);
        },
    },
    Row {
        category: Category::Behavior,
        group: "settings.group.clicks",
        label: "settings.tree_single_click",
        kind: Kind::Check,
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.tree_single_click, l);
        },
    },
    Row {
        category: Category::Behavior,
        group: "settings.group.clicks",
        label: "settings.list_single_click",
        kind: Kind::Check,
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.list_single_click, l);
        },
    },
    Row {
        category: Category::Behavior,
        group: "settings.group.drag_drop",
        label: "settings.drag_copy_modifier",
        kind: Kind::Field,
        show: |ui, f, _| {
            combo(
                ui,
                "drag-copy-modifier",
                &mut f.draft.drag_copy_modifier,
                &DragModifier::ALL,
                DragModifier::label,
            );
            note(ui, t("settings.drag_copy_modifier_note"), 0.0);
        },
    },
    Row {
        category: Category::Behavior,
        group: "settings.group.deleting",
        label: "settings.use_trash",
        kind: Kind::Check,
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.use_trash, l);
        },
    },
    Row {
        category: Category::Behavior,
        group: "settings.group.deleting",
        label: "settings.confirm_delete",
        kind: Kind::Check,
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.confirm_delete, l);
            if !f.draft.use_trash {
                note(ui, t("settings.confirm_delete_note"), CHECK_INDENT);
            }
        },
    },
    Row {
        category: Category::Transfers,
        group: "settings.group.conflicts",
        label: "settings.conflict_policy",
        kind: Kind::Field,
        show: |ui, f, _| {
            combo(
                ui,
                "conflict-policy",
                &mut f.draft.conflict_policy,
                &ConflictChoice::ALL,
                ConflictChoice::label,
            );
        },
    },
    Row {
        category: Category::Transfers,
        group: "settings.group.performance",
        label: "settings.copy_workers",
        kind: Kind::Field,
        show: |ui, f, _| copy_workers_row(ui, &mut f.draft.copy_workers),
    },
    Row {
        category: Category::Transfers,
        group: "settings.group.safety",
        label: "settings.verify_copies",
        kind: Kind::Check,
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.verify_copies, l);
            note(ui, t("settings.verify_copies_note"), CHECK_INDENT);
        },
    },
    Row {
        category: Category::Search,
        group: "settings.group.search",
        label: "settings.search_result_cap",
        kind: Kind::Field,
        show: |ui, f, _| result_cap_row(ui, &mut f.draft.search.result_cap),
    },
    Row {
        category: Category::Search,
        group: "settings.group.search",
        label: "settings.search_glob_scope",
        kind: Kind::Field,
        show: |ui, f, _| {
            combo(
                ui,
                "search-glob-scope",
                &mut f.draft.search.glob_scope,
                &[GlobScopeChoice::Name, GlobScopeChoice::RelativePath],
                glob_scope_text,
            );
            note(ui, t("settings.search_glob_scope_note"), 0.0);
        },
    },
    Row {
        category: Category::Search,
        group: "settings.group.search",
        label: "settings.search_warn_root",
        kind: Kind::Check,
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.search.warn_root, l);
        },
    },
];

fn glob_scope_text(scope: GlobScopeChoice) -> &'static str {
    match scope {
        GlobScopeChoice::Name => t("search.scope.name"),
        GlobScopeChoice::RelativePath => t("search.scope.relative_path"),
    }
}

fn result_cap_row(ui: &mut Ui, cap: &mut usize) {
    ui.add(
        egui::DragValue::new(cap)
            .range(MIN_RESULT_CAP..=MAX_RESULT_CAP)
            .speed(100.0),
    );
    note(ui, t("settings.search_result_cap_note"), 0.0);
}

/// A combo box of `choices`, each shown as `text(choice)`.
fn combo<T: Copy + PartialEq>(
    ui: &mut Ui,
    id_salt: &str,
    value: &mut T,
    choices: &[T],
    text: impl Fn(T) -> &'static str,
) {
    egui::ComboBox::from_id_salt(id_salt)
        .width(COMBO_WIDTH)
        .selected_text(text(*value))
        .show_ui(ui, |ui| {
            for &choice in choices {
                ui.selectable_value(value, choice, text(choice));
            }
        });
}

fn folder_order_text(order: FolderOrder) -> &'static str {
    match order {
        FolderOrder::Top => t("folder_order.top"),
        FolderOrder::Mixed => t("folder_order.mixed"),
        FolderOrder::Bottom => t("folder_order.bottom"),
    }
}

fn date_format_text(format: DateFormat) -> &'static str {
    match format {
        DateFormat::Locale => t("date_format.locale"),
        DateFormat::Iso => t("date_format.iso"),
    }
}

/// Each language is listed under its own name, so it stays findable when the UI language is unreadable.
fn language_name(language: Language) -> &'static str {
    match language {
        Language::System => t("language.system"),
        other => other.native_name(),
    }
}

/// The language combo, with a warning when the language needs a CJK font that is missing.
fn language_row(ui: &mut Ui, language: &mut Language) {
    combo(ui, "language", language, &Language::ALL, language_name);
    if !language.needs_cjk() || theme::cjk_font_found() {
        return;
    }
    ui.add(
        egui::Label::new(
            RichText::new(t("settings.cjk_missing"))
                .small()
                .color(theme::error()),
        )
        .wrap(),
    );
}

fn start_folder_row(ui: &mut Ui, f: &mut Form) {
    ui.add(
        TextEdit::singleline(f.start_text)
            .hint_text(t("settings.home_folder"))
            .desired_width(FIELD_WIDTH),
    );
    if ui.button(t("settings.use_current")).clicked() {
        *f.start_text = f.current_folder.clone();
    }
}

/// The theme picker with the buttons that open and reload the user theme folder.
fn theme_row(ui: &mut Ui, draft: &mut Settings) {
    theme_picker(ui, &mut draft.theme);
    ui.horizontal(|ui| {
        if ui.button(t("settings.open_themes")).clicked() {
            open_themes_dir();
        }
        if ui.button(t("settings.reload_themes")).clicked() {
            theme::reload();
            theme::apply(ui.ctx(), draft);
        }
    });
}

/// Creates the user theme folder if needed and opens it in the system file manager.
fn open_themes_dir() {
    let Some(dir) = Settings::themes_dir() else {
        return;
    };
    if let Err(err) = std::fs::create_dir_all(&dir).and_then(|()| open_with_system(&dir)) {
        eprintln!("fx: cannot open {}: {err}", dir.display());
    }
}

fn copy_workers_row(ui: &mut Ui, workers: &mut u32) {
    ui.add(
        egui::DragValue::new(workers)
            .range(0..=MAX_COPY_WORKERS)
            .custom_formatter(|n, _| {
                if n == 0.0 {
                    t("settings.copy_workers_auto").to_owned()
                } else {
                    format!("{n}")
                }
            }),
    );
    note(ui, t("settings.copy_workers_note"), 0.0);
}

/// The five swatch colours of a theme palette.
fn swatch_colors(p: &theme::Palette) -> [egui::Color32; 5] {
    [p.bg, p.panel_hi, p.text, p.accent, p.error]
}

/// Five swatches of a theme palette.
fn swatches(ui: &mut Ui, p: &theme::Palette) {
    for c in swatch_colors(p) {
        let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 14.0), Sense::hover());
        ui.painter().rect_filled(r, 2.0, c);
    }
}

/// Combo box of all themes, each with a strip of swatches (also on the selected entry).
fn theme_picker(ui: &mut Ui, id: &mut String) {
    let themes = theme::themes();
    let selected = themes.iter().find(|t| t.id == *id);
    let name = selected.map_or_else(|| id.clone(), |th| theme::display_name(th));
    // The selected text leaves room for the swatch strip, painted over the button afterwards.
    let strip = if selected.is_some() {
        5.0 * 12.0 + 4.0
    } else {
        0.0
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &name,
        strip,
        egui::TextFormat::simple(egui::TextStyle::Body.resolve(ui.style()), theme::text()),
    );
    let combo = egui::ComboBox::from_id_salt("theme")
        .width(COMBO_WIDTH)
        .selected_text(job)
        .show_ui(ui, |ui| {
            for t in &themes {
                theme_option(ui, id, t);
            }
        });
    if let Some(th) = selected {
        paint_swatch_strip(ui, combo.response.rect, &th.palette);
    }
}

/// One entry of the theme combo: its swatches and its name.
fn theme_option(ui: &mut Ui, id: &mut String, t: &theme::Theme) {
    ui.horizontal(|ui| {
        swatches(ui, &t.palette);
        ui.selectable_value(id, t.id.clone(), theme::display_name(t));
    });
}

/// Paints the swatches of `p` over the left end of the combo button `r`.
fn paint_swatch_strip(ui: &Ui, r: egui::Rect, p: &theme::Palette) {
    let x = r.left() + ui.spacing().button_padding.x;
    for (i, c) in swatch_colors(p).into_iter().enumerate() {
        let min = egui::pos2(x + i as f32 * 12.0, r.center().y - 7.0);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(min, egui::vec2(10.0, 14.0)),
            2.0,
            c,
        );
    }
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
        .width(COMBO_WIDTH)
        .selected_text(family.as_deref().unwrap_or(t("settings.font_default")))
        .height(300.0)
        .show_ui(ui, |ui| font_list(ui, family, filter_id));
}

/// The font combo's popup: a filter field and the matching system fonts.
fn font_list(ui: &mut Ui, family: &mut Option<String>, filter_id: egui::Id) {
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
}

/// A dim help text under a control, `indent` px in.
pub(super) fn note(ui: &mut Ui, text: &str, indent: f32) {
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: indent as i8,
            ..egui::Margin::ZERO
        })
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text).small().color(theme::dim())).wrap());
        });
}

/// A "show this column" checkbox; hiding only edits the draft, like every other row.
fn column_check(ui: &mut Ui, f: &mut Form, label: &str, column: ListColumn) {
    let mut shown = f.draft.column_shown(column);
    if ui.checkbox(&mut shown, label).changed() {
        f.draft.set_column_shown(column, shown);
    }
}
