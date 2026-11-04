# Settings

The GUI stores its settings in `<config dir>/fx/settings.toml`.
On Linux that is `$XDG_CONFIG_HOME/fx/settings.toml`, or `~/.config/fx/settings.toml` when `XDG_CONFIG_HOME` is unset.
On Windows it is `%APPDATA%\fx\settings.toml`.
A missing file means defaults.

## Settings dialog

The settings dialog has a category sidebar on the left with General, Appearance, Fonts, Listing and Behavior.
A search box filters the settings by name.

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

## Example

```toml
font_family = "Noto Sans"
list_font_size = 14.0
tree_font_size = 12.0
folder_order = "top"
```
