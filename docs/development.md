# Development

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release
```

## Quality gates

CI fails when code crosses these thresholds:

- Complexity: clippy's `excessive_nesting`, `cognitive_complexity`, `too_many_lines` and `too_many_arguments`, enabled in the workspace `Cargo.toml` with limits in `clippy.toml`.
  A nesting of 4 counts every block, so `impl { fn { for { if } } }` is the deepest a method may go.
- Coupling: `make coupling` runs [cargo-coupling](https://crates.io/crates/cargo-coupling) per crate and fails on any circular module dependency or a health grade below C.
  Install it with `cargo install --locked cargo-coupling`; settings live in `.coupling.toml`.

Fix a finding by splitting the code, not by raising a threshold or adding `#[allow]`.

The oracle test (`crates/fx-core/tests/oracle.rs`) checks that the native backend returns the same results as the std backend.
The copy tests (`crates/fx-core/tests/copy.rs`) copy and move temp trees of many small files through every schedule and host method and compare the result with the source.

## Copy benchmarks

```
python3 bench/copy/gen.py /data/datasets --scale 0.1   # D1-D6 of issue #15, fixed seed
cargo build --release -p fx-cli
bench/copy/run.sh /data/datasets /mnt/target D2 D3     # hyperfine: fx vs cp -a, xcp, fcp, rsync
```

Run `run.sh` as root for cold-cache numbers; results land in `bench/copy/results/`.

## Search benchmarks

Targets from issue #7, measured with release builds:

| Target | Goal |
|--------|------|
| Time to first result | under 50 ms on a warm SSD |
| Full walk of a 1M-file tree | within 15% of `fx walk --quiet` on the same tree (warm SSD) |
| Cancel latency | under 20 ms from cancel to the last worker stopping |

```
cargo build --release -p fx-cli
target/release/fx walk TREE --quiet
target/release/fx find TREE 'zzzqqq' --quiet     # no matches: pure matching overhead
target/release/fx find TREE '*.rs' --quiet       # many matches: adds one path allocation per match
cargo run --release -p fx-core --example search_cancel -- TREE 50
```

Numbers below are the median of 9 runs after one warm-up run, on a Ryzen 7 5800X3D (16 threads), Linux 6.18 under WSL2, ext4 on a virtual disk.
Storage detection reports `unknown` there, so the walk uses the SSD strategy (one thread per core).
All runs are warm cache; cold cache, HDD and network were not measured.

Synthetic tree: 1,000,000 empty files in 5,050 folders (200 files per folder; every 7th file ends in `.rs`, the rest in `.txt`).

| Run | Matches | Total | vs `walk --quiet` |
|-----|---------|-------|-------------------|
| `fx walk --quiet` | - | 35.9 ms | baseline |
| `fx find zzzqqq --quiet` | 0 | 37.3 ms | +4% |
| `fx find '*.rs' --quiet` | 145,000 | 52.1 ms | +45% |
| `fx find file1 --quiet` | 500,000 | 65.2 ms | +82% |

- Time to first result is 1.7 ms on the synthetic tree (occasionally about 4.4 ms).
  On `/usr` (241k files, 36k folders, a 25-match query) it is 12 to 21 ms.
- Cancel latency, from setting the flag to `search` returning with every worker stopped, over 50 runs: median 0.31 ms and worst 0.81 ms on the synthetic tree; median 0.44 ms and worst 6.1 ms on `/usr`.
- Real trees, full walk: `/usr` takes about 300 ms for both `walk --quiet` and a non-matching `find`; `~/.cargo/registry` (50k files) takes about 14 ms for `walk --quiet` and 15 ms for a non-matching `find`.

Status against the targets:

- First result: met on a warm SSD-like disk.
- Cancel: met.
- Full walk within 15%: met when few entries match (+4%), missed when many do (+45% at 14.5% of entries matching, +82% at 50%).
  A match costs one `PathBuf` allocation and a batch vector, and `walk` allocates paths for folders only.
  Realistic queries match a small share of entries, but the target is not met for match-heavy queries.
- Not measured: the Linux kernel checkout, cold cache, HDD and network.

The timings above were taken before hit paths became a single allocation, so they need to be measured again on an idle machine.

Laying out the result list takes a median of 0.32 ms per frame with 100,000 hits (release build, headless, without painting on a GPU); the goal is under 16 ms.
`cargo test --release -p fx-gui -- --ignored search_results_frame_time --nocapture` prints it.
Frame times of a real window while results stream in were not measured.

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
