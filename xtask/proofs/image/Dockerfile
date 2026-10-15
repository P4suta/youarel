# The image the proof gate runs in on a workstation, so a solver that runs away meets the container's memory limit instead of the host's.
# Every tool matches the pin of mise.toml and xtask/src/tool.rs, which the gate checks before it trusts a result.
# Kani's bundle is built for glibc 2.39, so the base is Debian trixie.
FROM rust:1.99.0-slim-trixie@sha256:2752b332db73fdbb7dc576f06c82ed1f312005784ef913d7e04a28f5f55dc581

RUN apt-get update \
  && apt-get install --yes --no-install-recommends build-essential ca-certificates curl git pkg-config rsync \
  && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt \
  && rustup target add thumbv7em-none-eabi \
  && rustup toolchain install nightly-2026-08-21 --profile minimal --component miri --component rust-src --component llvm-tools-preview

RUN cargo install --locked kani-verifier@0.68.0 cargo-fuzz@0.13.2 cargo-mutants@27.1.0 cargo-nextest@0.9.146 \
  && cargo kani setup \
  && rm -rf /usr/local/cargo/registry

WORKDIR /work
