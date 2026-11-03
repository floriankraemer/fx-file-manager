CARGO ?= cargo
ARGS  ?=

.PHONY: all build release gui cli test lint fmt check clean

all: build

## Debug build of the whole workspace
build:
	$(CARGO) build --workspace

## Optimized build (lto, panic=abort) of the whole workspace
release:
	$(CARGO) build --workspace --release

## Build and start the egui explorer: make gui ARGS=/usr
gui:
	$(CARGO) run --release -p fx-gui -- $(ARGS)

## Build and run the console lister: make cli ARGS="walk /usr --quiet"
cli:
	$(CARGO) run --release -p fx-cli -- $(ARGS)

test:
	$(CARGO) test --workspace

lint:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt:
	$(CARGO) fmt --all

## Everything CI checks
check:
	$(CARGO) fmt --all --check
	$(MAKE) lint test

clean:
	$(CARGO) clean
