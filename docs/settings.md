# Settings

The GUI stores its settings in `<config dir>/fx/settings.toml`.
On Linux that is `$XDG_CONFIG_HOME/fx/settings.toml`, or `~/.config/fx/settings.toml` when `XDG_CONFIG_HOME` is unset.
On Windows it is `%APPDATA%\fx\settings.toml`.
A missing file means defaults.

## Settings dialog

Settings opens in its own window (960x640, resizable), with a search field and category list on the left.
The categories are General, Appearance, Fonts, Listing, Behavior, Copy and move and Keyboard.
Pages are made of framed groups with a two-column label/control form.
Checkboxes carry their own label in the right column.
OK, Cancel and Apply sit at the bottom right.
The Keyboard page is an Action/Shortcut list grouped by category, with an edit strip (Record, Clear, Reset, Reset all) below it.
The search field filters the settings by name and shows the matches grouped by category.

## Language

- `language` (default `"system"`): language of the user interface.
  Values are `"system"`, `"en"`, `"de"`, `"uk"`, `"es"`, `"fr"` and `"zh-CN"`.

`"system"` follows the operating system locale and uses English when that language is not available.
The picker in the General category lists each language under its own name.
A change applies immediately, without a restart.
The German, Ukrainian, Spanish, French and Simplified Chinese texts are machine translated.
See [translating](translating.md) for how to fix or add a translation.

## Appearance

- `theme` (default `"nord-dark"`): id of the active theme.
  See [themes](themes.md) for the built-in themes and the format of user themes.
  The old values `"dark"` and `"light"` are still read and map to `nord-dark` and `nord-light`.

## Fonts

- `font_family` (optional string): name of a system font used for the whole UI.
  Unset keeps the built-in font.
- `list_font_size` (default `13.5`): font size of file list rows.
- `tree_font_size` (default `13.5`): font size of folder tree rows.

Font sizes range from 8 to 32.
Values outside that range are clamped on load.

## Listing

`folder_order` sets where folders sort relative to files:

- `"top"` (default): folders before files.
- `"mixed"`: folders and files sorted together.
- `"bottom"`: folders after files, as in Finder.

`date_format` sets how the Modified column and tooltips write dates:

- `"locale"` (default): the order of the UI language, for example `04.11.2026 14:30` in German and Ukrainian, `04/11/2026 14:30` in Spanish and French, `2026年11月4日 14:30` in Chinese.
  English keeps `2026-11-04 14:30`.
- `"iso"`: `2026-11-04 14:30` in every language.

Sizes, numbers and timings follow the UI language too (decimal separator, digit groups, unit names such as `Ko` in French).
Sizes always use 1024-based steps, and sorting uses raw bytes.

## Copy and move

- `conflict_policy` (default `"ask"`): what a paste does when an item with the same name exists.
  Values: `"ask"`, `"skip"`, `"overwrite"`, `"keep-both"`, `"overwrite-if-newer"`, `"skip-identical"`.
  Folders always merge.
- `copy_workers` (default `0`): copy workers per transfer.
  `0` lets the copy engine choose per storage type, see [Copy engine](copy-engine.md).
- `verify_copies` (default `false`): read every copied file again and compare its xxh3 checksum with the source.
  A mismatch removes the bad copy and is reported as an error for that item; a move then keeps its source.
  See [Copy engine](copy-engine.md) for the overhead.

## Keyboard

The Keyboard page lists every command with its shortcuts.
**Record** captures the next key combination and replaces the shortcuts of that command, **Unbind** removes them, and **Reset** restores the default.
Esc cancels a recording.
A command that shares a shortcut with another one shows a warning; both stay bound, and the more specific modifiers win.
Changes apply with OK or Apply.

Only changes are stored, in a `[keymap]` table of command id to a list of shortcuts.
An empty list unbinds the command.

```toml
[keymap]
rename = ["f2", "ctrl+r"]
open_settings = []
```

A shortcut is `+`-separated, case-insensitive, with the modifiers `ctrl`, `shift` and `alt` and the key last, for example `ctrl+shift+n`, `alt+left`, `f5` or `ctrl+comma`.
Unknown command ids and unreadable shortcuts are ignored and shown in a notice at startup.
Plain keys (no Ctrl or Alt) do not fire while a text field has focus.

| Command id | Default shortcuts |
| --- | --- |
| `new_tab` | `ctrl+t` |
| `close_tab` | `ctrl+w` |
| `next_tab` | `ctrl+tab` |
| `prev_tab` | `ctrl+shift+tab` |
| `refresh` | `f5` |
| `back` | `alt+left`, `backspace` |
| `forward` | `alt+right` |
| `up` | `alt+up` |
| `move_down` | `down` |
| `move_up` | `up` |
| `open_selected` | `enter` |
| `rename` | `f2` |
| `delete` | `delete` |
| `delete_permanent` | `shift+delete` |
| `select_all` | `ctrl+a` |
| `copy` | `ctrl+c` |
| `cut` | `ctrl+x` |
| `paste` | `ctrl+v` |
| `new_folder` | `ctrl+shift+n` |
| `properties` | `alt+enter` |
| `open_settings` | `ctrl+comma` |
| `about` | `f1` |

Menus and tooltips show the first shortcut of each command.

## Example

```toml
font_family = "Noto Sans"
list_font_size = 14.0
tree_font_size = 12.0
folder_order = "top"
theme = "nord-dark"
language = "system"
conflict_policy = "ask"
copy_workers = 0
verify_copies = false
```
