# Layout

- `crates/fx-core`: library with the listing domain, storage detection and strategy, the `list_dir` and `walk` use cases, and per-OS backends in `src/sys`.
- `crates/fx-app`: UI-agnostic explorer logic (tabs, history, sorting, filtering, file operations).
- `crates/fx-gui`: egui/eframe desktop app, binary `fx-gui`.
- `crates/fx-cli`: console app, binary `fx`.
- `crates/icon-theme`: embedded Material icon theme for the GUI.
  Icons the theme lacks come from Lucide, see `icons.md`.

## Per-OS listing backends

- Linux: `getdents64` and `statx` via rustix.
- Windows: `GetFileInformationByHandleEx` with `FileIdBothDirectoryInfo`.
- macOS: `getattrlistbulk`.
- `std_fs` is the portable baseline and the test oracle.

## Places and drives

On Windows, installed WSL distros are listed in their own WSL sidebar group, read from the `Lxss` registry key.
Each entry opens `\\wsl.localhost\<distro>`.
Docker's helper distros are hidden.
WSL paths are treated as network storage.
Dotfiles are not hidden there, since WSL sets no hidden attribute.
