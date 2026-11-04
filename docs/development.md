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

## Releases

Bump `version` in the workspace `Cargo.toml`, commit, then tag `vX.Y.Z`.
`.github/workflows/release.yml` refuses a tag that does not match the version and publishes Linux (tar.gz, .deb, AppImage), Windows (zip) and macOS (dmg, unsigned) assets.
