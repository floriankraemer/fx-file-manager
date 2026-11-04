# fx File Manager

A performance optimized cross platform file manager written in Rust.

## Documentation

- [Layout](docs/layout.md)
- [Settings](docs/settings.md)
- [Themes](docs/themes.md)
- [Console app](docs/console.md)
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

## License

MIT. See [LICENSE](LICENSE).
Copyright (c) 2026 Florian Krämer.
