# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# base: pinned Rust toolchain via rustup plus common build tools.
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS base

ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

ARG RUST_VERSION=1.97.1

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl build-essential pkg-config git \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
        sh -s -- -y --no-modify-path --profile minimal \
        --default-toolchain "${RUST_VERSION}" \
        --component clippy,rustfmt \
    && rustc --version && cargo --version

# sccache is a compiler-output cache keyed on source, flags and the target
# directory path. A `cargo clean` rebuild at the same path hits it fully; a
# build at a different target path misses. Pinned binary release, not
# `cargo install`, which would compile it from source on every image rebuild.
ARG SCCACHE_VERSION=0.17.0
RUN curl -fsSL -o /tmp/sccache.tar.gz \
      "https://github.com/mozilla/sccache/releases/download/v${SCCACHE_VERSION}/sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
    && tar -xzf /tmp/sccache.tar.gz -C /tmp \
    && mv "/tmp/sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl/sccache" /usr/local/cargo/bin/sccache \
    && chmod +x /usr/local/cargo/bin/sccache \
    && rm -rf /tmp/sccache.tar.gz "/tmp/sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl" \
    && sccache --version

# sccache does not cache incremental compilation, and dev builds enable it, so
# turn it off for the cache to hit.
ENV RUSTC_WRAPPER=sccache \
    SCCACHE_DIR=/sccache \
    CARGO_INCREMENTAL=0

WORKDIR /workspace

# ---------------------------------------------------------------------------
# linux-builder: toolchain plus the runtime and dev libraries egui/winit need,
# and an X server so the GUI can be smoke-tested headless (`make e2e`).
# ---------------------------------------------------------------------------
FROM base AS linux-builder

RUN apt-get update && apt-get install -y --no-install-recommends \
        libxkbcommon-dev libxkbcommon-x11-0 \
        libwayland-dev libwayland-client0 \
        libx11-dev libxcursor-dev libxi-dev libxrandr-dev libxcb1-dev \
        libgl1 libgl1-mesa-dri libegl1 \
        xvfb xauth x11-apps xdotool imagemagick \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /usr/local/cargo/registry /sccache && chmod 0777 /usr/local/cargo/registry /sccache

ENV CARGO_TARGET_DIR=/workspace/target

# ---------------------------------------------------------------------------
# linux-artifact: release binaries, exported as dist/linux/.
# ---------------------------------------------------------------------------
FROM linux-builder AS linux-build

COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/sccache \
    cargo build --release -p fx-gui -p fx-cli \
    && mkdir -p /out && cp target/release/fx-gui target/release/fx /out/

FROM scratch AS linux-artifact
COPY --from=linux-build /out/ /linux/

# ---------------------------------------------------------------------------
# windows-builder: mingw-w64 cross toolchain for the windows-gnu target.
# ---------------------------------------------------------------------------
FROM base AS windows-builder

RUN apt-get update && apt-get install -y --no-install-recommends \
        mingw-w64 \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add x86_64-pc-windows-gnu

ENV CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc

COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/sccache \
    cargo build --release --target x86_64-pc-windows-gnu -p fx-gui -p fx-cli \
    && mkdir -p /out && cp target/x86_64-pc-windows-gnu/release/fx-gui.exe target/x86_64-pc-windows-gnu/release/fx.exe /out/

FROM scratch AS windows-artifact
COPY --from=windows-builder /out/ /windows/
