# SPDX-License-Identifier: GPL-3.0-or-later
#
# Linux build environment with the Rust version from rust-toolchain.toml. Use it through the
# Makefile (`make test`, `make lint`, `make shell`), which passes RUST_VERSION.

ARG RUST_VERSION
FROM rust:${RUST_VERSION}-slim-trixie

# git: the core runs it, and its tests compare against it.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && rustup component add rustfmt clippy

# Caches live in a named volume mounted here. Docker copies this directory's permissions into the
# volume when it is first created, so the container can run as the host user.
RUN mkdir -p /cache && chmod 1777 /cache
ENV CARGO_HOME=/cache/cargo \
    CARGO_TARGET_DIR=/cache/target \
    HOME=/cache/home \
    RUSTUP_AUTO_INSTALL=0

WORKDIR /workspace
