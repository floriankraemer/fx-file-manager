//! Modal dialogs: delete confirmation, rename, new folder, properties, settings, paste conflicts.

use super::{about, settings};
use crate::theme;
use eframe::egui::{
    self, Align, Button, Context, Id, Key, Layout, Modal, RichText, TextEdit, Ui,
    text::{CCursor, CCursorRange},
};
use fx_app::{
    EntryDetails, Explorer, format_size, format_time, stem_chars, t, tn, tr, validate_name,
};
use fx_core::{Conflict, EntryKind, Resolution};
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
    Settings(Box<settings::State>),
    About,
    /// A paste found an existing item; only that item waits, the rest of the transfer goes on.
    Conflict {
        transfer: u64,
        conflict: Box<Conflict>,
        apply_to_all: bool,
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

    /// The oldest unanswered conflict of any transfer.
    pub fn conflict(explorer: &Explorer) -> Option<Self> {
        explorer.transfers().into_iter().find_map(|t| {
            t.conflict.map(|c| Self::Conflict {
                transfer: t.id,
                conflict: Box::new(c),
                apply_to_all: false,
            })
        })
    }

    pub fn settings(explorer: &Explorer) -> Self {
        Self::Settings(Box::new(settings::State::new(explorer.settings())))
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
        Dialog::Settings(state) => settings::show(ctx, explorer, state),
        Dialog::About => about::show(ctx),
        Dialog::Conflict {
            transfer,
            conflict,
            apply_to_all,
        } => conflict_dialog(ctx, explorer, *transfer, conflict, apply_to_all),
    }
}

fn conflict_dialog(
    ctx: &Context,
    explorer: &mut Explorer,
    transfer: u64,
    c: &Conflict,
    apply_to_all: &mut bool,
) -> bool {
    // Answered elsewhere (apply to all) or the transfer ended: nothing left to ask.
    let pending = explorer
        .transfers()
        .iter()
        .any(|t| t.id == transfer && t.conflict.as_ref().is_some_and(|p| p.id == c.id));
    if !pending {
        return true;
    }
    modal(ctx, "conflict-dialog", |ui| {
        let what = if c.dst_is_dir { "A folder" } else { "A file" };
        ui.heading(format!(
            "{what} named \"{}\" already exists",
            file_name(&c.dst)
        ));
        if let Some(parent) = c.dst.parent() {
            ui.label(RichText::new(format!("in {}", parent.display())).color(theme::dim()));
        }
        ui.add_space(6.0);
        let describe = |is_dir: bool, size: u64, modified: Option<std::time::SystemTime>| {
            let when = modified.map_or_else(|| "—".to_owned(), format_time);
            if is_dir {
                tr("conflict.folder_modified", &[("time", &when)])
            } else {
                tr(
                    "conflict.file_modified",
                    &[("size", &format_size(size)), ("time", &when)],
                )
            }
        };
        egui::Grid::new("conflict")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label(RichText::new(t("conflict.existing")).color(theme::dim()));
                ui.label(describe(c.dst_is_dir, c.dst_size, c.dst_modified));
                ui.end_row();
                ui.label(RichText::new(t("conflict.new")).color(theme::dim()));
                ui.label(describe(c.src_is_dir, c.src_size, c.src_modified));
                ui.end_row();
            });
        ui.add_space(6.0);
        ui.checkbox(apply_to_all, t("conflict.apply_all"));
        let mut answer = None;
        let mut cancel = false;
        button_row(ui, |ui| {
            // A file cannot replace a folder (or the reverse): only skip or keep both.
            let replace = c.src_is_dir == c.dst_is_dir && !c.dst_is_dir;
            let keep = ui.button(t("conflict.keep_both"));
            if ui
                .add_enabled(replace, Button::new(t("conflict.replace")))
                .clicked()
            {
                answer = Some(Resolution::Overwrite);
            }
            let skip = ui.button(t("conflict.skip"));
            default_focus(ui, &keep);
            if keep.clicked() {
                answer = Some(Resolution::KeepBoth);
            }
            if skip.clicked() {
                answer = Some(Resolution::Skip);
            }
            cancel = ui.button(t("conflict.cancel_transfer")).clicked();
        });
        if let Some(resolution) = answer {
            explorer.answer_conflict(transfer, c.id, resolution, *apply_to_all);
        }
        if cancel {
            explorer.cancel_transfer(transfer);
        }
        answer.is_some() || cancel
    })
}

pub(super) fn modal(ctx: &Context, id: &str, content: impl FnOnce(&mut Ui) -> bool) -> bool {
    let response = Modal::new(Id::unique(id)).show(ctx, |ui| {
        ui.set_width(WIDTH);
        content(ui)
    });
    response.inner || response.should_close()
}

/// Right-aligned button row; the closure adds buttons in right-to-left order (primary first).
pub(super) fn button_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
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
        let name = file_name(&paths[0]);
        let (single, multi) = if permanent {
            (
                "dialog.delete.permanent_single",
                "dialog.delete.permanent_multi",
            )
        } else {
            ("dialog.delete.trash_single", "dialog.delete.trash_multi")
        };
        let title = if count == 1 {
            tr(single, &[("name", &name)])
        } else {
            tn(multi, count as u64, &[])
        };
        if permanent {
            ui.label(RichText::new(title).heading().color(theme::error()));
        } else {
            ui.heading(title);
        }
        if count > 1 {
            ui.add_space(6.0);
            for path in paths.iter().take(LISTED_NAMES) {
                ui.label(RichText::new(file_name(path)).color(theme::dim()));
            }
            if count > LISTED_NAMES {
                ui.label(
                    RichText::new(tr(
                        "dialog.delete.more",
                        &[("n", &(count - LISTED_NAMES).to_string())],
                    ))
                    .color(theme::dim()),
                );
            }
        }
        let mut close = false;
        button_row(ui, |ui| {
            let (label, color) = if permanent {
                (t("button.delete_permanently"), theme::error())
            } else {
                (t("button.move_to_trash"), theme::accent())
            };
            let confirm = ui.add(Button::new(RichText::new(label).color(color)));
            let cancel = ui.add(Button::new(t("button.cancel")));
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
        ui.heading(t("cmd.rename"));
        ui.add_space(6.0);
        let selection = stem_chars(text);
        let mut close = false;
        let submit = name_field(ui, text, fresh, Some(selection));
        let error = validate_name(text).err();
        if let Some(err) = &error {
            ui.colored_label(theme::error(), err.to_string());
        }
        button_row(ui, |ui| {
            let ok = ui.add_enabled(error.is_none(), Button::new(t("button.rename")));
            close |= ui.button(t("button.cancel")).clicked();
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
        ui.heading(t("cmd.new_folder"));
        ui.add_space(6.0);
        let mut close = false;
        let submit = name_field(ui, text, fresh, None);
        let error = validate_name(text).err();
        if let Some(err) = &error {
            ui.colored_label(theme::error(), err.to_string());
        }
        button_row(ui, |ui| {
            let ok = ui.add_enabled(error.is_none(), Button::new(t("button.create")));
            close |= ui.button(t("button.cancel")).clicked();
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
        ui.heading(t("cmd.properties"));
        ui.add_space(6.0);
        let kind = match details.kind {
            EntryKind::Dir => t("kind.folder"),
            EntryKind::File => t("kind.file"),
            EntryKind::Symlink => t("kind.symlink"),
            EntryKind::Other => t("kind.misc"),
        };
        let size = details.size.map_or_else(
            || "—".to_owned(),
            |bytes| tn("prop.size_value", bytes, &[("size", &format_size(bytes))]),
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
                row(ui, t("prop.name"), &details.name);
                ui.label(RichText::new(t("prop.path")).color(theme::dim()));
                // A `&str` buffer is read-only but still selectable and copyable.
                ui.add(TextEdit::singleline(&mut path.as_str()).desired_width(260.0));
                ui.end_row();
                row(ui, t("prop.type"), kind);
                row(ui, t("prop.size"), &size);
                row(ui, t("prop.modified"), &modified);
                row(ui, t("prop.permissions"), &details.permissions);
            });
        let mut close = false;
        button_row(ui, |ui| {
            let button = ui.button(t("button.close"));
            default_focus(ui, &button);
            close = button.clicked();
        });
        close
    })
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}
