# Copy engine

Copy and move run in `fx-core` (`crates/fx-core/src/copy/`), with one OS backend per platform family in `crates/fx-core/src/sys/` (`unix_copy.rs`, `windows_copy.rs`, `std_copy.rs`).
The GUI paste and `fx cp` / `fx mv` both use it.

## Plan

`CopyPlan::for_pair(src_volume, dst_volume, op, cores)` is a pure function, tested row by row against the selection matrix.
`VolumeInfo` (from `volume_info(path)`) gives the volume id, the filesystem (`FsKind`), the storage kind (SSD, HDD, network, removable) and the filesystem capabilities (`FsCaps`).
Rules apply in this order:

1. A move on the same volume renames each top-level item and copies no data.
2. If either side is a network share, 16 workers keep requests in flight.
3. A clone-capable pair (btrfs, XFS, bcachefs, ZFS on Linux; one APFS volume on macOS; one ReFS or Dev Drive volume on Windows) clones first.
4. If either side is an HDD, the TwoPhase schedule copies with one data worker in disk order.
5. If either side is removable, 2 workers avoid thrashing a USB stick.
6. Otherwise SSD defaults apply: `clamp(2 * cores, 4, 32)` workers (`clamp(2 * cores, 8, 32)` on Windows).

Each file then runs through the plan's ordered method chain:

| OS | Chain |
|----|-------|
| Linux | `FICLONE` (clone pairs) -> `copy_file_range` -> `sendfile` (cross filesystem) -> read/write |
| macOS | `fclonefileat` (same APFS volume) -> read/write with `fcopyfile` metadata |
| Windows | `CopyFile2` -> `FSCTL_DUPLICATE_EXTENTS_TO_FILE` (ReFS) -> read/write |

A method that fails as unsupported (`EXDEV`, `EINVAL`, `EOPNOTSUPP`, ...) is disabled for the volume pair and the chain moves on.
A wrong plan is therefore only slower, never incorrect.

## Schedules

Streaming (SSD, network, removable) fuses the walk and the copy on a dedicated thread pool per transfer.
Directory tasks hold the source and destination directory handles, and on Linux and macOS every file operation is relative to them (`openat`, `mkdirat`, `linkat`).
Files are handed out in tasks of 64, and a task that has copied 8 MiB hands its remaining files to a new task.

TwoPhase (HDD) walks the whole tree and creates the destination folders first, so the totals are exact and a free-space shortfall is reported before any data moves.
It then copies the files one at a time sorted by the physical position of their first extent (`FIEMAP`, `FSCTL_GET_RETRIEVAL_POINTERS`), or by inode where that is unknown.
On one disk, small files are read in batches of up to 64 MiB and written afterwards, which trades head seeks for sequential runs.

## Correctness

- Destinations are created exclusively (`O_EXCL`, `COPY_FILE_FAIL_IF_EXISTS`), so an existing entry is never overwritten by accident.
- Conflict policies: ask, skip, overwrite, keep both (`name (2)`), overwrite if newer, skip identical.
- Overwrite writes a uniquely named `.<name>.<pid>-<n>.fx-part` next to the target and renames it over the target, so a failed overwrite keeps the old file.
- Folders merge into existing folders, which keep their own mode and times (except with skip identical, which treats the destination as an earlier run of the same copy).
- A source and destination that are the same file under two paths (bind mounts, hardlinks) are detected by inode: a copy duplicates it as `name (2)`, a move leaves it alone.
- Permissions, owner (as root), nanosecond times, xattrs and POSIX ACLs (Linux), `fcopyfile` metadata (macOS), and attributes, ADS and times (Windows, via `CopyFile2`) are copied.
- The mtime is set last, so an interrupted file never looks complete; re-running with skip identical rewrites exactly the incomplete files.
- Symlinks are recreated, never followed; hardlinked files stay hardlinked (Linux and macOS); sparse files keep their holes.
- Folders are created as `0700` and get their final mode and times after their children, so read-only source folders copy fine.
- A cross-volume move deletes a source item only after its copy finished with no error and no skipped entry and after a durability barrier on every destination device (`syncfs` on Linux, per-file `fsync` plus `F_FULLFSYNC` on macOS, per-file `FlushFileBuffers` on Windows).
- Even then each source entry is deleted only if its copy matches in kind, size, mtime and link target, so files added or changed in the source during the move stay.
- Cancel stops within one 16 MiB chunk per worker and removes the files that were in flight.
- Per-item errors never abort the job; transient network errors retry three times, and a full disk pauses the job until retry, skip or cancel.

## Progress

Workers update atomic counters; the GUI samples them ten times a second.
Totals grow while the walk is running ("at least"), so the ETA appears once the walk is complete.
It takes the slower of the byte-rate and the file-rate estimate, because small-file jobs are bound by files per second.

## Benchmarks

`bench/copy/gen.py` generates the datasets of issue #15 with a fixed seed, and `bench/copy/run.sh` runs `fx cp` against `cp -a`, `xcp`, `fcp` and `rsync` with `hyperfine`.
Use the `--storage`, `--workers`, `--method` and `--schedule` flags of `fx cp` to compare plans on the same hardware.
