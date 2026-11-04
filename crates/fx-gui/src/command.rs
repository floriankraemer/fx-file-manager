//! Named commands, their default shortcuts and the user keymap from `settings.toml`.
//!
//! `Command` is the parameterless, user-facing side of [`crate::action::Action`]: it has a stable id for
//! `[keymap]`, a title for the Keyboard page and default shortcuts. The resolved keymap is built when the
//! settings change, never per frame.

use eframe::egui::{Key, Modifiers};
use std::collections::BTreeMap;
use std::sync::RwLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    Refresh,
    Back,
    Forward,
    Up,
    MoveDown,
    MoveUp,
    OpenSelected,
    Rename,
    Delete,
    DeletePermanent,
    SelectAll,
    Copy,
    Cut,
    Paste,
    NewFolder,
    Properties,
    OpenSettings,
    About,
}

pub struct Def {
    pub command: Command,
    /// Name in `[keymap]`.
    pub id: &'static str,
    /// Catalog key of the title.
    pub title: &'static str,
    /// Catalog key of the group heading on the Keyboard page.
    pub group: &'static str,
    pub defaults: &'static [&'static str],
    /// Fires with Ctrl or Alt held while a text field has focus. Plain keys never do.
    pub while_typing: bool,
}

const fn def(
    command: Command,
    id: &'static str,
    title: &'static str,
    group: &'static str,
    defaults: &'static [&'static str],
    while_typing: bool,
) -> Def {
    Def {
        command,
        id,
        title,
        group,
        defaults,
        while_typing,
    }
}

/// Ordered by group, so the Keyboard page can start a heading whenever the group changes.
pub const COMMANDS: &[Def] = &[
    def(
        Command::NewTab,
        "new_tab",
        "command.new_tab",
        "command_group.tabs",
        &["ctrl+t"],
        true,
    ),
    def(
        Command::CloseTab,
        "close_tab",
        "command.close_tab",
        "command_group.tabs",
        &["ctrl+w"],
        true,
    ),
    def(
        Command::NextTab,
        "next_tab",
        "command.next_tab",
        "command_group.tabs",
        &["ctrl+tab"],
        true,
    ),
    def(
        Command::PrevTab,
        "prev_tab",
        "command.prev_tab",
        "command_group.tabs",
        &["ctrl+shift+tab"],
        true,
    ),
    def(
        Command::Back,
        "back",
        "command.back",
        "command_group.navigation",
        &["alt+left", "backspace"],
        true,
    ),
    def(
        Command::Forward,
        "forward",
        "command.forward",
        "command_group.navigation",
        &["alt+right"],
        true,
    ),
    def(
        Command::Up,
        "up",
        "command.up",
        "command_group.navigation",
        &["alt+up"],
        true,
    ),
    def(
        Command::Refresh,
        "refresh",
        "command.refresh",
        "command_group.navigation",
        &["f5"],
        true,
    ),
    def(
        Command::OpenSelected,
        "open_selected",
        "command.open_selected",
        "command_group.navigation",
        &["enter"],
        false,
    ),
    def(
        Command::MoveUp,
        "move_up",
        "command.move_up",
        "command_group.selection",
        &["up"],
        false,
    ),
    def(
        Command::MoveDown,
        "move_down",
        "command.move_down",
        "command_group.selection",
        &["down"],
        false,
    ),
    def(
        Command::SelectAll,
        "select_all",
        "command.select_all",
        "command_group.selection",
        &["ctrl+a"],
        false,
    ),
    def(
        Command::Rename,
        "rename",
        "command.rename",
        "command_group.file_ops",
        &["f2"],
        false,
    ),
    def(
        Command::Delete,
        "delete",
        "command.delete",
        "command_group.file_ops",
        &["delete"],
        false,
    ),
    def(
        Command::DeletePermanent,
        "delete_permanent",
        "command.delete_permanent",
        "command_group.file_ops",
        &["shift+delete"],
        false,
    ),
    def(
        Command::NewFolder,
        "new_folder",
        "command.new_folder",
        "command_group.file_ops",
        &["ctrl+shift+n"],
        false,
    ),
    def(
        Command::Properties,
        "properties",
        "command.properties",
        "command_group.file_ops",
        &["alt+enter"],
        false,
    ),
    def(
        Command::Copy,
        "copy",
        "command.copy",
        "command_group.clipboard",
        &["ctrl+c"],
        false,
    ),
    def(
        Command::Cut,
        "cut",
        "command.cut",
        "command_group.clipboard",
        &["ctrl+x"],
        false,
    ),
    def(
        Command::Paste,
        "paste",
        "command.paste",
        "command_group.clipboard",
        &["ctrl+v"],
        false,
    ),
    def(
        Command::OpenSettings,
        "open_settings",
        "command.open_settings",
        "command_group.application",
        &["ctrl+comma"],
        false,
    ),
    def(
        Command::About,
        "about",
        "command.about",
        "command_group.application",
        &["f1"],
        false,
    ),
];

impl Command {
    pub fn def(self) -> &'static Def {
        COMMANDS
            .iter()
            .find(|d| d.command == self)
            .expect("every command has a definition")
    }
}

/// A key with its modifiers, e.g. Ctrl+Shift+N.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub modifiers: Modifiers,
    pub key: Key,
}

impl Shortcut {
    /// Parses `ctrl+shift+n`: case-insensitive, `+`-separated, modifiers `ctrl`, `shift`, `alt`, key last.
    pub fn parse(text: &str) -> Option<Self> {
        let mut modifiers = Modifiers::NONE;
        let mut key = None;
        for part in text.split('+') {
            let part = part.trim().to_ascii_lowercase();
            match part.as_str() {
                "ctrl" => modifiers.ctrl = true,
                "shift" => modifiers.shift = true,
                "alt" => modifiers.alt = true,
                _ if key.is_none() => key = Some(key_from_name(&part)?),
                _ => return None,
            }
        }
        Some(Self {
            modifiers,
            key: key?,
        })
    }

    /// The canonical stored form, e.g. `ctrl+shift+n`.
    pub fn id(&self) -> String {
        self.text(|s| s.to_ascii_lowercase())
    }

    /// The form shown to the user, e.g. `Ctrl+Shift+N`.
    pub fn display(&self) -> String {
        self.text(str::to_owned)
    }

    fn text(&self, case: impl Fn(&str) -> String) -> String {
        let mut out = String::new();
        for (on, name) in [
            (self.modifiers.ctrl, "Ctrl"),
            (self.modifiers.shift, "Shift"),
            (self.modifiers.alt, "Alt"),
        ] {
            if on {
                out.push_str(&case(name));
                out.push('+');
            }
        }
        out + &case(&key_name(self.key))
    }

    fn weight(&self) -> u32 {
        u32::from(self.modifiers.ctrl)
            + u32::from(self.modifiers.shift)
            + u32::from(self.modifiers.alt)
    }
}

fn key_name(key: Key) -> String {
    match key {
        Key::Comma => "Comma".to_owned(),
        Key::Plus => "Plus".to_owned(),
        other => other.name().to_owned(),
    }
}

fn key_from_name(name: &str) -> Option<Key> {
    Key::ALL
        .iter()
        .copied()
        .find(|k| key_name(*k).eq_ignore_ascii_case(name))
}

pub struct Binding {
    pub shortcut: Shortcut,
    pub command: Command,
}

#[derive(Default)]
pub struct Keymap {
    /// Most specific modifiers first, so Ctrl+Shift+N is tried before Ctrl+N.
    pub bindings: Vec<Binding>,
}

/// Shortcut strings of `command`: the override when present, else the defaults.
fn strings(command: Command, overrides: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let d = command.def();
    overrides
        .get(d.id)
        .cloned()
        .unwrap_or_else(|| d.defaults.iter().map(|s| (*s).to_owned()).collect())
}

/// Defaults plus `overrides`. Unknown ids and unparsable shortcuts are skipped and returned as problems.
pub fn resolve(overrides: &BTreeMap<String, Vec<String>>) -> (Keymap, Vec<String>) {
    let mut problems = Vec::new();
    for (id, keys) in overrides {
        if !COMMANDS.iter().any(|d| d.id == id) {
            problems.push(id.clone());
            continue;
        }
        for k in keys.iter().filter(|k| Shortcut::parse(k).is_none()) {
            problems.push(format!("{id} = {k}"));
        }
    }
    let mut bindings: Vec<Binding> = COMMANDS
        .iter()
        .flat_map(|d| {
            strings(d.command, overrides)
                .into_iter()
                .filter_map(|s| Shortcut::parse(&s))
                .map(|shortcut| Binding {
                    shortcut,
                    command: d.command,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    bindings.sort_by_key(|b| std::cmp::Reverse(b.shortcut.weight()));
    (Keymap { bindings }, problems)
}

/// Shortcuts of `command` under `overrides`, parsed.
pub fn shortcuts(command: Command, overrides: &BTreeMap<String, Vec<String>>) -> Vec<Shortcut> {
    strings(command, overrides)
        .iter()
        .filter_map(|s| Shortcut::parse(s))
        .collect()
}

/// Other commands that share a shortcut with `command`.
pub fn conflicts(command: Command, overrides: &BTreeMap<String, Vec<String>>) -> Vec<Command> {
    let mine = shortcuts(command, overrides);
    COMMANDS
        .iter()
        .map(|d| d.command)
        .filter(|c| *c != command)
        .filter(|c| shortcuts(*c, overrides).iter().any(|s| mine.contains(s)))
        .collect()
}

/// The keymap the views read shortcut hints from; the app installs it when settings change.
static HINTS: RwLock<Vec<(Command, String)>> = RwLock::new(Vec::new());

pub fn install_hints(overrides: &BTreeMap<String, Vec<String>>) {
    let hints = COMMANDS
        .iter()
        .filter_map(|d| {
            shortcuts(d.command, overrides)
                .first()
                .map(|s| (d.command, s.display()))
        })
        .collect();
    *HINTS.write().unwrap_or_else(|e| e.into_inner()) = hints;
}

/// First shortcut of `command` for menus and tooltips; empty when unbound.
pub fn hint(command: Command) -> String {
    HINTS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(c, _)| *c == command)
        .map(|(_, s)| s.clone())
        .unwrap_or_default()
}

/// A tooltip text with the live shortcut appended, e.g. `Rename (F2)`.
pub fn tip(key: &'static str, command: Command) -> String {
    let (text, hint) = (fx_app::t(key), hint(command));
    if hint.is_empty() {
        text.to_owned()
    } else {
        format!("{text} ({hint})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.iter().map(|s| (*s).to_owned()).collect()))
            .collect()
    }

    #[test]
    fn parse_and_format_round_trip() {
        for text in [
            "ctrl+shift+n",
            "f2",
            "alt+left",
            "ctrl+comma",
            "shift+delete",
            "ctrl+tab",
        ] {
            let s = Shortcut::parse(text).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(s.id(), text);
        }
        assert_eq!(
            Shortcut::parse("Ctrl+Shift+N").unwrap().display(),
            "Ctrl+Shift+N"
        );
        assert_eq!(Shortcut::parse("ctrl+,").map(|s| s.key), None);
        assert!(Shortcut::parse("ctrl+").is_none());
        assert!(Shortcut::parse("nope").is_none());
        assert!(Shortcut::parse("a+b").is_none());
    }

    #[test]
    fn defaults_reproduce_the_hard_wired_bindings() {
        let (keymap, problems) = resolve(&BTreeMap::new());
        assert!(problems.is_empty());
        let has = |text: &str, c: Command| {
            let s = Shortcut::parse(text).unwrap();
            keymap
                .bindings
                .iter()
                .any(|b| b.shortcut == s && b.command == c)
        };
        assert!(has("ctrl+t", Command::NewTab));
        assert!(has("ctrl+w", Command::CloseTab));
        assert!(has("ctrl+tab", Command::NextTab));
        assert!(has("ctrl+shift+tab", Command::PrevTab));
        assert!(has("f5", Command::Refresh));
        assert!(has("alt+left", Command::Back));
        assert!(has("backspace", Command::Back));
        assert!(has("alt+right", Command::Forward));
        assert!(has("alt+up", Command::Up));
        assert!(has("down", Command::MoveDown));
        assert!(has("up", Command::MoveUp));
        assert!(has("enter", Command::OpenSelected));
        assert!(has("f2", Command::Rename));
        assert!(has("delete", Command::Delete));
        assert!(has("shift+delete", Command::DeletePermanent));
        assert!(has("ctrl+a", Command::SelectAll));
        assert!(has("ctrl+c", Command::Copy));
        assert!(has("ctrl+x", Command::Cut));
        assert!(has("ctrl+v", Command::Paste));
        assert!(has("ctrl+shift+n", Command::NewFolder));
        assert!(has("alt+enter", Command::Properties));
        assert!(has("ctrl+comma", Command::OpenSettings));
        assert!(has("f1", Command::About));
        assert_eq!(keymap.bindings.len(), 23);
    }

    #[test]
    fn overrides_rebind_add_and_unbind() {
        let o = map(&[("rename", &["f3", "ctrl+r"]), ("about", &[])]);
        let (keymap, problems) = resolve(&o);
        assert!(problems.is_empty());
        let keys = |c| {
            shortcuts(c, &o)
                .iter()
                .map(Shortcut::id)
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(Command::Rename), ["f3", "ctrl+r"]);
        assert!(keys(Command::About).is_empty());
        assert!(!keymap.bindings.iter().any(|b| b.command == Command::About));
    }

    #[test]
    fn bad_entries_are_reported_and_skipped() {
        let o = map(&[("nonsense", &["f9"]), ("copy", &["ctrl+c", "ctrl+bogus"])]);
        let (keymap, problems) = resolve(&o);
        assert_eq!(problems, ["copy = ctrl+bogus", "nonsense"]);
        assert!(keymap.bindings.iter().any(|b| b.command == Command::Copy));
    }

    #[test]
    fn specific_modifiers_come_first_and_conflicts_are_found() {
        let (keymap, _) = resolve(&BTreeMap::new());
        let weights: Vec<u32> = keymap
            .bindings
            .iter()
            .map(|b| b.shortcut.weight())
            .collect();
        assert!(weights.windows(2).all(|w| w[0] >= w[1]));
        let o = map(&[("rename", &["ctrl+c"])]);
        assert_eq!(conflicts(Command::Rename, &o), [Command::Copy]);
        assert_eq!(conflicts(Command::Copy, &o), [Command::Rename]);
        assert!(conflicts(Command::Paste, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn resolving_a_large_keymap_is_fast() {
        let o: BTreeMap<_, _> = (0..100)
            .map(|i| (format!("x{i}"), vec!["f9".to_owned()]))
            .collect();
        let start = std::time::Instant::now();
        let _ = resolve(&o);
        assert!(start.elapsed() < std::time::Duration::from_millis(50));
    }
}
