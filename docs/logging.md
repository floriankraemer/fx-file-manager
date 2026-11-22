# Logging

The desktop app writes a plain-text diagnostic log.
It is meant for bug reports: attach the files when something went wrong.

## Location

| OS | Folder |
|----|--------|
| Linux | `$XDG_STATE_HOME/fx/logs`, else `~/.local/state/fx/logs` |
| macOS | `~/Library/Logs/fx` (visible in Console.app) |
| Windows | `%LOCALAPPDATA%\fx\logs` |

The first line of every session names the version, git hash, OS and the log file path.

## Size limits

The current file is `fx.log`.
At 2 MiB it is rotated to `fx.1.log`, and older files shift up to `fx.3.log`; the oldest is deleted.
The folder therefore never holds more than 8 MiB of log, however much is logged.
If rotation fails, the current file is truncated; if that fails too, logging stops.
A single record is cut at 8 KiB.
A failed write never stops or slows the app.

## Levels

`FX_LOG=error|warn|info|debug|trace|off` sets the level for fx's own crates; the default is `info`.
Dependencies (eframe, winit, wgpu, zbus) log at `warn` or quieter.
Debug builds also echo every line to stderr.

- `error`: an operation failed in a way the user must know about, and panics.
- `warn`: something failed or degraded, e.g. a setting could not be saved or a copy method was disabled.
- `info`: rare lifecycle events: startup, transfers started and finished, settings saved.
- `debug`: expected failures and timing, e.g. unreadable folders during a search.

Never log at `info` or above per entry, per chunk or per frame.
Use `debug`, aggregate per folder or per operation, or log once per key.

## Panics

Release builds abort on panic.
A panic hook writes the message, thread and backtrace to the log before the process ends.

## Privacy

The log stays on the local disk and is never sent anywhere.
It contains file paths, so review it before attaching it to a public issue.
