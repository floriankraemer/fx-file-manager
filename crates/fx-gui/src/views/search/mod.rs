//! Recursive search in a window: the bar, the result list, the status line, and the input they raise.
//!
//! Views raise [`SearchAction`]s; [`execute`] applies them to the explorer, so `App` only forwards.

mod bar;
mod results;
mod root_notice;
mod status;
mod unreadable;

pub use bar::bar;
pub use results::results;
pub use root_notice::show as root_notice;
pub use status::summary as status;

use crate::command::Command;
use eframe::egui::Context;
use fx_app::{Explorer, SearchStart, SearchView};
use std::path::PathBuf;

/// What the user asked of the search, from the bar, the result list, the status line or a shortcut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchAction {
    /// Show the bar and focus its field.
    Open,
    /// The field's text changed: (re)start the search, or clear it when blank.
    Start(String),
    /// Stop walking and keep the results.
    Stop,
    /// Drop the results and hide the bar.
    Close,
    /// Esc: close the unreadable-folders window, else stop a running search, else close.
    Escape,
    ShowUnreadable,
    ToggleCase,
    OpenHit(usize),
    OpenSelected,
    RevealHit(usize),
    RevealSelected,
    CopyHitPath(usize),
    MoveSelection(isize),
}

/// What the window has to do after an action ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Follow {
    Done,
    /// Ask the user before searching from a filesystem root.
    RootNotice,
}

/// What the bar of one window remembers between frames.
#[derive(Default)]
pub struct State {
    open: bool,
    text: String,
    focus: bool,
    /// Give the field the focus a click on a bar button took, without selecting its text.
    refocus: bool,
    /// Tab and folder the bar belongs to; showing anything else resets it to that tab's search.
    key: Option<(u64, PathBuf)>,
    scroll_to_selection: bool,
    /// A reveal asked for its item to be scrolled into view once the folder has loaded.
    reveal_scroll: bool,
    unreadable_open: bool,
}

impl State {
    /// Follows the active tab: another tab or folder shows that tab's own search, or no bar.
    fn sync(&mut self, explorer: &Explorer) {
        self.unreadable_open &= explorer.search().is_some();
        let tab = explorer.active();
        let key = (tab.id(), tab.path().to_path_buf());
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.key = Some(key);
        self.open = explorer.search().is_some();
        self.text = explorer
            .search()
            .map(|s| s.query().to_owned())
            .unwrap_or_default();
    }

    /// Shows the bar and focuses its field, selecting what is typed.
    fn open(&mut self) {
        self.open = true;
        self.focus = true;
    }

    fn close(&mut self) {
        self.open = false;
        self.text.clear();
        self.unreadable_open = false;
    }

    /// True once, when a reveal's folder has loaded and its item is selected.
    pub fn take_reveal_scroll(&mut self, explorer: &Explorer) -> bool {
        let ready = self.reveal_scroll && explorer.active().selected().is_some();
        if ready {
            self.reveal_scroll = false;
        }
        ready
    }
}

/// The search action a command raises, if any. Navigation commands act on the results while a search shows.
pub fn command_action(command: Command, explorer: &Explorer) -> Option<SearchAction> {
    let searching = explorer.search().is_some();
    match command {
        Command::Search => Some(SearchAction::Open),
        Command::CloseSearch => Some(SearchAction::Escape),
        Command::RevealSearchHit => Some(SearchAction::RevealSelected),
        Command::OpenSelected if searching => Some(SearchAction::OpenSelected),
        Command::MoveDown if searching => Some(SearchAction::MoveSelection(1)),
        Command::MoveUp if searching => Some(SearchAction::MoveSelection(-1)),
        _ => None,
    }
}

pub fn execute(
    ctx: &Context,
    explorer: &mut Explorer,
    state: &mut State,
    action: SearchAction,
) -> Follow {
    match action {
        SearchAction::Open => state.open(),
        SearchAction::Start(text) => return start(explorer, &text),
        SearchAction::Stop => explorer.cancel_search(),
        SearchAction::Close => close(explorer, state),
        SearchAction::Escape => escape(explorer, state),
        SearchAction::ShowUnreadable => state.unreadable_open = true,
        SearchAction::ToggleCase => explorer.toggle_search_case(),
        SearchAction::OpenHit(row) => explorer.open_search_hit(row),
        SearchAction::OpenSelected => open_selected(explorer),
        SearchAction::RevealHit(row) => reveal(explorer, state, row),
        SearchAction::RevealSelected => reveal_selected(explorer, state),
        SearchAction::CopyHitPath(row) => copy_path(ctx, explorer, row),
        SearchAction::MoveSelection(delta) => move_selection(ctx, explorer, state, delta),
    }
    Follow::Done
}

fn start(explorer: &mut Explorer, text: &str) -> Follow {
    match explorer.request_search(text) {
        SearchStart::Started => Follow::Done,
        SearchStart::NeedsRootNotice => Follow::RootNotice,
    }
}

fn close(explorer: &mut Explorer, state: &mut State) {
    explorer.clear_search();
    state.close();
}

fn escape(explorer: &mut Explorer, state: &mut State) {
    if std::mem::take(&mut state.unreadable_open) {
        return;
    }
    if !explorer.escape_search() {
        state.close();
    }
}

fn open_selected(explorer: &mut Explorer) {
    if let Some(row) = explorer.search().and_then(SearchView::selected) {
        explorer.open_search_hit(row);
    }
}

fn reveal(explorer: &mut Explorer, state: &mut State, row: usize) {
    explorer.reveal_search_hit(row);
    state.reveal_scroll = true;
}

fn reveal_selected(explorer: &mut Explorer, state: &mut State) {
    if let Some(row) = explorer.search().and_then(SearchView::selected) {
        reveal(explorer, state, row);
    }
}

fn copy_path(ctx: &Context, explorer: &Explorer, row: usize) {
    let hit = explorer.search().and_then(|s| s.hits().get(row));
    if let Some(hit) = hit {
        ctx.copy_text(hit.path.display().to_string());
    }
}

fn move_selection(ctx: &Context, explorer: &mut Explorer, state: &mut State, delta: isize) {
    explorer.move_search_selection(delta);
    state.scroll_to_selection = true;
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn explorer() -> Explorer {
        Explorer::new(std::env::temp_dir(), || {})
    }

    #[test]
    fn escape_closes_the_unreadable_window_before_the_bar() {
        let (ctx, mut explorer) = (Context::default(), explorer());
        let mut state = State::default();
        state.open();
        state.unreadable_open = true;
        execute(&ctx, &mut explorer, &mut state, SearchAction::Escape);
        assert!(state.open && !state.unreadable_open);
        execute(&ctx, &mut explorer, &mut state, SearchAction::Escape);
        assert!(!state.open);
    }

    #[test]
    fn moving_the_selection_asks_the_results_to_scroll() {
        let (ctx, mut explorer) = (Context::default(), explorer());
        let mut state = State::default();
        execute(
            &ctx,
            &mut explorer,
            &mut state,
            SearchAction::MoveSelection(1),
        );
        assert!(state.scroll_to_selection);
    }

    #[test]
    fn result_navigation_commands_only_apply_while_a_search_shows() {
        let explorer = explorer();
        assert_eq!(
            command_action(Command::Search, &explorer),
            Some(SearchAction::Open)
        );
        assert_eq!(command_action(Command::MoveDown, &explorer), None);
        assert_eq!(command_action(Command::OpenSelected, &explorer), None);
    }

    #[test]
    fn navigation_commands_act_on_the_results_while_a_search_shows() {
        let explorer = crate::test_support::searching_explorer();
        for (command, action) in [
            (Command::MoveDown, SearchAction::MoveSelection(1)),
            (Command::MoveUp, SearchAction::MoveSelection(-1)),
            (Command::OpenSelected, SearchAction::OpenSelected),
            (Command::RevealSearchHit, SearchAction::RevealSelected),
        ] {
            assert_eq!(command_action(command, &explorer), Some(action));
        }
    }
}
