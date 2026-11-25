//! Modal dialogs: delete confirmation, rename, new folder, properties, paste conflicts.

use super::modal::{button_row, default_focus, modal};
use crate::theme;
use eframe::egui::{
    self, Button, Context, Key, RichText, TextEdit, Ui,
    text::{CCursor, CCursorRange},
};
use fx_app::{
    DetailsState, EntryDetails, Explorer, format_size, format_time, stem_chars, t, tn, tr,
    validate_name,
};
use fx_core::{Conflict, EntryKind, Resolution};
use std::path::PathBuf;

const LISTED_NAMES: usize = 5;

pub enum Dialog {
    /// `paths` pin what the user was shown, so a reload behind the dialog cannot redirect the delete.
    Delete {
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
    /// `path` pins the bookmark the user picked, so a reorder behind the dialog cannot rename another one.
    RenameBookmark {
        index: usize,
        path: PathBuf,
        text: String,
        fresh: bool,
    },
    /// Shows [`Explorer::details`], which a background read fills in.
    Properties,
    /// Searching from a filesystem root; the explorer holds the query that waits for the answer.
    SearchRoot,
    /// A paste found an existing item; only that item waits, the rest of the transfer goes on.
    Conflict {
        transfer: u64,
        conflict: Box<Conflict>,
        apply_to_all: bool,
    },
}

impl Dialog {
    pub fn delete_paths(paths: Vec<PathBuf>, permanent: bool) -> Option<Self> {
        (!paths.is_empty()).then_some(Self::Delete { paths, permanent })
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

    pub fn rename_bookmark(explorer: &Explorer, index: usize) -> Option<Self> {
        let bookmark = explorer.bookmarks().get(index)?;
        Some(Self::RenameBookmark {
            index,
            path: bookmark.path.clone(),
            text: bookmark.label.clone(),
            fresh: true,
        })
    }

    pub fn new_folder(explorer: &Explorer) -> Self {
        Self::NewFolder {
            text: explorer.suggest_folder_name(),
            fresh: true,
        }
    }

    pub fn properties(explorer: &mut Explorer) -> Option<Self> {
        let row = explorer.active().selected()?;
        explorer.request_details(row).then_some(Self::Properties)
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
}

/// Draws the dialog. Returns true when it should be closed.
pub fn show(ctx: &Context, dialog: &mut Dialog, explorer: &mut Explorer) -> bool {
    match dialog {
        Dialog::Delete { paths, permanent } => delete(ctx, explorer, paths, *permanent),
        Dialog::Rename {
            row,
            path,
            text,
            fresh,
        } => rename(ctx, explorer, *row, path, text, fresh),
        Dialog::NewFolder { text, fresh } => new_folder(ctx, explorer, text, fresh),
        Dialog::RenameBookmark {
            index,
            path,
            text,
            fresh,
        } => rename_bookmark(ctx, explorer, *index, path, text, fresh),
        Dialog::Properties => properties(ctx, explorer),
        Dialog::SearchRoot => search_root(ctx, explorer),
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
    if !conflict_pending(explorer, transfer, c) {
        return true;
    }
    modal(ctx, "conflict-dialog", |ui| {
        conflict_header(ui, c);
        conflict_table(ui, c);
        ui.add_space(6.0);
        ui.checkbox(apply_to_all, t("conflict.apply_all"));
        let mut answer = ConflictAnswer::Pending;
        button_row(ui, |ui| answer = conflict_buttons(ui, c));
        apply_answer(explorer, transfer, c.id, *apply_to_all, answer)
    })
}

enum ConflictAnswer {
    Pending,
    Resolve(Resolution),
    CancelTransfer,
}

/// False once the conflict was answered elsewhere (apply to all) or the transfer ended.
fn conflict_pending(explorer: &Explorer, transfer: u64, c: &Conflict) -> bool {
    explorer
        .transfers()
        .iter()
        .any(|t| t.id == transfer && t.conflict.as_ref().is_some_and(|p| p.id == c.id))
}

fn conflict_header(ui: &mut Ui, c: &Conflict) {
    let heading_key = if c.dst_is_dir {
        "conflict.folder_exists"
    } else {
        "conflict.file_exists"
    };
    ui.heading(tr(heading_key, &[("name", &file_name(&c.dst))]));
    if let Some(parent) = c.dst.parent() {
        let dir = parent.display().to_string();
        dim_label(ui, tr("conflict.location", &[("dir", &dir)]));
    }
    ui.add_space(6.0);
}

fn conflict_table(ui: &mut Ui, c: &Conflict) {
    egui::Grid::new("conflict")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            let sides = [
                (
                    "conflict.existing",
                    c.dst_is_dir,
                    c.dst_size,
                    c.dst_modified,
                ),
                ("conflict.new", c.src_is_dir, c.src_size, c.src_modified),
            ];
            for (label, is_dir, size, modified) in sides {
                dim_label(ui, t(label));
                ui.label(describe_side(is_dir, size, modified));
                ui.end_row();
            }
        });
}

fn describe_side(is_dir: bool, size: u64, modified: Option<std::time::SystemTime>) -> String {
    let when = modified.map_or_else(|| "—".to_owned(), format_time);
    if is_dir {
        return tr("conflict.folder_modified", &[("time", &when)]);
    }
    tr(
        "conflict.file_modified",
        &[("size", &format_size(size)), ("time", &when)],
    )
}

fn conflict_buttons(ui: &mut Ui, c: &Conflict) -> ConflictAnswer {
    // A file cannot replace a folder (or the reverse): only skip or keep both.
    let replaceable = !c.src_is_dir && !c.dst_is_dir;
    let keep = ui.button(t("conflict.keep_both"));
    let replace = ui.add_enabled(replaceable, Button::new(t("conflict.replace")));
    let skip = ui.button(t("conflict.skip"));
    default_focus(ui, &keep);
    let cancel = ui.button(t("conflict.cancel_transfer"));
    if replace.clicked() {
        return ConflictAnswer::Resolve(Resolution::Overwrite);
    }
    if keep.clicked() {
        return ConflictAnswer::Resolve(Resolution::KeepBoth);
    }
    if skip.clicked() {
        return ConflictAnswer::Resolve(Resolution::Skip);
    }
    if cancel.clicked() {
        return ConflictAnswer::CancelTransfer;
    }
    ConflictAnswer::Pending
}

/// Forwards the answer to the transfer. Returns true when the dialog is done.
fn apply_answer(
    explorer: &mut Explorer,
    transfer: u64,
    conflict: u64,
    apply_to_all: bool,
    answer: ConflictAnswer,
) -> bool {
    match answer {
        ConflictAnswer::Pending => false,
        ConflictAnswer::Resolve(resolution) => {
            explorer.answer_conflict(transfer, conflict, resolution, apply_to_all);
            true
        }
        ConflictAnswer::CancelTransfer => {
            explorer.cancel_transfer(transfer);
            true
        }
    }
}

fn delete(ctx: &Context, explorer: &mut Explorer, paths: &[PathBuf], permanent: bool) -> bool {
    modal(ctx, "delete-dialog", |ui| {
        let title = delete_title(paths, permanent);
        if permanent {
            ui.label(RichText::new(title).heading().color(theme::error()));
        } else {
            ui.heading(title);
        }
        list_names(ui, paths);
        let mut close = false;
        button_row(ui, |ui| {
            close = delete_buttons(ui, explorer, paths, permanent)
        });
        close
    })
}

fn delete_title(paths: &[PathBuf], permanent: bool) -> String {
    let (single, multi) = if permanent {
        (
            "dialog.delete.permanent_single",
            "dialog.delete.permanent_multi",
        )
    } else {
        ("dialog.delete.trash_single", "dialog.delete.trash_multi")
    };
    match paths {
        [only] => tr(single, &[("name", &file_name(only))]),
        _ => tn(multi, paths.len() as u64, &[]),
    }
}

fn list_names(ui: &mut Ui, paths: &[PathBuf]) {
    if paths.len() <= 1 {
        return;
    }
    ui.add_space(6.0);
    for path in paths.iter().take(LISTED_NAMES) {
        dim_label(ui, file_name(path));
    }
    if let Some(more) = paths.len().checked_sub(LISTED_NAMES).filter(|&n| n > 0) {
        dim_label(ui, tr("dialog.delete.more", &[("n", &more.to_string())]));
    }
}

/// Returns true when the dialog is done.
fn delete_buttons(
    ui: &mut Ui,
    explorer: &mut Explorer,
    paths: &[PathBuf],
    permanent: bool,
) -> bool {
    let (label, color) = if permanent {
        (t("button.delete_permanently"), theme::error())
    } else {
        (t("button.move_to_trash"), theme::accent())
    };
    let confirm = ui.add(Button::new(RichText::new(label).color(color)));
    let cancel = ui.add(Button::new(t("button.cancel")));
    default_focus(ui, if permanent { &cancel } else { &confirm });
    if confirm.clicked() {
        explorer.delete_paths(paths.to_vec(), permanent);
    }
    confirm.clicked() || cancel.clicked()
}

struct NameForm {
    id: &'static str,
    heading: &'static str,
    confirm: &'static str,
    select_stem: bool,
    /// `None` when valid; an empty message disables confirming without showing anything.
    validate: fn(&str) -> Option<String>,
}

#[derive(PartialEq)]
enum NameOutcome {
    Open,
    Cancelled,
    Confirmed,
}

impl NameOutcome {
    fn closes(&self) -> bool {
        *self != Self::Open
    }
}

/// Modal with a name field, a confirm button and cancel. Enter in the field confirms.
fn name_dialog(ctx: &Context, form: &NameForm, text: &mut String, fresh: &mut bool) -> NameOutcome {
    let mut outcome = NameOutcome::Open;
    let closed = modal(ctx, form.id, |ui| {
        ui.heading(t(form.heading));
        ui.add_space(6.0);
        let select = form.select_stem.then(|| stem_chars(text));
        let submit = name_field(ui, text, fresh, select);
        let error = (form.validate)(text);
        if let Some(message) = error.as_deref().filter(|m| !m.is_empty()) {
            ui.colored_label(theme::error(), message);
        }
        let valid = error.is_none();
        button_row(ui, |ui| {
            let ok = ui.add_enabled(valid, Button::new(t(form.confirm)));
            let cancel = ui.button(t("button.cancel"));
            outcome = if valid && (ok.clicked() || submit) {
                NameOutcome::Confirmed
            } else if cancel.clicked() {
                NameOutcome::Cancelled
            } else {
                NameOutcome::Open
            };
        });
        outcome.closes()
    });
    if closed && outcome == NameOutcome::Open {
        return NameOutcome::Cancelled;
    }
    outcome
}

fn invalid_name(text: &str) -> Option<String> {
    validate_name(text).err().map(|e| e.to_string())
}

fn blank_label(text: &str) -> Option<String> {
    text.trim().is_empty().then(String::new)
}

fn rename(
    ctx: &Context,
    explorer: &mut Explorer,
    row: usize,
    path: &PathBuf,
    text: &mut String,
    fresh: &mut bool,
) -> bool {
    let form = NameForm {
        id: "rename-dialog",
        heading: "cmd.rename",
        confirm: "button.rename",
        select_stem: true,
        validate: invalid_name,
    };
    let outcome = name_dialog(ctx, &form, text, fresh);
    let unchanged = *text == file_name(path);
    let same_row = explorer.entry_path(row).as_ref() == Some(path);
    if outcome == NameOutcome::Confirmed && same_row && !unchanged {
        explorer.rename(row, text);
    }
    outcome.closes()
}

fn rename_bookmark(
    ctx: &Context,
    explorer: &mut Explorer,
    index: usize,
    path: &PathBuf,
    text: &mut String,
    fresh: &mut bool,
) -> bool {
    let form = NameForm {
        id: "rename-bookmark-dialog",
        heading: "cmd.rename_bookmark",
        confirm: "button.rename",
        select_stem: false,
        validate: blank_label,
    };
    let outcome = name_dialog(ctx, &form, text, fresh);
    let same_bookmark = explorer.bookmarks().get(index).map(|b| &b.path) == Some(path);
    if outcome == NameOutcome::Confirmed && same_bookmark {
        explorer.rename_bookmark(index, text);
    }
    outcome.closes()
}

fn new_folder(ctx: &Context, explorer: &mut Explorer, text: &mut String, fresh: &mut bool) -> bool {
    let form = NameForm {
        id: "new-folder-dialog",
        heading: "cmd.new_folder",
        confirm: "button.create",
        select_stem: false,
        validate: invalid_name,
    };
    let outcome = name_dialog(ctx, &form, text, fresh);
    if outcome == NameOutcome::Confirmed {
        explorer.create_folder(text);
    }
    outcome.closes()
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

fn search_root(ctx: &Context, explorer: &mut Explorer) -> bool {
    modal(ctx, "search-root-dialog", |ui| {
        ui.heading(t("search.root_notice.title"));
        ui.add_space(6.0);
        let root = explorer.active().path().display().to_string();
        ui.label(tr("search.root_notice.body", &[("root", &root)]));
        let (mut go, mut silence) = (false, false);
        button_row(ui, |ui| {
            let proceed = ui.button(t("search.root_notice.continue"));
            silence = ui.button(t("search.root_notice.dont_show")).clicked();
            default_focus(ui, &proceed);
            go = proceed.clicked();
        });
        if go || silence {
            explorer.acknowledge_root_notice(silence);
        }
        go || silence
    })
}

fn properties(ctx: &Context, explorer: &Explorer) -> bool {
    let details = match explorer.details() {
        Some(DetailsState::Loading) => None,
        Some(DetailsState::Ready(details)) => Some(details),
        Some(DetailsState::Gone) | None => return true,
    };
    modal(ctx, "properties-dialog", |ui| {
        ui.heading(t("cmd.properties"));
        ui.add_space(6.0);
        match details {
            Some(details) => property_grid(ui, details),
            None => {
                ui.spinner();
            }
        }
        let mut close = false;
        button_row(ui, |ui| {
            let button = ui.button(t("button.close"));
            default_focus(ui, &button);
            close = button.clicked();
        });
        close
    })
}

fn property_grid(ui: &mut Ui, details: &EntryDetails) {
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
            property_row(ui, "prop.name", &details.name);
            dim_label(ui, t("prop.path"));
            // A `&str` buffer is read-only but still selectable and copyable.
            ui.add(TextEdit::singleline(&mut path.as_str()).desired_width(260.0));
            ui.end_row();
            property_row(ui, "prop.type", kind_label(details.kind));
            property_row(ui, "prop.size", &size);
            property_row(ui, "prop.modified", &modified);
            property_row(ui, "prop.permissions", &details.permissions);
        });
}

fn property_row(ui: &mut Ui, label_key: &'static str, value: &str) {
    dim_label(ui, t(label_key));
    ui.label(value);
    ui.end_row();
}

fn kind_label(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Dir => t("kind.folder"),
        EntryKind::File => t("kind.file"),
        EntryKind::Symlink => t("kind.symlink"),
        EntryKind::Other => t("kind.misc"),
    }
}

fn dim_label(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).color(theme::dim()));
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}
