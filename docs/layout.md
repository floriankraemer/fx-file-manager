# Layout

- `crates/fx-core`: library with the listing domain, storage detection and strategy, the `list_dir` and `walk` use cases, and per-OS backends in `src/sys`.
- `crates/fx-app`: UI-agnostic explorer logic (tabs, history, sorting, filtering, file operations).
- `crates/fx-gui`: egui/eframe desktop app, binary `fx-gui`.
- `crates/fx-cli`: console app, binary `fx`.
- `crates/icon-theme`: embedded Material icon theme for the GUI.

## Per-OS listing backends

- Linux: `getdents64` and `statx` via rustix.
- Windows: `GetFileInformationByHandleEx` with `FileIdBothDirectoryInfo`.
- macOS: `getattrlistbulk`.
- `std_fs` is the portable baseline and the test oracle.
