# Changelog

All notable changes to fx are listed here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [0.5.0] - 2026-11-21

### Changed

- fx is now licensed under the GPL-3.0-only instead of the MIT license.
- Copies made in the same folder are named "name copy N" (#49, #50).

### Added

- WSL distros appear in their own group in the Windows sidebar.

## [0.4.0] - 2026-11-16

### Added

- Drag files out of fx into other programs, such as an email client (#46).
  Windows and macOS use the system drag and drop.
  On Linux fx acts as an X11 drag source, so it runs through X11 when a session offers both X11 and Wayland.
- Drag files between fx windows and drop files from other programs into a window's folder (#44).
- Command palette on Ctrl+Shift+P (#45).
- The fx logo in the README and as the app icon (#42).

### Fixed

- Dropping dragged files on a folder row of the listing now highlights the row and moves or copies the files into it (#47).

### Changed

- Large modules are split, and CI gates complexity and coupling (#43).

## [0.3.0] - 2026-11-15

### Added

- Bookmarks: press Ctrl+D to bookmark the current folder, or use the sidebar context menu to add, rename and remove them (#36).
  Bookmarks whose folder is missing stay in the list, dimmed.
- Right-click a folder in the file list to add it to or remove it from the bookmarks (#39).
- fx reopens the windows and tabs of the last session on startup (#36).
  The new `restore_session` setting turns this off.
  Bookmarks and the session are stored in `state.toml` next to `settings.toml`.

### Changed

- About opens in its own window, which closes with the window's close button, Esc or the Close button (#39).
- Docker release builds keep the cargo target directory between builds, so unchanged dependencies are not recompiled (#38).

### Fixed

- The close button on tabs works, and clicking a tab's title selects it (#37).
- Tabs have even padding on both sides (#37).

## [0.2.0] - 2026-11-15

### Added

- Drag entries from the file list onto a folder in the list or in the folder tree to move them (#33).
  Hold Ctrl (Option on macOS) to copy instead.
  The new `drag_copy_modifier` setting picks the key: `ctrl`, `shift` or `alt`.
- File › New Window (Ctrl+N) opens another window with its own tabs and folder tree (#31).
  All windows share settings, the clipboard and the transfers panel.
- Click beside the breadcrumbs to type or paste a path.
- Delete folders from the folder tree context menu.
- The sidebar lists network shares mapped to drive letters, including A: and B:, with their share name, for example `A: (\\server\data)` (#30).
  The new `show_share_names` setting turns the share name off (Windows only).
- File › Exit, with a bindable `exit` command that has no default shortcut (#30).
- "Open with…" in the file context menu on Windows (#30).
- Rolling `latest` pre-release built from every push to `master`, and arm64 Linux builds.

### Changed

- Menus, context menus, tabs and toolbar buttons use the same padding.
- The About button next to the filter field was removed; About stays in the Help menu and on F1.

### Fixed

- Tabs showing a folder that was deleted move to the nearest existing parent folder.

## [0.1.0] - 2026-11-04

First release.

[0.4.0]: https://github.com/floriankraemer/fx-file-manager/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/floriankraemer/fx-file-manager/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/floriankraemer/fx-file-manager/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/floriankraemer/fx-file-manager/releases/tag/v0.1.0
