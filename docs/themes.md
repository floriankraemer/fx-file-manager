# Themes

A theme sets every color of the GUI, including the age-bucket colors.
Choose one in Settings, Appearance, Theme.
The choice previews live and reverts on Cancel.
The setting is stored as `theme = "<id>"` in `settings.toml`.
An unknown id falls back to `nord-dark`.
Old configs with `theme = "dark"` or `"light"` load as `nord-dark` and `nord-light`.

## Built-in themes

| Id | Name | Base |
| --- | --- | --- |
| `nord-dark` (default) | Nord Dark | dark |
| `nord-light` | Nord Light | light |
| `solarized-dark` | Solarized Dark | dark |
| `solarized-light` | Solarized Light | light |
| `dracula` | Dracula | dark |
| `gruvbox-dark` | Gruvbox Dark | dark |
| `high-contrast` | High Contrast | dark |

Unit tests require every built-in theme to reach a contrast of 4.5:1 for `text` on `bg` and `panel`, and 3:1 for `dim` on `bg`.

## User themes

Put a `*.toml` file in `<config dir>/fx/themes/`.
On Linux that is `~/.config/fx/themes/`, on Windows `%APPDATA%\fx\themes\`.
The file name without `.toml` is the theme id.
Settings has the buttons "Open themes folder" and "Reload themes".

- `name` (required): name shown in the picker.
- `base` (required): `"dark"` or `"light"`.
  It selects the starting egui visuals and the icon variant.
- Colors (all optional, `"#RRGGBB"`): a missing color is taken from Nord Dark or Nord Light, matching `base`.

Color keys:
`bg`, `panel`, `panel_hi`, `border`, `text`, `dim`, `accent`, `header_text`, `selected`, `error`, `faint`, `separator`, `selection_bg`,
`age_minutes`, `age_hours`, `age_days`, `age_weeks`, `age_months`, `age_years`.

A file that fails to parse, has an invalid color, or uses the id of a built-in theme is reported on stderr and skipped.
User themes are not contrast-checked.

## Example

```toml
name = "Midnight"
base = "dark"
bg = "#0B1020"
panel = "#121A30"
text = "#E0E6F5"
accent = "#7AA2F7"
```
