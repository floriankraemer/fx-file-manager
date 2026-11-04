//! IDE-style settings dialog: searchable category list on the left, option rows on the right.
//!
//! Every option is declared once in [`ROWS`]; pages and search both derive from that registry.

use crate::command::{self, COMMANDS, Command, Def, Shortcut};
use crate::theme;
use eframe::egui::{
    self, Align, Context, Event, Id, Key, Layout, Modal, Modifiers, RichText, ScrollArea, Sense,
    TextEdit, Ui,
};
use fx_app::{
    ConflictChoice, DateFormat, Density, Explorer, FolderOrder, Language, MAX_FONT_SIZE,
    MIN_FONT_SIZE, Settings, open_with_system, t,
};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    General,
    Appearance,
    Fonts,
    Listing,
    Behavior,
    Transfers,
    Keyboard,
}

/// Layout metrics, matching the IDE's settings dialog (4/8/12/16 px spacing, 26 px controls).
const DIALOG_SIZE: egui::Vec2 = egui::vec2(880.0, 600.0);
const SIDEBAR_WIDTH: f32 = 200.0;
const SIDEBAR_ROW: f32 = 27.0;
const CONTROL_HEIGHT: f32 = 26.0;
const COLUMN_GAP: f32 = 16.0;
const FIELD_WIDTH: f32 = 320.0;
const COMBO_WIDTH: f32 = 240.0;
const BUTTON_MIN_WIDTH: f32 = 84.0;

/// The worker override goes up to robocopy's `/MT` maximum.
const MAX_COPY_WORKERS: u32 = 128;

impl Category {
    pub const ALL: [Category; 7] = [
        Category::General,
        Category::Appearance,
        Category::Fonts,
        Category::Listing,
        Category::Behavior,
        Category::Transfers,
        Category::Keyboard,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::General => t("settings.cat.general"),
            Category::Appearance => t("settings.cat.appearance"),
            Category::Fonts => t("settings.cat.fonts"),
            Category::Listing => t("settings.cat.listing"),
            Category::Behavior => t("settings.cat.behavior"),
            Category::Transfers => t("settings.cat.transfers"),
            Category::Keyboard => t("settings.cat.keyboard"),
        }
    }
}

/// Mutable state the option rows edit.
pub struct Form<'a> {
    draft: &'a mut Settings,
    start_text: &'a mut String,
    current_folder: String,
}

/// How a row is laid out in the two-column form.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Label in the left column, control in the right.
    Field,
    /// A checkbox carrying its own label, in the right column.
    Check,
}

struct Row {
    category: Category,
    /// Catalog key of the group heading the row sits under.
    group: &'static str,
    /// Catalog key of the label.
    label: &'static str,
    kind: Kind,
    /// Draws the control (and any note under it). Field rows get the label for accessibility only.
    show: fn(&mut Ui, &mut Form, &'static str),
}

/// A dim help text under a control.
fn note(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).small().color(theme::dim())).wrap());
}

const ROWS: &[Row] = &[
    Row {
        category: Category::General,
        group: "settings.group.interface",
        label: "settings.language",
        kind: Kind::Field,
        show: |ui, f, _| {
            language_combo(ui, &mut f.draft.language);
            if f.draft.language.needs_cjk() && !theme::cjk_font_found() {
                ui.add(
                    egui::Label::new(
                        RichText::new(t("settings.cjk_missing"))
                            .small()
                            .color(theme::error()),
                    )
                    .wrap(),
                );
            }
        },
    },
    Row {
        category: Category::General,
        group: "settings.group.startup",
        label: "settings.start_folder",
        kind: Kind::Field,
        show: |ui, f, _| {
            ui.add(
                TextEdit::singleline(f.start_text)
                    .hint_text(t("settings.home_folder"))
                    .desired_width(FIELD_WIDTH),
            );
            if ui.button(t("settings.use_current")).clicked() {
                *f.start_text = f.current_folder.clone();
            }
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
        show: |ui, f, _| {
            theme_picker(ui, &mut f.draft.theme);
            ui.horizontal(|ui| {
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
            });
        },
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
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.show_ext_column, l);
        },
    },
    Row {
        category: Category::Appearance,
        group: "settings.group.columns",
        label: "settings.age_column",
        kind: Kind::Check,
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.show_age_column, l);
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
            let o = &mut f.draft.folder_order;
            let text = |o: FolderOrder| match o {
                FolderOrder::Top => t("folder_order.top"),
                FolderOrder::Mixed => t("folder_order.mixed"),
                FolderOrder::Bottom => t("folder_order.bottom"),
            };
            egui::ComboBox::from_id_salt("folder-order")
                .width(COMBO_WIDTH)
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
        group: "settings.group.display",
        label: "settings.date_format",
        kind: Kind::Field,
        show: |ui, f, _| {
            let d = &mut f.draft.date_format;
            let text = |d: DateFormat| match d {
                DateFormat::Locale => t("date_format.locale"),
                DateFormat::Iso => t("date_format.iso"),
            };
            egui::ComboBox::from_id_salt("date-format")
                .width(COMBO_WIDTH)
                .selected_text(text(*d))
                .show_ui(ui, |ui| {
                    for v in [DateFormat::Locale, DateFormat::Iso] {
                        ui.selectable_value(d, v, text(v));
                    }
                });
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
                note(ui, t("settings.confirm_delete_note"));
            }
        },
    },
    Row {
        category: Category::Transfers,
        group: "settings.group.conflicts",
        label: "settings.conflict_policy",
        kind: Kind::Field,
        show: |ui, f, _| {
            let policy = &mut f.draft.conflict_policy;
            egui::ComboBox::from_id_salt("conflict-policy")
                .width(COMBO_WIDTH)
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
        group: "settings.group.performance",
        label: "settings.copy_workers",
        kind: Kind::Field,
        show: |ui, f, _| {
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
            note(ui, t("settings.copy_workers_note"));
        },
    },
    Row {
        category: Category::Transfers,
        group: "settings.group.safety",
        label: "settings.verify_copies",
        kind: Kind::Check,
        show: |ui, f, l| {
            ui.checkbox(&mut f.draft.verify_copies, l);
            note(ui, t("settings.verify_copies_note"));
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
        .width(COMBO_WIDTH)
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
        .width(COMBO_WIDTH)
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
        .width(COMBO_WIDTH)
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

/// A key combination pressed this frame: `Some(None)` for a bare Esc (cancel). The key events are consumed.
fn capture(ctx: &Context) -> Option<Option<Shortcut>> {
    ctx.input_mut(|i| {
        let (key, m) = i.events.iter().find_map(|e| match e {
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => Some((*key, *modifiers)),
            _ => None,
        })?;
        i.events.retain(|e| !matches!(e, Event::Key { .. }));
        if key == Key::Escape && !(m.ctrl || m.shift || m.alt) {
            return Some(None);
        }
        let mut modifiers = Modifiers::NONE;
        modifiers.ctrl = m.ctrl;
        modifiers.shift = m.shift;
        modifiers.alt = m.alt;
        Some(Some(Shortcut { modifiers, key }))
    })
}

/// One grid row per command: title, current shortcuts (and a conflict warning), Record, Unbind and Reset.
fn keyboard_section(ui: &mut Ui, st: &mut State, commands: &[&Def]) {
    if let Some(command) = st.recording
        && let Some(captured) = capture(ui.ctx())
    {
        if let Some(shortcut) = captured {
            st.draft
                .keymap
                .insert(command.def().id.to_owned(), vec![shortcut.id()]);
        }
        st.recording = None;
    }
    egui::Grid::new("settings-keyboard")
        .num_columns(3)
        .striped(true)
        .spacing([COLUMN_GAP, 4.0])
        .min_row_height(CONTROL_HEIGHT)
        .show(ui, |ui| {
            for d in commands {
                let shortcuts = command::shortcuts(d.command, &st.draft.keymap);
                ui.label(t(d.title));
                ui.vertical(|ui| {
                    if st.recording == Some(d.command) {
                        ui.colored_label(theme::accent(), t("settings.keyboard_press"));
                    } else if shortcuts.is_empty() {
                        ui.label(RichText::new(t("settings.keyboard_none")).color(theme::dim()));
                    } else {
                        let text: Vec<String> = shortcuts.iter().map(Shortcut::display).collect();
                        ui.label(text.join(", "));
                    }
                    let others: Vec<&str> = command::conflicts(d.command, &st.draft.keymap)
                        .into_iter()
                        .map(|c| t(c.def().title))
                        .collect();
                    if !others.is_empty() {
                        ui.label(
                            RichText::new(fx_app::tr(
                                "settings.keyboard_also",
                                &[("others", &others.join(", "))],
                            ))
                            .small()
                            .color(theme::error()),
                        );
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button(t("settings.keyboard_record")).clicked() {
                        st.recording = Some(d.command);
                    }
                    if ui
                        .add_enabled(
                            !shortcuts.is_empty(),
                            egui::Button::new(t("settings.keyboard_unbind")),
                        )
                        .clicked()
                    {
                        st.draft.keymap.insert(d.id.to_owned(), Vec::new());
                    }
                    if ui
                        .add_enabled(
                            st.draft.keymap.contains_key(d.id),
                            egui::Button::new(t("settings.keyboard_reset")),
                        )
                        .clicked()
                    {
                        st.draft.keymap.remove(d.id);
                    }
                });
                ui.end_row();
            }
        });
}

/// A page or search-result title.
fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).heading().strong());
    ui.add_space(4.0);
}

/// A group heading with a rule under it, like a Qt group box title.
fn group_heading(ui: &mut Ui, text: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(text).strong().color(theme::accent()));
    ui.separator();
    ui.add_space(2.0);
}

/// Width of the label column: the widest field label of the rows shown, so every group lines up.
fn label_width(ui: &Ui, rows: &[&Row]) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    rows.iter()
        .filter(|r| r.kind == Kind::Field)
        .map(|r| {
            ui.painter()
                .layout_no_wrap(
                    t(r.label).to_owned(),
                    font.clone(),
                    egui::Color32::PLACEHOLDER,
                )
                .size()
                .x
        })
        .fold(120.0, f32::max)
}

/// Rows as a two-column form (label | control), split under group headings,
/// and under category titles too when `titles` is set (search results).
fn form(ui: &mut Ui, rows: &[&Row], form: &mut Form, titles: bool) {
    let label_w = label_width(ui, rows);
    let field_w = (ui.available_width() - label_w - COLUMN_GAP).max(FIELD_WIDTH);
    let mut start = 0;
    while start < rows.len() {
        let (category, group) = (rows[start].category, rows[start].group);
        let end = start
            + rows[start..]
                .iter()
                .take_while(|r| r.category == category && r.group == group)
                .count();
        if titles && (start == 0 || rows[start - 1].category != category) {
            if start > 0 {
                ui.add_space(12.0);
            }
            title(ui, category.label());
        }
        group_heading(ui, t(group));
        egui::Grid::new(("settings-form", category, group))
            .num_columns(2)
            .spacing([COLUMN_GAP, 8.0])
            .show(ui, |ui| {
                for r in &rows[start..end] {
                    let label = t(r.label);
                    ui.allocate_ui_with_layout(
                        egui::vec2(label_w, CONTROL_HEIGHT),
                        Layout::right_to_left(Align::Center),
                        |ui| {
                            ui.set_min_size(egui::vec2(label_w, CONTROL_HEIGHT));
                            if r.kind == Kind::Field {
                                ui.label(label);
                            }
                        },
                    );
                    ui.vertical(|ui| {
                        ui.set_max_width(field_w);
                        (r.show)(ui, form, label);
                    });
                    ui.end_row();
                }
            });
        start = end;
    }
}

/// A sidebar category entry: full-width 27 px row, rounded fill when selected or hovered.
fn category_row(ui: &mut Ui, label: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), SIDEBAR_ROW),
        Sense::click(),
    );
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, label));
    let fill = if selected {
        Some(theme::selection_bg())
    } else if resp.hovered() {
        Some(theme::panel_hi())
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, 6.0, fill);
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect,
            6.0,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().text(
        rect.left_center() + egui::vec2(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Body.resolve(ui.style()),
        theme::text(),
    );
    resp
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
    /// The command whose next key combination is being captured.
    recording: Option<Command>,
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
            recording: None,
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

/// The right-hand side: the selected category's page, or search results across all pages.
fn page(ui: &mut Ui, st: &mut State, current_folder: String) {
    let query = st.query.trim().to_lowercase();
    let searching = !query.is_empty();
    let rows: Vec<&Row> = if searching {
        search(&query)
    } else {
        ROWS.iter().filter(|r| r.category == st.category).collect()
    };
    let keyboard_page = !searching && st.category == Category::Keyboard;
    let commands: Vec<&Def> = COMMANDS
        .iter()
        .filter(|d| {
            keyboard_page
                || (searching
                    && (t(d.title).to_lowercase().contains(&query)
                        || Category::Keyboard.label().to_lowercase().contains(&query)))
        })
        .collect();
    if !searching {
        title(ui, st.category.label());
    }
    if rows.is_empty() && commands.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new(t("settings.none")).color(theme::dim()));
        return;
    }
    let mut f = Form {
        draft: &mut st.draft,
        start_text: &mut st.start_text,
        current_folder,
    };
    form(ui, &rows, &mut f, searching);
    if !commands.is_empty() {
        if searching {
            ui.add_space(12.0);
            title(ui, Category::Keyboard.label());
        } else {
            note(ui, t("settings.keyboard_note"));
        }
        ui.add_space(8.0);
        keyboard_section(ui, st, &commands);
    }
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
    let size = DIALOG_SIZE.min(ctx.content_rect().size() - egui::vec2(64.0, 64.0));
    let frame = egui::Frame::popup(&ctx.global_style()).inner_margin(16);
    let response = Modal::new(Id::unique("settings-dialog"))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(size.x);
            ui.set_height(size.y);
            let spacing = ui.spacing_mut();
            spacing.item_spacing = egui::vec2(8.0, 4.0);
            spacing.interact_size.y = CONTROL_HEIGHT;
            spacing.icon_spacing = 6.0;
            ui.heading(t("settings.title"));
            ui.add_space(8.0);
            // Footer: separator, 12 px gap, 26 px buttons.
            let body = ui.available_height() - (CONTROL_HEIGHT + 12.0 + 12.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(SIDEBAR_WIDTH, body),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(SIDEBAR_WIDTH);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add(
                            TextEdit::singleline(&mut st.query)
                                .hint_text(t("settings.search"))
                                .desired_width(f32::INFINITY)
                                .min_size(egui::vec2(0.0, CONTROL_HEIGHT))
                                .align(egui::Align2::LEFT_CENTER),
                        );
                        ui.add_space(6.0);
                        for c in Category::ALL {
                            let selected = st.category == c && st.query.is_empty();
                            if category_row(ui, c.label(), selected).clicked() {
                                st.category = c;
                                st.query.clear();
                            }
                        }
                    },
                );
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(8.0);
                ScrollArea::vertical()
                    .id_salt("settings-page")
                    .max_height(body)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_max_width(ui.available_width() - 12.0);
                        page(ui, st, current_folder);
                    });
            });
            ui.separator();
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let min = egui::vec2(BUTTON_MIN_WIDTH, CONTROL_HEIGHT);
                if ui
                    .add(egui::Button::new(t("button.apply")).min_size(min))
                    .clicked()
                {
                    apply = true;
                }
                if ui
                    .add(egui::Button::new(t("button.cancel")).min_size(min))
                    .clicked()
                {
                    done = true;
                    theme::apply(ctx, explorer.settings());
                }
                let ok = egui::Button::new(RichText::new(t("button.ok")).color(theme::on_accent()))
                    .fill(theme::accent())
                    .min_size(min);
                if ui.add(ok).clicked() {
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
            assert_ne!(t(r.group), r.group, "missing text for {}", r.group);
        }
        for c in Category::ALL {
            assert!(!c.label().starts_with("settings."), "{c:?}");
        }
    }

    #[test]
    fn groups_are_contiguous_within_a_category() {
        // `form` starts a new heading whenever the group changes, so a group split in two would show twice.
        let mut seen = Vec::new();
        for (i, r) in ROWS.iter().enumerate() {
            if i == 0 || ROWS[i - 1].category != r.category || ROWS[i - 1].group != r.group {
                assert!(!seen.contains(&(r.category, r.group)), "{} split", r.group);
                seen.push((r.category, r.group));
            }
        }
    }

    #[test]
    fn every_category_has_rows() {
        // The Keyboard page is generated from the command table, not from ROWS.
        for c in Category::ALL
            .into_iter()
            .filter(|c| *c != Category::Keyboard)
        {
            assert!(ROWS.iter().any(|r| r.category == c));
        }
    }
}
