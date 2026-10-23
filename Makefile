# SPDX-License-Identifier: GPL-3.0-or-later
#
# Every build goes through here, so they all use the Rust version pinned in rust-toolchain.toml:
#   - Linux builds, formatting, lints and tests run in Docker (compose.yaml).
#   - macOS builds run natively (Apple's SDK only exists on macOS), through rustup with the same
#     pinned version, never through whatever `cargo` is first on PATH (e.g. Homebrew's).
#
# `make help` lists the targets.

RUST_VERSION := $(shell sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
export RUST_VERSION
export HOST_UID := $(shell id -u)
export HOST_GID := $(shell id -g)

COMPOSE ?= docker compose
IN_DOCKER = $(COMPOSE) run --rm rust
RUSTUP ?= rustup
# The pinned toolchain's own binaries, first on PATH. `rustup run` alone is not enough: some rustup
# installs (Homebrew's, for one) leave PATH alone, so cargo would pick up another `rustc`.
TOOLCHAIN_BIN = $(shell $(RUSTUP) run $(RUST_VERSION) rustc --print sysroot)/bin
NATIVE_CARGO = PATH="$(TOOLCHAIN_BIN):$$PATH" RUSTC="$(TOOLCHAIN_BIN)/rustc" "$(TOOLCHAIN_BIN)/cargo"

.DEFAULT_GOAL := help
.PHONY: help image shell fmt lint test check toolchain which-rust bindings macos-app macos-release macos-test bench clean

help: ## Show this help
	@echo "Rust $(RUST_VERSION) (from rust-toolchain.toml)"
	@echo
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "}; {printf "  %-15s %s\n", $$1, $$2}'

# --- Linux, in Docker -----------------------------------------------------------------------

image: ## Build the Linux build image
	$(COMPOSE) build rust

shell: image ## Open a shell in the Linux build environment
	$(IN_DOCKER) bash

fmt: image ## Format all Rust code
	$(IN_DOCKER) cargo fmt --all

lint: image ## Check formatting and run clippy (Linux)
	$(IN_DOCKER) cargo fmt --all --check
	$(IN_DOCKER) cargo clippy --workspace --all-targets --locked -- -D warnings

test: image ## Run the Rust test suite (Linux)
	$(IN_DOCKER) cargo test --workspace --locked

check: lint test ## lint + test

# --- macOS, native --------------------------------------------------------------------------

toolchain: ## Install the pinned Rust toolchain with rustup (macOS builds)
	@command -v $(RUSTUP) >/dev/null || { echo "rustup is required: https://rustup.rs"; exit 1; }
	$(RUSTUP) toolchain install $(RUST_VERSION) --profile minimal --component rustfmt,clippy

which-rust: toolchain ## Show the Rust toolchain macOS builds use
	@$(NATIVE_CARGO) --version
	@PATH="$(TOOLCHAIN_BIN):$$PATH" rustc --version --verbose | grep -E '^(rustc|host)'
	@echo "from $(TOOLCHAIN_BIN)"

bindings: toolchain ## Regenerate the Swift bindings
	$(NATIVE_CARGO) xtask swift-bindings

macos-app: toolchain ## Build target/macos/Branchosaurus.app (debug)
	$(NATIVE_CARGO) xtask macos-app

macos-release: toolchain ## Build a universal release app
	$(RUSTUP) target add --toolchain $(RUST_VERSION) aarch64-apple-darwin x86_64-apple-darwin
	$(NATIVE_CARGO) xtask macos-app --release --universal

macos-test: toolchain ## Run the Swift tests
	$(NATIVE_CARGO) xtask macos-test

bench: toolchain ## Time history loading: make bench REPO=path/to/repo
	@test -n "$(REPO)" || { echo "usage: make bench REPO=path/to/repo"; exit 1; }
	$(NATIVE_CARGO) run --release -p branchosaurus-core --example log -- "$(REPO)" 40 >/dev/null

clean: ## Remove build output and the Docker cache volume
	$(NATIVE_CARGO) clean
	rm -rf apps/macos/.build apps/macos/Sources/BranchosaurusFFI/include apps/macos/Sources/BranchosaurusCore/Generated
	$(COMPOSE) down --volumes --remove-orphans
