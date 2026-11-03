//! Modal dialogs: delete confirmation, rename, new folder, properties, settings.

use crate::theme;
use eframe::egui::{
    self, Align, Button, Context, Id, Key, Layout, Modal, RichText, TextEdit, Ui,
    text::{CCursor, CCursorRange},
};
use fx_app::{
    Density, EntryDetails, Explorer, Settings, ThemeChoice, format_size, format_time, stem_chars,
    validate_name,
};
use fx_core::EntryKind;
use std::path::PathBuf;

const LISTED_NAMES: usize = 5;
const WIDTH: f32 = 380.0;

pub enum Dialog {
    /// `paths` pin what the user was shown, so a reload behind the dialog cannot redirect the delete.
    Delete {
        rows: Vec<usize>,
        paths: Vec<PathBuf>,
        permanent: bool,
    },
    Rename {
        row: usize,
        path: PathBuf,
        text: String,
        fresh: bool,
    },
    NewFolder {
        text: String,
        fresh: bool,
    },
    Properties(EntryDetails),
    Settings {
        draft: Settings,
        start_text: String,
    },
}

impl Dialog {
    pub fn delete(explorer: &Explorer, permanent: bool) -> Option<Self> {
        let rows = explorer.active().selection();
        let paths: Vec<PathBuf> = rows
            .iter()
            .filter_map(|&row| explorer.entry_path(row))
            .collect();
        (!paths.is_empty()).then_some(Self::Delete {
            rows,
            paths,
            permanent,
        })
    }

    pub fn rename(explorer: &Explorer) -> Option<Self> {
        let row = explorer.active().selected()?;
        let path = explorer.entry_path(row)?;
        let text = file_name(&path);
        Some(Self::Rename {
            row,
            path,
            text,
            fresh: true,
        })
    }

    pub fn new_folder(explorer: &Explorer) -> Self {
        Self::NewFolder {
            text: explorer.suggest_folder_name(),
            fresh: true,
        }
    }

    pub fn properties(explorer: &Explorer) -> Option<Self> {
        let row = explorer.active().selected()?;
        explorer.entry_details(row).map(Self::Properties)
    }

    pub fn settings(explorer: &Explorer) -> Self {
        let draft = explorer.settings().clone();
        let start_text = draft
            .start_folder
            .as_ref()
            .map_or_else(String::new, |p| p.display().to_string());
        Self::Settings { draft, start_text }
    }
}

/// Draws the dialog. Returns true when it should be closed.
pub fn show(ctx: &Context, dialog: &mut Dialog, explorer: &mut Explorer) -> bool {
    match dialog {
        Dialog::Delete {
            rows,
            paths,
            permanent,
        } => delete(ctx, explorer, rows, paths, *permanent),
        Dialog::Rename {
            row,
            path,
            text,
            fresh,
        } => rename(ctx, explorer, *row, path, text, fresh),
        Dialog::NewFolder { text, fresh } => new_folder(ctx, explorer, text, fresh),
        Dialog::Properties(details) => properties(ctx, details),
        Dialog::Settings { draft, start_text } => settings(ctx, explorer, draft, start_text),
    }
}

fn modal(ctx: &Context, id: &str, content: impl FnOnce(&mut Ui) -> bool) -> bool {
    let response = Modal::new(Id::unique(id)).show(ctx, |ui| {
        ui.set_width(WIDTH);
        content(ui)
    });
    response.inner || response.should_close()
}

/// Right-aligned button row; the closure adds buttons in right-to-left order (primary first).
fn button_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.add_space(8.0);
    ui.with_layout(Layout::right_to_left(Align::Center), add);
}

/// Focuses `response` once, while nothing else holds the focus, so Enter triggers the default button.
fn default_focus(ui: &Ui, response: &egui::Response) {
    if ui.memory(|m| m.focused().is_none()) {
        response.request_focus();
    }
}

fn delete(
    ctx: &Context,
    explorer: &mut Explorer,
    rows: &[usize],
    paths: &[PathBuf],
    permanent: bool,
) -> bool {
    modal(ctx, "delete-dialog", |ui| {
        let count = paths.len();
        let subject = if count == 1 {
            format!("\"{}\"", file_name(&paths[0]))
        } else {
            format!("{count} items")
        };
        if permanent {
            ui.label(
                RichText::new(format!(
                    "Permanently delete {subject}? This cannot be undone."
                ))
                .heading()
                .color(theme::error()),
            );
        } else {
            ui.heading(format!("Move {subject} to Trash?"));
        }
        if count > 1 {
            ui.add_space(6.0);
            for path in paths.iter().take(LISTED_NAMES) {
                ui.label(RichText::new(file_name(path)).color(theme::dim()));
            }
            if count > LISTED_NAMES {
                ui.label(
                    RichText::new(format!("…and {} more", count - LISTED_NAMES))
                        .color(theme::dim()),
                );
            }
        }
        let mut close = false;
        button_row(ui, |ui| {
            let (label, color) = if permanent {
                ("Delete permanently", theme::error())
            } else {
                ("Move to Trash", theme::accent())
            };
            let confirm = ui.add(Button::new(RichText::new(label).color(color)));
            let cancel = ui.add(Button::new("Cancel"));
            default_focus(ui, if permanent { &cancel } else { &confirm });
            if confirm.clicked() {
                let unchanged = rows
                    .iter()
                    .zip(paths)
                    .all(|(&row, path)| explorer.entry_path(row).as_ref() == Some(path));
                if unchanged {
                    explorer.delete(rows, permanent);
                }
                close = true;
            }
            close |= cancel.clicked();
        });
        close
    })
}

fn rename(
    ctx: &Context,
    explorer: &mut Explorer,
    row: usize,
    path: &PathBuf,
    text: &mut String,
    fresh: &mut bool,
) -> bool {
    modal(ctx, "rename-dialog", |ui| {
        ui.heading("Rename");
        ui.add_space(6.0);
        let selection = stem_chars(text);
        let mut close = false;
        let submit = name_field(ui, text, fresh, Some(selection));
        let error = validate_name(text).err();
        if let Some(err) = &error {
            ui.colored_label(theme::error(), err.to_string());
        }
        button_row(ui, |ui| {
            let ok = ui.add_enabled(error.is_none(), Button::new("Rename"));
            close |= ui.button("Cancel").clicked();
            if error.is_none() && (ok.clicked() || submit) {
                if explorer.entry_path(row).as_ref() == Some(path) && *text != file_name(path) {
                    explorer.rename(row, text);
                }
                close = true;
            }
        });
        close
    })
}

fn new_folder(ctx: &Context, explorer: &mut Explorer, text: &mut String, fresh: &mut bool) -> bool {
    modal(ctx, "new-folder-dialog", |ui| {
        ui.heading("New folder");
        ui.add_space(6.0);
        let mut close = false;
        let submit = name_field(ui, text, fresh, None);
        let error = validate_name(text).err();
        if let Some(err) = &error {
            ui.colored_label(theme::error(), err.to_string());
        }
        button_row(ui, |ui| {
            let ok = ui.add_enabled(error.is_none(), Button::new("Create"));
            close |= ui.button("Cancel").clicked();
            if error.is_none() && (ok.clicked() || submit) {
                explorer.create_folder(text);
                close = true;
            }
        });
        close
    })
}

/// A name field that grabs focus and pre-selects `select_chars` characters (all if `None`) when fresh.
/// Returns true when Enter was pressed in it.
fn name_field(
    ui: &mut Ui,
    text: &mut String,
    fresh: &mut bool,
    select_chars: Option<usize>,
) -> bool {
    let id = ui.make_persistent_id("name-field");
    let mut out = TextEdit::singleline(text)
        .id(id)
        .desired_width(f32::INFINITY)
        .show(ui);
    if std::mem::take(fresh) {
        out.response.request_focus();
        let end = select_chars.unwrap_or_else(|| text.chars().count());
        out.state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(end))));
        out.state.store(ui.ctx(), id);
    }
    let submit = out.response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    if submit {
        out.response.request_focus();
    }
    submit
}

fn properties(ctx: &Context, details: &EntryDetails) -> bool {
    modal(ctx, "properties-dialog", |ui| {
        ui.heading("Properties");
        ui.add_space(6.0);
        let kind = match details.kind {
            EntryKind::Dir => "Folder",
            EntryKind::File => "File",
            EntryKind::Symlink => "Symbolic link",
            EntryKind::Other => "Other",
        };
        let size = details.size.map_or_else(
            || "—".to_owned(),
            |bytes| format!("{} ({bytes} bytes)", format_size(bytes)),
        );
        let modified = details.modified.map_or_else(|| "—".to_owned(), format_time);
        let path = details.path.display().to_string();
        egui::Grid::new("properties")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                let row = |ui: &mut Ui, label: &str, value: &str| {
                    ui.label(RichText::new(label).color(theme::dim()));
                    ui.label(value);
                    ui.end_row();
                };
                row(ui, "Name", &details.name);
                ui.label(RichText::new("Path").color(theme::dim()));
                // A `&str` buffer is read-only but still selectable and copyable.
                ui.add(TextEdit::singleline(&mut path.as_str()).desired_width(260.0));
                ui.end_row();
                row(ui, "Type", kind);
                row(ui, "Size", &size);
                row(ui, "Modified", &modified);
                row(ui, "Permissions", &details.permissions);
            });
        let mut close = false;
        button_row(ui, |ui| {
            let button = ui.button("Close");
            default_focus(ui, &button);
            close = button.clicked();
        });
        close
    })
}

fn settings(
    ctx: &Context,
    explorer: &mut Explorer,
    draft: &mut Settings,
    start_text: &mut String,
) -> bool {
    modal(ctx, "settings-dialog", |ui| {
        ui.heading("Settings");

        section(ui, "General");
        ui.horizontal(|ui| {
            ui.label("Start folder");
            ui.add(
                TextEdit::singleline(start_text)
                    .hint_text("Home folder")
                    .desired_width(160.0),
            );
            if ui.button("Use current folder").clicked() {
                *start_text = explorer.active().path().display().to_string();
            }
        });
        ui.checkbox(&mut draft.show_hidden, "Show hidden items");

        section(ui, "Deleting");
        ui.checkbox(&mut draft.use_trash, "Move deleted items to Trash");
        ui.checkbox(&mut draft.confirm_delete, "Confirm before deleting");
        if !draft.use_trash {
            ui.label(
                RichText::new("Without Trash, deleting is permanent and always asks first.")
                    .small()
                    .color(theme::dim()),
            );
        }

        section(ui, "Appearance");
        ui.horizontal(|ui| {
            ui.label("Theme");
            ui.radio_value(&mut draft.theme, ThemeChoice::Dark, "Dark");
            ui.radio_value(&mut draft.theme, ThemeChoice::Light, "Light");
        });
        ui.horizontal(|ui| {
            ui.label("Density");
            ui.radio_value(&mut draft.density, Density::Compact, "Compact");
            ui.radio_value(&mut draft.density, Density::Comfortable, "Comfortable");
        });
        ui.checkbox(&mut draft.show_ext_column, "Show Ext column");
        ui.checkbox(&mut draft.show_age_column, "Show Age column");

        let mut apply = |draft: &mut Settings| {
            let folder = start_text.trim();
            draft.start_folder = (!folder.is_empty()).then(|| PathBuf::from(folder));
            explorer.set_settings(draft.clone());
        };
        let mut close = false;
        button_row(ui, |ui| {
            if ui.button("Apply").clicked() {
                apply(draft);
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
            if ui.button("OK").clicked() {
                apply(draft);
                close = true;
            }
        });
        close
    })
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).strong().color(theme::accent()));
    ui.separator();
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}
