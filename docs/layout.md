# Layout

- `crates/fx-core`: library with the listing domain, storage detection and strategy, the `list_dir` and `walk` use cases, and per-OS backends in `src/sys`.
- `crates/fx-app`: UI-agnostic explorer logic (tabs, history, sorting, filtering, file operations).
- `crates/fx-gui`: egui/eframe desktop app, binary `fx-gui`.
- `crates/fx-cli`: console app, binary `fx`.
- `crates/icon-theme`: embedded Material icon theme for the GUI.
  Icons the theme lacks come from Lucide, see `icons.md`.

## Recursive search

Ctrl+F or Ctrl+E opens a search bar between the toolbar and the list and roots a search at the active tab's folder.
Every keystroke cancels the running walk and starts a new one; there is no index.

- `fx-core` `search` runs the parallel `walk` with a cancel flag and sends one batch of hits per folder.
- `fx-app` `Explorer::start_search` runs it on a worker thread per tab.
  Batches carry a generation, so stale ones are dropped.
  Navigating the tab ends its search.
- `fx-gui` `views/search.rs` draws the bar and a result table (Name, Folder relative to the search folder, Type) that replaces the folder table while a search exists.
  Results stream in unsorted, in walk order.
- The status bar shows the state (searching, complete, stopped), match count, folders walked, unreadable folders and elapsed time.
- Enter or a double click opens a result: a folder is entered, a file is opened.
  Ctrl+Enter or "Reveal in folder" goes to the parent and selects the item.
- Esc stops a running search and keeps its results; Esc again clears it and restores the folder view.
- Plain text matches the file name as a case-insensitive substring; `*` and `?` make it a wildcard match.
- The result limit, the wildcard scope and the notice before searching from a filesystem root are in the Search page of the settings, see `settings.md`.
- `fx find` runs the same search from the console.

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
