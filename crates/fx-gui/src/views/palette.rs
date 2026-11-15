//! Command palette: every command by name, narrowed as the user types. Opened with Ctrl+Shift+P.

use super::menu;
use crate::command::{self, COMMANDS, Command};
use eframe::egui::{Context, Id, Key, Modal, Modifiers, ScrollArea, TextEdit, Ui};
use fx_app::{Explorer, t};

const WIDTH: f32 = 480.0;
const LIST_HEIGHT: f32 = 360.0;

pub struct State {
    query: String,
    selected: usize,
    /// Focus the text field on the first frame.
    fresh: bool,
    /// Scroll the selected row into view; set by the keys, so the mouse wheel is not fought.
    reveal: bool,
}

impl State {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            selected: 0,
            fresh: true,
            reveal: false,
        }
    }
}

pub enum Outcome {
    Pending,
    Close,
    Run(Command),
}

#[derive(Default)]
struct Keys {
    up: bool,
    down: bool,
    enter: bool,
    escape: bool,
}

/// Shows the palette over the window; the caller drops `state` once the outcome is not `Pending`.
pub fn show(ctx: &Context, state: &mut State, explorer: &Explorer) -> Outcome {
    let commands = filter(&state.query);
    state.selected = state.selected.min(commands.len().saturating_sub(1));
    let response = Modal::new(Id::unique("command-palette")).show(ctx, |ui| {
        ui.set_width(WIDTH);
        content(ui, state, &commands, explorer)
    });
    match response.inner {
        Outcome::Pending if response.should_close() => Outcome::Close,
        outcome => outcome,
    }
}

/// Commands whose translated title contains `query`, case-insensitively, in registry order.
pub fn filter(query: &str) -> Vec<Command> {
    let query = query.trim().to_lowercase();
    COMMANDS
        .iter()
        .filter(|d| t(d.title).to_lowercase().contains(&query))
        .map(|d| d.command)
        .collect()
}

fn content(ui: &mut Ui, state: &mut State, commands: &[Command], explorer: &Explorer) -> Outcome {
    let keys = take_keys(ui);
    if keys.escape {
        return Outcome::Close;
    }
    move_selection(state, &keys, commands.len());
    if keys.enter
        && let Some(command) = commands.get(state.selected).copied()
        && command::enabled(command, explorer)
    {
        return Outcome::Run(command);
    }
    search(ui, state);
    if commands.is_empty() {
        ui.label(t("palette.no_match"));
        return Outcome::Pending;
    }
    list(ui, state, commands, explorer).map_or(Outcome::Pending, Outcome::Run)
}

/// Takes the navigation keys before the text field sees them.
fn take_keys(ui: &Ui) -> Keys {
    ui.input_mut(|i| Keys {
        up: i.consume_key(Modifiers::NONE, Key::ArrowUp),
        down: i.consume_key(Modifiers::NONE, Key::ArrowDown),
        enter: i.consume_key(Modifiers::NONE, Key::Enter),
        escape: i.consume_key(Modifiers::NONE, Key::Escape),
    })
}

fn move_selection(state: &mut State, keys: &Keys, len: usize) {
    let last = len.saturating_sub(1);
    let next = match (keys.up, keys.down) {
        (_, true) => (state.selected + 1).min(last),
        (true, false) => state.selected.saturating_sub(1),
        (false, false) => return,
    };
    if next != state.selected {
        state.selected = next;
        state.reveal = true;
    }
}

fn search(ui: &mut Ui, state: &mut State) {
    let response = ui.add(
        TextEdit::singleline(&mut state.query)
            .hint_text(t("palette.placeholder"))
            .desired_width(f32::INFINITY),
    );
    if response.changed() {
        state.selected = 0;
        state.reveal = true;
    }
    if std::mem::take(&mut state.fresh) {
        response.request_focus();
    }
}

/// The filtered rows; returns the command whose row was clicked. Disabled rows cannot be clicked.
fn list(
    ui: &mut Ui,
    state: &mut State,
    commands: &[Command],
    explorer: &Explorer,
) -> Option<Command> {
    let mut clicked = None;
    ScrollArea::vertical()
        .max_height(LIST_HEIGHT)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (row, &command) in commands.iter().enumerate() {
                let selected = row == state.selected;
                let enabled = command::enabled(command, explorer);
                let response = menu::item_at(
                    ui,
                    t(command.def().title),
                    &command::hint(command),
                    enabled,
                    selected,
                );
                if selected && state.reveal {
                    response.scroll_to_me(None);
                }
                if response.clicked() {
                    clicked = Some(command);
                }
            }
        });
    state.reveal = false;
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_lists_every_command() {
        assert_eq!(filter("").len(), COMMANDS.len());
        assert_eq!(filter("   ").len(), COMMANDS.len());
    }

    #[test]
    fn query_matches_titles_case_insensitively() {
        assert_eq!(filter("NEW TAB"), [Command::NewTab]);
        let tabs = filter("tab");
        for command in [
            Command::NewTab,
            Command::CloseTab,
            Command::NextTab,
            Command::PrevTab,
        ] {
            assert!(tabs.contains(&command), "{command:?}");
        }
    }

    #[test]
    fn unknown_query_matches_nothing() {
        assert!(filter("no such command").is_empty());
    }
}
