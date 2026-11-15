# Changelog

All notable changes to fx are listed here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

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

[0.2.0]: https://github.com/floriankraemer/fx-file-manager/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/floriankraemer/fx-file-manager/releases/tag/v0.1.0
