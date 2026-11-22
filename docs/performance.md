# Performance

How fx keeps listing, sorting and copying fast.
Copy internals are in [copy-engine.md](copy-engine.md); measurements are in [benchmarks/copy.md](benchmarks/copy.md).

- Linux reads directories with `getdents64` into one buffer per listing.
- Entry metadata is fetched with dirfd-relative `statx`, so no full paths are built.
- Names live in one byte arena and entries refer to them by offset.
- Stats fan out across threads only above 512 entries, so small folders stay serial.
- Rayon pools are cached per thread count instead of respawned per call.
- Large listings are sorted in parallel.
- Sorting and filtering operate on `u32` index vectors, not copies of entries.
- Background loads carry a generation counter, so stale results are dropped.
- The UI is fully decoupled from every other process, so it stays responsive.
  Listing, copying and moving run on background threads, and the UI only samples their state.
- Recursive search reuses the parallel walk (names and kinds only, no stat), matches bytes without allocating per entry, builds one path per match and streams one batch per folder.
  Cancelling stops every worker within about a millisecond; measurements are in [development.md](development.md#search-benchmarks).
- Release builds use fat LTO, one codegen unit and `panic = "abort"`.

## File system reading strategies

Storage kind is detected per path and picks the concurrency strategy.

- SSD: one thread per core for stat and walk.
- HDD: one stat thread, stats sorted by inode to cut seeks, two walk threads.
- Network (SMB, NFS, 9p, FUSE, UNC, mapped drives): 8 requests in flight with 256 KiB buffers, since latency dominates.

## Copy and move strategies

A pure planner picks workers, schedule and an ordered method chain for each pair of source and destination volume.
Unsupported methods are disabled for the pair on first failure, so a wrong guess costs speed, never correctness.

| Pair | Methods | Workers | Schedule |
|------|---------|---------|----------|
| Same volume, move | no-replace rename per item | 1 | - |
| Clone-capable (btrfs, XFS, bcachefs, ZFS, APFS, ReFS) | clone, then in-kernel copy | 2 x cores (4 on HDD/USB) | streaming |
| Linux, SSD | `copy_file_range` -> `sendfile` -> read/write | 2 x cores, 4-32 | streaming |
| Windows, SSD | `CopyFile2` | 2 x cores, 8-32 | streaming |
| Any HDD | in-kernel copy or `CopyFile2` | 1 | walk first, copy in disk order |
| USB | in-kernel copy or `CopyFile2` | 2 | streaming |
| Network | `copy_file_range` (server-side) or `CopyFile2` | 16 | streaming |

- Walk and copy run fused on a dedicated pool per transfer, so listing in other tabs never waits for a copy.
- Linux and macOS resolve every file relative to open directory handles, never by full path.
- Progress is a set of atomic counters sampled by the UI, not a message per file.
- Overwrites go through a temp file and a rename; a move deletes its source only after a flushed, error-free copy.
