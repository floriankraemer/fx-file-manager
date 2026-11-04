# Copy engine

Copy and move run in `fx-core` (`crates/fx-core/src/copy/`), with one OS backend per platform family in `crates/fx-core/src/sys/` (`unix_copy.rs`, `windows_copy.rs`, `std_copy.rs`).
The GUI paste and `fx cp` / `fx mv` both use it.

## Plan

`CopyPlan::for_pair(src_volume, dst_volume, op, cores)` is a pure function, tested row by row against the selection matrix.
`VolumeInfo` (from `volume_info(path)`) gives the volume id, the filesystem (`FsKind`), the storage kind (SSD, HDD, network, removable) and the filesystem capabilities (`FsCaps`).
Rules apply in this order:

1. A move on the same volume renames each top-level item and copies no data.
2. If either side is a network share, 16 workers keep requests in flight.
3. A clone-capable pair (btrfs, XFS, bcachefs, ZFS on Linux; one volume that reports `VOL_CAP_INT_CLONE` on macOS, else one APFS volume; one ReFS or Dev Drive volume on Windows) clones first.
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
Before it starts, the streaming schedule adds up the sizes of the dense (not sparse) top-level files and warns when the destination has less free space.
Folders are not counted, because their size is only known after the walk.
It is a warning only: the job still starts, and a full disk later pauses it as before.

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
- Symlinks are recreated, never followed; hardlinked files stay hardlinked (a name that arrives while the first of its group is being copied waits for it, so workers never copy a group twice); FIFOs are recreated (Linux and macOS); sparse files keep their holes.
- Folders are created as `0700` and get their final mode and times after their children, so read-only source folders copy fine.
- A cross-volume move deletes a source item only after its copy finished with no error and no skipped entry and after a durability barrier on every destination device (`syncfs` on Linux, per-file `fsync` plus `F_FULLFSYNC` on macOS, per-file `FlushFileBuffers` on Windows).
- Even then each source entry is deleted only if its copy matches in kind, size, mtime and link target, so files added or changed in the source during the move stay.
- Cancel stops within one 16 MiB chunk per worker and removes the files that were in flight.
- Per-item errors never abort the job; transient network errors retry three times, and a full disk pauses the job until retry, skip or cancel.

## Windows hardlinks and junctions

Windows reports the link count and file ID only through a handle.
The backend opens each source file once with attribute-only access (`GetFileInformationByHandle`), so every file costs one extra cheap open.
The file ID (`GetFileInformationByHandleEx`, `FileIdInfo`) is read only for files with more than one name.
Files with a 128-bit ID (ReFS) are never grouped, so two different files cannot be linked by mistake; they are copied as separate files.
Later names of a group become hardlinks of the first copy, for copies and for moves across volumes.
If linking fails (for example the NTFS limit of 1023 links), the data is copied instead.

Junctions (`IO_REPARSE_TAG_MOUNT_POINT`) are recreated as junctions with `FSCTL_SET_REPARSE_POINT`, which needs no special privilege.

| Junction target | Copy |
|----------------|------|
| A folder inside the copied tree | A junction to the same place in the copy (also when the top-level folder is renamed to `name (2)`) |
| Anywhere else (another folder, drive or share) | A junction to the same target |
| A volume mount point (`\??\Volume{...}`) or a symbolic link | Copied as a symbolic link, as before |

A cross-volume move deletes a rewritten junction only if the copy points to the rewritten target.
A destination filesystem without reparse points (FAT, exFAT) reports a per-item error.

## Progress

Workers update atomic counters; the GUI samples them ten times a second.
Totals grow while the walk is running ("at least"), so the ETA appears once the walk is complete.
It takes the slower of the byte-rate and the file-rate estimate, because small-file jobs are bound by files per second.

## Verification

`fx cp --verify`, `fx mv --verify` and the "Verify copies" setting re-read every copied file once it is complete and compare a 128-bit xxh3 hash (and the length) of the source and the destination.
A mismatch removes the bad copy and is reported as a per-item error (`verify`), so a move keeps its source.
Links, symlinks and special files are not verified.
The cost is one extra read of both sides, so a verified copy takes roughly twice as long when the data is not cached, and less when it is.
The destination is usually still in the page cache, so verification proves that the engine wrote the right bytes, not that the medium stored them.
Hashing uses the `xxhash-rust` crate (feature `xxh3` only, no further dependencies).

## Experiments

These methods are opt-in and never chosen by `CopyPlan::for_pair`; the final go or no-go decisions follow the benchmarks of issue #21 (see [benchmarks](benchmarks/copy.md)).
`fx cp --experimental NAME` tries one first and falls back to the planned chain for files it does not apply to or when the OS refuses it.
`fx cp --method NAME` forces one method without fallback, as for the other methods.

| Name | OS | Applies to | What it does |
|------|----|------------|--------------|
| `uring` | Linux, `--features uring` | Dense files up to 1 MiB | A linked `READ` -> `WRITE` pair per file in one `io_uring` submission. A missing or blocked `io_uring` (`EPERM`, `ENOSYS`) disables it for the volume pair. |
| `parallel` | Linux | Dense files of 1 GiB and more | 64 MiB ranges copied by up to 4 threads with positional `copy_file_range`. |
| `o-direct` | Linux | Dense files of 1 GiB and more | `O_DIRECT` read and write in 8 MiB aligned blocks, so no write-back stall. |
| `raw-small` | Windows | Files under 64 KiB | One read and one write plus times and attributes, instead of `CopyFile2`. It does not copy alternate data streams or security descriptors. |

Not built yet: linked `OPENAT`/`CLOSE` and registered buffers for `uring`, overlapped I/O for `parallel` on Windows.

### macOS directory clone

`clonefileat` on a whole directory would replace the per-file walk on APFS.
It is not adopted.
It needs macOS hardware to verify against the metadata, symlink and conflict rules of this engine, and to benchmark against per-file cloning.
That work is tracked under #21.
Until then the volume's `VOL_CAP_INT_CLONE` flag (`getattrlist`) only decides whether files are cloned one by one; a volume that does not report the flag falls back to the filesystem type, and the per-pair method cache still disables cloning on the first `ENOTSUP`.

## Benchmarks

`bench/copy/gen.py` generates the datasets of issue #15 with a fixed seed, and `bench/copy/run.sh` runs `fx cp` against `cp -a`, `xcp`, `fcp` and `rsync` with `hyperfine`.
Results: [benchmarks](benchmarks/copy.md).
Use the `--storage`, `--workers`, `--method`, `--experimental`, `--verify` and `--schedule` flags of `fx cp` to compare plans on the same hardware.
