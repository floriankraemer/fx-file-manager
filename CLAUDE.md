# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

CI (`.github/workflows/test.yml`, also called by the release workflows) runs on Linux, Windows and macOS: fmt check, clippy with `-D warnings`, tests, then a release `fx` smoke run.

```
make check                    # cargo fmt --check + clippy -D warnings + tests + coupling (host)
make coupling                 # cargo-coupling gate per crate (cargo install --locked cargo-coupling)
cargo test --workspace
cargo test -p fx-core --test oracle          # one integration test file
cargo test -p fx-app every_catalog_matches   # one test by name filter
cargo clippy --workspace --all-targets -- -D warnings
make gui ARGS=/usr            # release build + run egui app
make cli ARGS="walk /usr --quiet"   # console lister (binary `fx`)
make test-container           # fmt/clippy/tests in Linux Docker image under Xvfb
make coverage                 # cargo-llvm-cov, fails under 85% lines (fx-gui excluded)
```

Rust edition 2024, MSRV 1.95. Release profile is fat LTO, 1 codegen unit, `panic = "abort"`.
Linux and Windows release artifacts are built in Docker (`make build-linux`, `make build-windows`); macOS needs a Mac host.

## Architecture

Five-crate workspace with strict layering; dependencies point downward only.

- `fx-core`: listing domain, storage detection, copy/move engine. No UI.
  - `src/sys/{linux,windows,macos}.rs` are per-OS fast listing backends (`getdents64`+`statx`, `FileIdBothDirectoryInfo`, `getattrlistbulk`).
  - `sys/std_fs.rs` is the portable baseline and the test oracle: `tests/oracle.rs` asserts native backends match it.
  - `storage.rs`/`volume.rs` classify a path (SSD, HDD, network, USB) and that picks the concurrency strategy.
  - `copy/`: `plan.rs` is a pure planner that picks workers, schedule and an ordered method chain per source/destination volume pair; `pipeline/` executes it (one submodule per concern: preflight, conflicts, entry, streaming, two_phase, moves, links); `progress.rs` is atomic counters sampled by the UI. Methods unsupported for a pair are disabled on first failure. `tests/copy.rs` runs every schedule and method.
  - Names live in one byte arena, entries refer by offset; sorting/filtering work on `u32` index vectors. Keep these invariants when touching `list.rs`/`listing.rs`/`sort.rs`.
- `fx-app`: UI-agnostic view-model (`Explorer` owns tabs, history, `DirView` sort/filter, file ops, transfers, settings, i18n). Background loads carry a generation counter so stale results are dropped. No egui dependency.
- `fx-gui`: egui/eframe presentation only; "renders state and forwards input; holds no logic".
  - Views (`views/*`) raise `Action`s (`action.rs`); `App` (`app.rs`) executes them against `Explorer`.
  - `command.rs` maps named `Command`s to default shortcuts plus the user `[keymap]` from `settings.toml`; resolved keymap is rebuilt on settings change, not per frame.
  - Themes are TOML files in `crates/fx-gui/themes/`. `build.rs` embeds version, git hash (`FX_GIT_HASH`) and build date (`FX_BUILD_DATE`/`SOURCE_DATE_EPOCH`).
- `fx-cli`: console app, binary `fx`.
- `icon-theme`: embedded Material icon theme packed from `third_party/material-icon-theme`.

## Code quality

- Follow SOLID and Clean Code: small functions with one responsibility, at most 2 levels of control-flow nesting inside a function body, early returns over nested branches.
- CI enforces it. Complexity: clippy `excessive_nesting` (4, counting `impl`/`fn` blocks), `cognitive_complexity` (15), `too_many_lines` (80), `too_many_arguments` (7), configured in workspace `Cargo.toml` + `clippy.toml`. Coupling: `make coupling` (cargo-coupling per crate) fails on any circular module dependency or grade below C.
- Fix findings by restructuring, never by raising thresholds or adding `#[allow]` (only generated code is exempt).

## i18n rules

- Never put a user-facing string literal in `fx-gui`. Use `t("key")`, `tr("key", &[("name", v)])` or `tn("key", count, &[])` from `fx_app::i18n`.
- Catalogs are `crates/fx-app/locales/{en,de,es,fr,uk,zh-CN}.toml`, embedded at compile time. English is the source and fallback.
- Add every new key to all six catalogs; test `every_catalog_matches_english` fails on missing/unknown keys, placeholder mismatch or missing plural forms (CLDR suffixes `.one/.few/.many/.other`, count passed as `{n}`).
- New language steps are in `docs/translating.md`.

## Misc

- Release: bump workspace `version` in `Cargo.toml`, commit, tag `vX.Y.Z[-pre]`; `release.yml` rejects a non-semver or mismatched tag, runs tests, builds via `build.yml` (smoke-tested, version stamped), and creates a **draft** release with `SHA256SUMS` for you to edit and publish.
- Every push to `master` rebuilds the rolling `latest` pre-release (`latest.yml`). Commits and PR titles follow Conventional Commits; the types pick the next version.
- Icons: Material icon theme first; anything it lacks uses Lucide (ISC), vendored in `third_party/lucide/` with attribution. Never draw glyphs by hand or use text arrows. See `docs/icons.md`.
- More detail in `docs/` (`layout.md`, `settings.md`, `themes.md`, `copy-engine.md`, `performance.md`, `development.md`).
