# Icons

- Entry (file and folder) icons come from the Material icon theme in `third_party/material-icon-theme`.
- For anything the Material theme has no icon for, use a Lucide icon.
  Toolbar back, forward, up and refresh are examples.
- Do not draw new icons by hand and do not use text glyphs such as arrows, because the default font may not contain them.

## Adding a Lucide icon

1. Copy the SVG from <https://github.com/lucide-icons/lucide/tree/main/icons> into `third_party/lucide/`.
2. Add it to `lucide_svg` in `crates/fx-gui/src/views/icons.rs` and to the glyph test.
3. Keep `third_party/lucide/LICENSE` unchanged.

## Attribution

- Material icon theme: MIT license, copyright Material Extensions (PKief).
  License text in `third_party/material-icon-theme/LICENSE`.
- Lucide: ISC license, copyright Lucide Icons and Contributors.
  Some Lucide icons derive from Feather (MIT, copyright Cole Bemis).
  Both license texts are in `third_party/lucide/LICENSE`.
- Both licenses are compatible with GPL-3.0-only.
- The About window names both icon sources.
