# fx File Manager

A performance optimized cross platform file manager written in Rust.

## Documentation

- [Layout](docs/layout.md)
- [Settings](docs/settings.md)
- [Themes](docs/themes.md)
- [Console app](docs/console.md)
- [Copy engine](docs/copy-engine.md)
- [Development, builds and releases](docs/development.md)

## Performance Optimizations

- Linux reads directories with `getdents64` into one buffer per listing.
- Entry metadata is fetched with dirfd-relative `statx`, so no full paths are built.
- Names live in one byte arena and entries refer to them by offset.
- Stats fan out across threads only above 512 entries, so small folders stay serial.
- Rayon pools are cached per thread count instead of respawned per call.
- Large listings are sorted in parallel.
- Sorting and filtering operate on `u32` index vectors, not copies of entries.
- Background loads carry a generation counter, so stale results are dropped.
- Release builds use fat LTO, one codegen unit and `panic = "abort"`.

### File system reading strategies

Storage kind is detected per path and picks the concurrency strategy.

- SSD: one thread per core for stat and walk.
- HDD: one stat thread, stats sorted by inode to cut seeks, two walk threads.
- Network (SMB, NFS, 9p, FUSE, UNC, mapped drives): 8 requests in flight with 256 KiB buffers, since latency dominates.

### Copy and move strategies

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

## License

MIT. See [LICENSE](LICENSE).
Copyright (c) 2026 Florian Krämer.
