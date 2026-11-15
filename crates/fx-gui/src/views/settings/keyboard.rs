//! The Keyboard page: the command list with its shortcuts and the strip that edits them.

use super::layout::note;
use super::{BUTTON_MIN_WIDTH, CONTROL_HEIGHT, ROW_GAP, State, TABLE_ROW};
use crate::command::{self, Command, Def, Shortcut};
use crate::theme;
use eframe::egui::{
    self, Align, Context, Event, Key, Layout, Modifiers, RichText, Sense, TextEdit, Ui,
};
use egui_extras::{Column, TableBuilder, TableRow};
use fx_app::t;

/// A key combination pressed this frame: `Some(None)` for a bare Esc (cancel). The key events are consumed.
fn capture(ctx: &Context) -> Option<Option<Shortcut>> {
    ctx.input_mut(|i| {
        let (key, m) = i.events.iter().find_map(pressed_key)?;
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

/// The key and modifiers of a key press event.
fn pressed_key(e: &Event) -> Option<(Key, Modifiers)> {
    match e {
        Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => Some((*key, *modifiers)),
        _ => None,
    }
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

/// The commands clicked in the list this frame.
#[derive(Default)]
struct Clicks {
    clicked: Option<Command>,
    double_clicked: Option<Command>,
}

impl Clicks {
    fn record(&mut self, command: Command, response: &egui::Response) {
        if response.double_clicked() {
            self.double_clicked = Some(command);
        } else if response.clicked() {
            self.clicked = Some(command);
        }
    }
}

/// The Keyboard list (Action | Shortcut, grouped) with the edit strip under it.
/// `table_height` is the height of the scrolling list.
pub(super) fn keyboard_section(
    ui: &mut Ui,
    st: &mut State,
    commands: &[&'static Def],
    table_height: f32,
) {
    finish_recording(ui.ctx(), st);
    let lines = lines(commands);
    let clicks = egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, theme::panel_hi()))
        .corner_radius(4)
        .show(ui, |ui| keyboard_table(ui, st, &lines, table_height))
        .inner;
    select(st, &clicks);
    ui.add_space(ROW_GAP);
    edit_strip(ui, st);
    ui.add_space(4.0);
    note(ui, t("settings.keyboard_note"), 0.0);
}

/// Stores the key combination pressed while recording; Esc only stops the recording.
fn finish_recording(ctx: &Context, st: &mut State) {
    let Some(command) = st.recording else {
        return;
    };
    let Some(captured) = capture(ctx) else {
        return;
    };
    if let Some(shortcut) = captured {
        st.draft
            .keymap
            .insert(command.def().id.to_owned(), vec![shortcut.id()]);
    }
    st.recording = None;
}

/// Selects the clicked command; a double click also starts recording it.
fn select(st: &mut State, clicks: &Clicks) {
    let Some(command) = clicks.double_clicked.or(clicks.clicked) else {
        return;
    };
    if st.selected != Some(command) {
        st.recording = None;
    }
    st.selected = Some(command);
    if clicks.double_clicked.is_some() {
        st.recording = Some(command);
    }
}

/// The scrolling Action | Shortcut table.
fn keyboard_table(ui: &mut Ui, st: &State, lines: &[Line], table_height: f32) -> Clicks {
    let mut clicks = Clicks::default();
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
        .header(TABLE_ROW, header_row)
        .body(|body| {
            body.rows(TABLE_ROW, lines.len(), |mut row| {
                let line = &lines[row.index()];
                table_row(&mut row, line, st, &mut clicks);
            });
        });
    clicks
}

fn header_row(mut header: TableRow<'_, '_>) {
    for key in ["settings.keyboard_action", "settings.keyboard_shortcut"] {
        header.col(|ui| {
            ui.add(
                egui::Label::new(RichText::new(t(key)).strong().color(theme::header_text()))
                    .selectable(false),
            );
        });
    }
}

/// One table line: a dim group header, or a command with its shortcuts.
fn table_row(row: &mut TableRow<'_, '_>, line: &Line, st: &State, clicks: &mut Clicks) {
    match line {
        Line::Group(group) => {
            row.col(|ui| {
                ui.add(
                    egui::Label::new(RichText::new(t(group)).strong().color(theme::dim()))
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
            clicks.record(d.command, &row.response());
        }
    }
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

/// What the edit strip's field shows: the prompt while recording, else the selected command's shortcuts.
fn shown_text(st: &State, selected: Option<&Def>) -> String {
    if st.recording.is_some() {
        return t("settings.keyboard_press").to_owned();
    }
    let Some(d) = selected else {
        return String::new();
    };
    command::shortcuts(d.command, &st.draft.keymap)
        .iter()
        .map(Shortcut::display)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Acts on the selected row: a read-only field plus Record, Clear, Reset and Reset all.
fn edit_strip(ui: &mut Ui, st: &mut State) {
    let selected = st.selected.map(Command::def);
    let mut shown = shown_text(st, selected);
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
