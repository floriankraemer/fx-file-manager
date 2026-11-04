# Settings

The GUI stores its settings in `<config dir>/fx/settings.toml`.
On Linux that is `$XDG_CONFIG_HOME/fx/settings.toml`, or `~/.config/fx/settings.toml` when `XDG_CONFIG_HOME` is unset.
On Windows it is `%APPDATA%\fx\settings.toml`.
A missing file means defaults.

## Settings dialog

The settings dialog has a category sidebar on the left with General, Appearance, Fonts, Listing, Behavior and Copy and move.
A search box filters the settings by name.

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

## Copy and move

- `conflict_policy` (default `"ask"`): what a paste does when an item with the same name exists.
  Values: `"ask"`, `"skip"`, `"overwrite"`, `"keep-both"`, `"overwrite-if-newer"`, `"skip-identical"`.
  Folders always merge.
- `copy_workers` (default `0`): copy workers per transfer.
  `0` lets the copy engine choose per storage type, see [Copy engine](copy-engine.md).

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
```
