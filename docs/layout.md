# Layout

- `crates/fx-core`: library with the listing domain, storage detection and strategy, the `list_dir` and `walk` use cases, and per-OS backends in `src/sys`.
- `crates/fx-thumbs`: thumbnails from the OS cache (freedesktop cache and tumbler on Linux, the Windows shell, QuickLook on macOS) with in-process image decoding as fallback.
- `crates/fx-app`: UI-agnostic explorer logic (tabs, history, sorting, filtering, file operations).
- `crates/fx-gui`: egui/eframe desktop app, binary `fx-gui`.
- `crates/fx-cli`: console app, binary `fx`.
- `crates/icon-theme`: embedded Material icon theme for the GUI.
  Icons the theme lacks come from Lucide, see `icons.md`.

## Recursive search

Ctrl+F or Ctrl+E opens a search bar between the toolbar and the list and roots a search at the active tab's folder.
Every keystroke cancels the running walk and starts a new one; there is no index.

Module map, from the bottom layer up:

- `fx-core` `search/query.rs` decides what matches: substring or glob, ASCII case folding unless match case is on, and the byte range to highlight.
- `fx-core` `search/mod.rs` runs `walk_cancellable` with a visitor.
  It sends one `Hits` event per folder, never empty, and one `Unreadable` event per folder that cannot be read.
- `fx-core` `walk.rs` provides `WalkControl`, which holds the cancel flag, the live counters and the storage kind of the root, and the `WalkVisitor` trait that the search implements.
- `fx-app` `search/slot.rs` holds the search of a tab and its generation.
  Events of an older generation are dropped.
- `fx-app` `search/worker.rs` runs one thread per search.
  It drops hidden hits and enforces the result cap before anything reaches the channel.
  The UI drains the channel for at most about 4 ms per frame.
- `fx-app` `search/view.rs` is the state the UI reads: hits in arrival order, unreadable folders (the first 1,000 are kept), status, elapsed time and the selection.
- `fx-app` `explorer/search.rs` holds the use cases: start, restart on setting changes, cancel, clear, open, reveal and the root notice.
- `fx-gui` `views/search/` draws the bar, the results, the status line, the unreadable list and the root notice.
  Views raise a `SearchAction`, and `execute` applies it to the `Explorer`.

Behaviour:

- Results stream in unsorted, in walk order.
  Enter or a double click opens a result: a folder is entered, a file is opened.
  Ctrl+Enter or "Reveal in folder" goes to the parent and selects the item.
- Matches are highlighted in the name column.
- Esc closes the unreadable list if it is open.
  Otherwise it stops a running search and keeps its results.
  Esc again clears the search and restores the folder view.
- Plain text matches the file name as a substring.
  `*` and `?` make it a wildcard match.
  The `glob_scope` setting decides whether the wildcard matches the file name or the path relative to the search folder.
- The `Aa` button turns match case on and off, stored as `search.case_sensitive`.
  Off, matching ignores ASCII case.
- The `* ?` chip is lit while the query is a wildcard.
- The `network` chip and a note show when the searched folder is on a network share.
  The note says the search can take a while, and the Stop button turns red.
  The walk detects the storage kind of its root when it starts, and the view reads it from there.
- The status line shows the state, the match count, the folders walked, the elapsed time and the unreadable count.
  The unreadable count is a link to a list of those folders, with their reasons.
  Past 1,000 folders the list says how many are not shown.
- When the result cap stops the search, the status reads "Stopped at limit" and the list says how many matches it shows.
- When nothing matches, the empty view names the query and says how many folders were searched and how long that took.
- The first search from a filesystem root, such as `/` or `C:\`, waits for a notice.
  The notice shows once per session.
  "Don't show again" turns `warn_root` off.
- Changing show hidden, match case, the wildcard scope or the result limit restarts the searches that are open in all tabs.
- The result limit, the wildcard scope and the root notice are in the Search page of the settings, see `settings.md`.
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
