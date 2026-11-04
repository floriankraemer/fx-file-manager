# Development

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release
```

The oracle test (`crates/fx-core/tests/oracle.rs`) checks that the native backend returns the same results as the std backend.
The copy tests (`crates/fx-core/tests/copy.rs`) copy and move temp trees of many small files through every schedule and host method and compare the result with the source.

## Copy benchmarks

```
python3 bench/copy/gen.py /data/datasets --scale 0.1   # D1-D6 of issue #15, fixed seed
cargo build --release -p fx-cli
bench/copy/run.sh /data/datasets /mnt/target D2 D3     # hyperfine: fx vs cp -a, xcp, fcp, rsync
```

Run `run.sh` as root for cold-cache numbers; results land in `bench/copy/results/`.

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
