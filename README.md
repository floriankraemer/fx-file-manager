# fx

Fast folder-listing core for a tabbed file explorer, plus the `fx` console app.

## Layout

- `crates/fx-core`: library with the listing domain, storage detection and strategy, the `list_dir` and `walk` use cases, and per-OS backends in `src/sys`.
- `crates/fx-app`: UI-agnostic explorer logic (tabs, history, sorting, filtering, file operations).
- `crates/fx-gui`: egui/eframe desktop app, binary `fx-gui`.
- `crates/fx-cli`: console app, binary `fx`.
- `crates/icon-theme`: embedded Material icon theme for the GUI.

Per-OS listing backends:

- Linux: `getdents64` and `statx` via rustix.
- Windows: `GetFileInformationByHandleEx` with `FileIdBothDirectoryInfo`.
- macOS: `getattrlistbulk`.
- `std_fs` is the portable baseline and the test oracle.

## Storage strategies

Storage kind is detected per path and picks the concurrency strategy.

- SSD: one thread per core for stat and walk.
- HDD: one stat thread, stats sorted by inode to cut seeks, two walk threads.
- Network (SMB, NFS, 9p, FUSE, UNC, mapped drives): 8 requests in flight with 256 KiB buffers, since latency dominates.

## Performance

- Linux reads directories with `getdents64` into one buffer per listing.
- Entry metadata is fetched with dirfd-relative `statx`, so no full paths are built.
- Names live in one byte arena and entries refer to them by offset.
- Stats fan out across threads only above 512 entries, so small folders stay serial.
- Rayon pools are cached per thread count instead of respawned per call.
- Large listings are sorted in parallel.
- Sorting and filtering operate on `u32` index vectors, not copies of entries.
- Background loads carry a generation counter, so stale results are dropped.
- Release builds use fat LTO, one codegen unit and `panic = "abort"`.

## Usage

```
fx [PATH]  [--names] [--no-sort] [--backend native|std] [--storage ssd|hdd|network] [--quiet]
fx walk [PATH] [--print] [--backend native|std] [--storage ssd|hdd|network] [--quiet]
```

## Development

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release
```

The oracle test (`crates/fx-core/tests/oracle.rs`) checks that the native backend returns the same results as the std backend.

## Builds and releases

Linux and Windows artifacts are built in Docker (`docker/Dockerfile`), so they need no local toolchain:

```
make test-container   # fmt, clippy -D warnings, tests, inside the Linux image
make e2e              # launch fx-gui under Xvfb, screenshot to dist/e2e/screenshot.png
make build-linux      # dist/linux/{fx,fx-gui}
make build-windows    # dist/windows/{fx.exe,fx-gui.exe}
make build-osx        # native, on a Mac only: dist/macos/
```

Releases: bump `version` in the workspace `Cargo.toml`, commit, then tag `vX.Y.Z`.
`.github/workflows/release.yml` refuses a tag that does not match the version and publishes Linux (tar.gz, .deb, AppImage), Windows (zip) and macOS (dmg, unsigned) assets.

## License

MIT. See [LICENSE](LICENSE).
Copyright (c) 2026 Florian Krämer.
