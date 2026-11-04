# Development

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

## Build info

The About dialog (toolbar button or `F1`) shows the version, build hash, build date and author.
`crates/fx-gui/build.rs` resolves the hash from the env var `FX_GIT_HASH`, then `git rev-parse --short=12 HEAD`, then `unknown`.
Local builds append `-dirty` when the tree has uncommitted changes.
The date comes from `FX_BUILD_DATE`, then `SOURCE_DATE_EPOCH`, then the current UTC date.
Without `.git` (for example in Docker, where `.dockerignore` excludes it) the hash is `unknown` unless `FX_GIT_HASH` is set.
`release.yml` computes both values once in the `meta` job and passes them to all platform builds, as `--build-arg` for Docker and as env vars for macOS.
To override locally: `FX_GIT_HASH=abc123 FX_BUILD_DATE=2026-01-01 cargo build -p fx-gui`.

## Releases

Bump `version` in the workspace `Cargo.toml`, commit, then tag `vX.Y.Z`.
`.github/workflows/release.yml` refuses a tag that does not match the version and publishes Linux (tar.gz, .deb, AppImage), Windows (zip) and macOS (dmg, unsigned) assets.
