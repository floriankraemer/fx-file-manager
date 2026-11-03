CARGO  ?= cargo
ARGS   ?=
DOCKER ?= docker
DOCKERFILE := docker/Dockerfile
LINUX_IMAGE := fx-linux-builder

# Named volumes so the crate registry and the sccache store survive `--rm`.
# The image chmods both mountpoints 0777, so any uid can write them.
DOCKER_MOUNTS = -v "$(CURDIR)":/workspace -w /workspace \
	-v fx-cargo-registry:/usr/local/cargo/registry \
	-v fx-sccache:/sccache

# Run as the developer who owns the checkout, not root: `/workspace` is a bind
# mount, and a root container would leave `target/` owned by root on the host.
# HOME is redirected because the image's /root is not writable by this uid.
DOCKER_USER = --user $(shell id -u):$(shell id -g) -e HOME=/tmp
RUN_LINUX = $(DOCKER) run --rm --init $(DOCKER_USER) $(DOCKER_MOUNTS) $(LINUX_IMAGE)

.PHONY: help all debug build build-linux build-windows build-osx release gui cli test lint fmt check \
	linux-image test-container e2e clean

.DEFAULT_GOAL := help

##@ General

## Show this help
help:
	@awk 'BEGIN { c_head = "\033[1;33m"; c_tgt = "\033[36m"; c_off = "\033[0m" } \
		/^##@/ { printf "\n%s%s%s\n", c_head, substr($$0, 5), c_off; next } \
		/^## / { desc = substr($$0, 4); next } \
		/^[a-zA-Z_-]+:/ { if (desc != "") printf "  %s%-18s%s %s\n", c_tgt, substr($$0, 1, index($$0, ":") - 1), c_off, desc; desc = "" }' \
		$(MAKEFILE_LIST)
	@echo

all: build

##@ Development

## Debug build of the whole workspace
debug:
	$(CARGO) build --workspace

## Optimized build of the whole workspace
release:
	$(CARGO) build --workspace --release

## Build and start the egui explorer: make gui ARGS=/usr
gui:
	$(CARGO) run --release -p fx-gui -- $(ARGS)

## Build and run the console lister: make cli ARGS="walk /usr --quiet"
cli:
	$(CARGO) run --release -p fx-cli -- $(ARGS)

##@ Quality

## Run all tests on the host
test:
	$(CARGO) test --workspace

## Clippy with warnings as errors
lint:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

## Format all crates
fmt:
	$(CARGO) fmt --all

## Everything CI checks (host)
check:
	$(CARGO) fmt --all --check
	$(MAKE) lint test

## fmt, clippy and tests inside the Linux image
test-container: linux-image
	$(RUN_LINUX) sh -c 'cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && xvfb-run -a cargo test --workspace'

##@ Containers

## Build the Linux builder image (toolchain, GUI libs, Xvfb)
linux-image:
	$(DOCKER) build --target linux-builder -t $(LINUX_IMAGE) -f $(DOCKERFILE) .

## Launch the release GUI under Xvfb; screenshot to dist/e2e/
e2e: linux-image
	$(RUN_LINUX) sh -c 'cargo build --release -p fx-gui && scripts/e2e-smoke.sh target/release/fx-gui /usr dist/e2e/screenshot.png'

##@ Release builds

## Linux and Windows artifacts (macOS needs a Mac: see build-osx)
build: build-linux build-windows

## Export dist/linux/ (fx-gui, fx) built in Docker
build-linux:
	$(DOCKER) buildx build --target linux-artifact -f $(DOCKERFILE) --output type=local,dest=dist/ .

## Export dist/windows/ (fx-gui.exe, fx.exe) cross-built with mingw-w64
build-windows:
	$(DOCKER) buildx build --target windows-artifact -f $(DOCKERFILE) --output type=local,dest=dist/ .

## Native macOS build into dist/macos/ (Mac host only, no container)
build-osx:
	@if [ "$$(uname -s)" != Darwin ]; then echo "build-osx needs a macOS host (Apple SDK cannot run in a Linux container)" >&2; exit 1; fi
	$(CARGO) build --release --workspace
	mkdir -p dist/macos
	cp target/release/fx-gui target/release/fx dist/macos/

##@ Maintenance

## cargo clean and remove dist/
clean:
	$(CARGO) clean
	rm -rf dist
