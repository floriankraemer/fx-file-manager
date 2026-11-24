# Console app

The `fx` binary lists, walks, copies and moves from the command line.

```
fx [PATH]  [--names] [--no-sort] [--backend native|std] [--storage ssd|hdd|network|removable] [--quiet]
fx walk [PATH] [--print] [--backend native|std] [--storage ssd|hdd|network|removable] [--quiet]
fx find PATH PATTERN [--relative] [--case-sensitive] [--backend native|std] [--storage ssd|hdd|network|removable] [--quiet]
fx cp SRC... DEST [--conflict POLICY] [--storage KIND] [--workers N] [--method METHOD] [--schedule SCHEDULE] [--quiet]
fx mv SRC... DEST [same options as cp]
```

## Find

`fx find PATH PATTERN` prints the paths below `PATH` whose name matches `PATTERN`, unordered, as the walk produces them.
Plain text is a substring, matched case-insensitively (ASCII letters only) unless `--case-sensitive` is given.
A pattern with `*` or `?` is a wildcard match on the name, or on the path relative to `PATH` with `--relative`; the same case rule applies.
Folders that cannot be read are reported on stderr as `unreadable: PATH: ERROR` and the walk goes on.
The summary line reports matches, folders walked, the number of unreadable folders, the time to the first result and the total time.
`--quiet` skips the match output and the unreadable-folder lines, which is what the benchmarks use.

## Copy and move

`fx cp` follows `cp -a`: with an existing folder as `DEST` every source is copied into it, and with a single source and a new `DEST` the copy gets that name.
`fx mv` renames on the same volume and otherwise copies, flushes and then deletes each source that was copied completely.
See [Copy engine](copy-engine.md) for how the plan is chosen.

- `--conflict skip|overwrite|keep-both|overwrite-if-newer|skip-identical` (default `skip`): what to do with existing entries.
  Folders always merge.
  `skip-identical` resumes an interrupted copy.
- `--storage`, `--workers`, `--method`, `--schedule`: override the plan, for benchmarks.
  `--method` forces a single data method with no fallback (`ficlone`, `copy-file-range`, `sendfile`, `clonefileat`, `copyfile2`, `duplicate-extents`, `read-write`).
  `--schedule` is `streaming`, `two-phase` (disk order by extent) or `two-phase-inode`.
- `--quiet` hides the progress line on stderr.

The summary line reports files, bytes, clones, skips, the time and the plan used.
The exit code is non-zero if any item failed.
