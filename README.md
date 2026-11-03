# fx

Fast folder-listing core for a tabbed file explorer, plus the `fx` console app.

## Layout

`crates/fx-core` is the library: listing domain, storage detection and strategy, `list_dir` and `walk` use cases, and per-OS backends in `src/sys`.
Linux uses getdents64 and statx via rustix.
Windows uses GetFileInformationByHandleEx with FileFullDirectoryInfo.
macOS uses getattrlistbulk.
`std_fs` is the baseline and oracle.
`crates/fx-cli` is the binary `fx`.

## Storage strategies

- SSD: all cores.
- HDD: 1 stat thread, inode-ordered, and 2 walk threads.
- Network (SMB, NFS, 9p, FUSE, UNC, mapped drives): about 8 requests in flight and bigger buffers, because network listing is latency-bound.

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

The oracle test (`crates/fx-core/tests/oracle.rs`) checks that the native backend equals the std backend.
