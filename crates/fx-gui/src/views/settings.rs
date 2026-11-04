//! IDE-style settings dialog: searchable category list on the left, option rows on the right.
//!
//! Every option is declared once in [`ROWS`]; pages and search both derive from that registry.

use crate::command::{self, COMMANDS, Command, Def, Shortcut};
use crate::theme;
use eframe::egui::{
    self, Align, Context, Event, Key, Layout, Modifiers, RichText, ScrollArea, Sense, TextEdit, Ui,
    ViewportBuilder, ViewportClass, ViewportCommand, ViewportId,
};
use egui_extras::{Column, TableBuilder};
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
const SIDEBAR_WIDTH: f32 = 200.0;
const SIDEBAR_ROW: f32 = 26.0;
const CONTROL_HEIGHT: f32 = 26.0;
const GROUP_GAP: f32 = 12.0;
const ROW_GAP: f32 = 8.0;
const TABLE_ROW: f32 = 24.0;
/// Indent of a note under a checkbox: the box plus its gap.
const CHECK_INDENT: f32 = 24.0;
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

/// A dim help text under a control, `indent` px in.
fn note(ui: &mut Ui, text: &str, indent: f32) {
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: indent as i8,
            ..egui::Margin::ZERO
        })
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text).small().color(theme::dim())).wrap());
        });
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
            note(ui, t("settings.copy_workers_note"), 0.0);
        },
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

/// Five swatches of a theme palette.
fn swatches(ui: &mut Ui, p: &theme::Palette) {
    for c in [p.bg, p.panel_hi, p.text, p.accent, p.error] {
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
                ui.horizontal(|ui| {
                    swatches(ui, &t.palette);
                    ui.selectable_value(id, t.id.clone(), theme::display_name(t));
                });
            }
        });
    if let Some(th) = selected {
        let r = combo.response.rect;
        let p = &th.palette;
        let x = r.left() + ui.spacing().button_padding.x;
        for (i, c) in [p.bg, p.panel_hi, p.text, p.accent, p.error]
            .into_iter()
            .enumerate()
        {
            let min = egui::pos2(x + i as f32 * 12.0, r.center().y - 7.0);
            ui.painter().rect_filled(
                egui::Rect::from_min_size(min, egui::vec2(10.0, 14.0)),
                2.0,
                c,
            );
        }
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

/// A table line on the Keyboard page.
enum Line {
    Group(&'static str),
    Command(&'static Def),
}

/// Commands as lines, with a group header before each group (COMMANDS is ordered by group).
fn lines(commands: &[&'static Def]) -> Vec<Line> {
    let mut out = Vec::new();
    for d in commands {
        if !matches!(out.last(), Some(Line::Command(p)) if p.group == d.group) {
            out.push(Line::Group(d.group));
        }
        out.push(Line::Command(d));
    }
    out
}

/// Whether the draft's shortcuts of `d` differ from its defaults.
fn changed(d: &Def, st: &State) -> bool {
    st.draft
        .keymap
        .get(d.id)
        .is_some_and(|keys| keys.as_slice() != d.defaults)
}

/// The Keyboard list (Action | Shortcut, grouped) with the edit strip under it.
/// `table_height` is the height of the scrolling list.
fn keyboard_section(ui: &mut Ui, st: &mut State, commands: &[&'static Def], table_height: f32) {
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
    let lines = lines(commands);
    let mut clicked = None;
    let mut double_clicked = None;
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, theme::panel_hi()))
        .corner_radius(4)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            TableBuilder::new(ui)
                .id_salt("settings-keyboard")
                .striped(true)
                .resizable(false)
                .sense(Sense::click())
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::remainder().at_least(160.0).clip(true))
                .column(Column::initial(260.0).at_least(140.0).clip(true))
                .min_scrolled_height(table_height)
                .max_scroll_height(table_height)
                .header(TABLE_ROW, |mut header| {
                    for key in ["settings.keyboard_action", "settings.keyboard_shortcut"] {
                        header.col(|ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(t(key)).strong().color(theme::header_text()),
                                )
                                .selectable(false),
                            );
                        });
                    }
                })
                .body(|body| {
                    body.rows(TABLE_ROW, lines.len(), |mut row| {
                        match &lines[row.index()] {
                            Line::Group(group) => {
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(t(group)).strong().color(theme::dim()),
                                        )
                                        .selectable(false),
                                    );
                                });
                                row.col(|_| {});
                            }
                            Line::Command(d) => {
                                row.set_selected(st.selected == Some(d.command));
                                row.col(|ui| {
                                    ui.add_space(16.0);
                                    ui.add(egui::Label::new(t(d.title)).selectable(false));
                                });
                                row.col(|ui| shortcut_cell(ui, st, d));
                                let r = row.response();
                                if r.double_clicked() {
                                    double_clicked = Some(d.command);
                                } else if r.clicked() {
                                    clicked = Some(d.command);
                                }
                            }
                        }
                    });
                });
        });
    if let Some(command) = double_clicked.or(clicked) {
        if st.selected != Some(command) {
            st.recording = None;
        }
        st.selected = Some(command);
        if double_clicked.is_some() {
            st.recording = Some(command);
        }
    }
    ui.add_space(ROW_GAP);
    edit_strip(ui, st);
    ui.add_space(4.0);
    note(ui, t("settings.keyboard_note"), 0.0);
}

/// The shortcuts of one command: bold when changed from the default, with a conflict warning.
fn shortcut_cell(ui: &mut Ui, st: &State, d: &Def) {
    let shortcuts = command::shortcuts(d.command, &st.draft.keymap);
    if shortcuts.is_empty() {
        ui.label(RichText::new(t("settings.keyboard_none")).color(theme::dim()));
    } else {
        let text: Vec<String> = shortcuts.iter().map(Shortcut::display).collect();
        let mut text = RichText::new(text.join(", "));
        if changed(d, st) {
            text = text.strong();
        }
        ui.add(egui::Label::new(text).selectable(false));
    }
    let others: Vec<&str> = command::conflicts(d.command, &st.draft.keymap)
        .into_iter()
        .map(|c| t(c.def().title))
        .collect();
    if !others.is_empty() {
        let also = fx_app::tr("settings.keyboard_also", &[("others", &others.join(", "))]);
        ui.add(
            egui::Label::new(RichText::new(format!("\u{26A0} {also}")).color(theme::error()))
                .selectable(false),
        );
    }
}

/// Acts on the selected row: a read-only field plus Record, Clear, Reset and Reset all.
fn edit_strip(ui: &mut Ui, st: &mut State) {
    let selected = st.selected.map(Command::def);
    let mut shown = match (st.recording, selected) {
        (Some(_), _) => t("settings.keyboard_press").to_owned(),
        (None, Some(d)) => {
            let keys = command::shortcuts(d.command, &st.draft.keymap);
            if keys.is_empty() {
                String::new()
            } else {
                keys.iter()
                    .map(Shortcut::display)
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        }
        (None, None) => String::new(),
    };
    let min = egui::vec2(BUTTON_MIN_WIDTH, CONTROL_HEIGHT);
    ui.horizontal_wrapped(|ui| {
        ui.label(t("settings.keyboard_shortcut"));
        let width = (ui.available_width() - 3.0 * (BUTTON_MIN_WIDTH + 8.0) - 8.0).max(160.0);
        ui.add(
            TextEdit::singleline(&mut shown)
                .interactive(false)
                .desired_width(width)
                .min_size(egui::vec2(0.0, CONTROL_HEIGHT))
                .align(egui::Align2::LEFT_CENTER),
        );
        let has_keys =
            selected.is_some_and(|d| !command::shortcuts(d.command, &st.draft.keymap).is_empty());
        let overridden = selected.is_some_and(|d| st.draft.keymap.contains_key(d.id));
        let button = |ui: &mut Ui, enabled: bool, key: &'static str| {
            ui.add_enabled(enabled, egui::Button::new(t(key)).min_size(min))
                .clicked()
        };
        if button(ui, selected.is_some(), "settings.keyboard_record") {
            st.recording = st.selected;
        }
        if button(ui, has_keys, "settings.keyboard_clear")
            && let Some(d) = selected
        {
            st.draft.keymap.insert(d.id.to_owned(), Vec::new());
        }
        if button(ui, overridden, "settings.keyboard_reset")
            && let Some(d) = selected
        {
            st.draft.keymap.remove(d.id);
        }
        if button(
            ui,
            !st.draft.keymap.is_empty(),
            "settings.keyboard_reset_all",
        ) {
            st.draft.keymap.clear();
        }
    });
}

/// A page or search-result title.
fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).heading().strong());
    ui.add_space(4.0);
}

/// A bordered group with its title top-left inside, like a Qt group box.
fn group_box(ui: &mut Ui, title: &str, add_contents: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, theme::panel_hi()))
        .corner_radius(6)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong());
            ui.add_space(ROW_GAP);
            add_contents(ui);
        });
    ui.add_space(GROUP_GAP);
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
        .fold(0.0, f32::max)
        .ceil()
}

/// Rows as group boxes; field rows are label | control, check rows span both columns.
/// In search results (`searching`) the box titles read "Category › Group".
fn form(ui: &mut Ui, rows: &[&Row], form: &mut Form, searching: bool) {
    let label_w = label_width(ui, rows);
    let mut start = 0;
    while start < rows.len() {
        let (category, group) = (rows[start].category, rows[start].group);
        let end = start
            + rows[start..]
                .iter()
                .take_while(|r| r.category == category && r.group == group)
                .count();
        let heading = if searching {
            fx_app::tr(
                "settings.search_group",
                &[("category", category.label()), ("group", t(group))],
            )
        } else {
            t(group).to_owned()
        };
        group_box(ui, &heading, |ui| {
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            for r in &rows[start..end] {
                let label = t(r.label);
                if r.kind == Kind::Check {
                    ui.vertical(|ui| (r.show)(ui, form, label));
                    continue;
                }
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = COLUMN_GAP;
                    ui.allocate_ui_with_layout(
                        egui::vec2(label_w, CONTROL_HEIGHT),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.set_min_size(egui::vec2(label_w, CONTROL_HEIGHT));
                            ui.label(label);
                        },
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = ROW_GAP;
                        (r.show)(ui, form, label);
                    });
                });
            }
        });
        start = end;
    }
}

/// A sidebar category entry: full-width 26 px row, rounded fill when selected or hovered.
fn category_row(ui: &mut Ui, label: &str, selected: bool, dimmed: bool) -> egui::Response {
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
        if dimmed { theme::dim() } else { theme::text() },
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

/// Commands whose title, group or the Keyboard category label contains `query`.
fn search_commands(query: &str) -> Vec<&'static Def> {
    let q = query.trim().to_lowercase();
    let keyboard = Category::Keyboard.label().to_lowercase().contains(&q);
    COMMANDS
        .iter()
        .filter(|d| keyboard || t(d.title).to_lowercase().contains(&q))
        .collect()
}

/// Whether the search `query` finds anything in `category`.
fn category_matches(category: Category, query: &str) -> bool {
    if category == Category::Keyboard {
        !search_commands(query).is_empty()
    } else {
        search(query).iter().any(|r| r.category == category)
    }
}

pub struct State {
    pub draft: Settings,
    pub start_text: String,
    pub category: Category,
    pub query: String,
    /// The command selected in the Keyboard list.
    selected: Option<Command>,
    /// The command whose next key combination is being captured.
    recording: Option<Command>,
    /// What the live preview last applied.
    previewed: Settings,
    /// Rendered inside the main window (no native viewports available).
    embedded: bool,
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
            selected: None,
            recording: None,
            previewed: saved.clone(),
            embedded: false,
        }
    }

    /// The draft as it would be saved: with the typed start folder.
    fn pending(&self) -> Settings {
        let mut settings = self.draft.clone();
        let folder = self.start_text.trim();
        settings.start_folder = (!folder.is_empty()).then(|| PathBuf::from(folder));
        settings
    }

    /// Whether the settings window is shown inside the main window rather than as its own.
    pub fn embedded(&self) -> bool {
        self.embedded
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
    let commands: Vec<&Def> = if searching {
        search_commands(&query)
    } else if keyboard_page {
        COMMANDS.iter().collect()
    } else {
        Vec::new()
    };
    if !searching {
        title(ui, st.category.label());
    }
    if rows.is_empty() && commands.is_empty() {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(t("settings.none")).color(theme::dim()));
        });
        return;
    }
    let mut f = Form {
        draft: &mut st.draft,
        start_text: &mut st.start_text,
        current_folder,
    };
    form(ui, &rows, &mut f, searching);
    if commands.is_empty() {
        return;
    }
    if keyboard_page {
        // Fill the page: list, then strip (26) + note (~20) + gaps.
        let height = (ui.available_height() - TABLE_ROW - CONTROL_HEIGHT - 48.0).max(120.0);
        keyboard_section(ui, st, &commands, height);
    } else {
        group_box(ui, Category::Keyboard.label(), |ui| {
            keyboard_section(ui, st, &commands, 220.0);
        });
    }
}

/// The sidebar: search field and the category list (dimmed when the search finds nothing there).
fn sidebar(ui: &mut Ui, st: &mut State) {
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.add(
        TextEdit::singleline(&mut st.query)
            .hint_text(t("settings.search"))
            .desired_width(f32::INFINITY)
            .min_size(egui::vec2(0.0, CONTROL_HEIGHT))
            .align(egui::Align2::LEFT_CENTER),
    );
    ui.add_space(6.0);
    let searching = !st.query.trim().is_empty();
    for c in Category::ALL {
        let selected = st.category == c && !searching;
        let dimmed = searching && !category_matches(c, &st.query);
        if category_row(ui, c.label(), selected, dimmed).clicked() {
            st.category = c;
            st.query.clear();
        }
    }
}

/// The settings window's id, for focusing it from the main window.
pub fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("settings")
}

/// What the footer asked for.
#[derive(Default)]
struct Outcome {
    apply: bool,
    done: bool,
}

/// Window content: footer, sidebar and page. `ok`/`cancel` are signalled through the outcome.
fn content(ui: &mut Ui, explorer: &Explorer, st: &mut State, out: &mut Outcome) {
    let dirty = st.pending() != *explorer.settings();
    let style = ui.style().clone();
    egui::Panel::bottom("settings-footer")
        .frame(egui::Frame::side_top_panel(&style).inner_margin(12))
        .show(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let min = egui::vec2(BUTTON_MIN_WIDTH, CONTROL_HEIGHT);
                if ui
                    .add_enabled(dirty, egui::Button::new(t("button.apply")).min_size(min))
                    .clicked()
                {
                    out.apply = true;
                }
                if ui
                    .add(egui::Button::new(t("button.cancel")).min_size(min))
                    .clicked()
                {
                    out.done = true;
                }
                let ok = egui::Button::new(RichText::new(t("button.ok")).color(theme::on_accent()))
                    .fill(theme::accent())
                    .min_size(min);
                if ui.add(ok).clicked() {
                    out.apply = true;
                    out.done = true;
                }
            });
        });
    egui::Panel::left("settings-sidebar")
        .resizable(false)
        .exact_size(SIDEBAR_WIDTH)
        .frame(egui::Frame::side_top_panel(&style).inner_margin(12))
        .show(ui, |ui| sidebar(ui, st));
    let current_folder = explorer.active().path().display().to_string();
    let margin = egui::Margin {
        left: 16,
        right: 12,
        top: 12,
        bottom: 12,
    };
    egui::CentralPanel::default()
        .frame(egui::Frame::central_panel(&style).inner_margin(margin))
        .show(ui, |ui| {
            let spacing = ui.spacing_mut();
            spacing.item_spacing = egui::vec2(8.0, ROW_GAP);
            spacing.interact_size.y = CONTROL_HEIGHT;
            spacing.icon_spacing = 6.0;
            if st.query.trim().is_empty() && st.category == Category::Keyboard {
                page(ui, st, current_folder);
            } else {
                ScrollArea::vertical()
                    .id_salt("settings-page")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_max_width(ui.available_width() - 12.0);
                        page(ui, st, current_folder);
                    });
            }
        });
}

/// Draws the settings window. Returns true when it should be closed.
pub fn show(ctx: &Context, explorer: &mut Explorer, st: &mut State) -> bool {
    let builder = ViewportBuilder::default()
        .with_title(t("settings.title"))
        .with_inner_size([960.0, 640.0])
        .with_min_inner_size([740.0, 510.0]);
    let mut out = Outcome::default();
    ctx.show_viewport_immediate(viewport_id(), builder, |ui, class| {
        let ctx = ui.ctx().clone();
        st.embedded = class == ViewportClass::EmbeddedWindow;
        if st.embedded {
            ui.set_min_size(egui::vec2(900.0, 560.0));
        }
        // Esc first stops a recording or clears a non-empty search, else cancels.
        let esc_free = st.recording.is_none() && !egui::Popup::is_any_open(&ctx);
        if esc_free && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            if st.query.is_empty() {
                out.done = true;
            } else {
                st.query.clear();
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            out.done = true;
        }
        if look_changed(&st.draft, &st.previewed) {
            theme::apply(&ctx, &st.draft);
            st.previewed = st.draft.clone();
        }
        content(ui, explorer, st, &mut out);
    });
    if out.apply {
        let pending = st.pending();
        explorer.set_settings(pending);
    }
    if out.done {
        theme::apply(ctx, explorer.settings());
    }
    out.done
}

/// Brings the settings window to the front.
pub fn focus(ctx: &Context) {
    ctx.send_viewport_cmd_to(viewport_id(), ViewportCommand::Focus);
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
        assert!(search_commands("zzz").is_empty());
        assert!(category_matches(Category::Keyboard, "rename"));
        assert!(!category_matches(Category::Fonts, "rename"));
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
    fn command_groups_have_text() {
        for d in COMMANDS {
            assert_ne!(t(d.group), d.group, "missing text for {}", d.group);
        }
    }

    #[test]
    fn commands_are_ordered_by_group() {
        let mut seen: Vec<&str> = Vec::new();
        for d in COMMANDS {
            if seen.last() != Some(&d.group) {
                assert!(!seen.contains(&d.group), "{} split", d.group);
                seen.push(d.group);
            }
        }
    }

    #[test]
    fn groups_are_contiguous_within_a_category() {
        // `form` starts a new box whenever the group changes, so a group split in two would show twice.
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
