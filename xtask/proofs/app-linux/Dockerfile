# The Linux host of the window launcher's check: WebKitGTK, the tray's indicator library, and the pinned TypeScript compiler, so a Linux host needs none of their development packages.
FROM docker.io/library/node:22-bookworm-slim@sha256:c3de60bf2f9dd0ac6370e6117950ff62d6e339527e7472301c9c78a017978392 AS node

FROM rust:1.99.0-slim-trixie@sha256:2752b332db73fdbb7dc576f06c82ed1f312005784ef913d7e04a28f5f55dc581

COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
  && apt-get update \
  && apt-get install --yes --no-install-recommends build-essential ca-certificates pkg-config rsync \
    libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev \
  && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt \
  && cargo install --locked cargo-nextest@0.9.146 \
  && rm -rf /usr/local/cargo/registry

# The lockfile pins the compiler and its native binaries by integrity hash.
COPY package.json package-lock.json /opt/typescript/
RUN cd /opt/typescript && npm ci --no-audit --no-fund
ENV PATH=/opt/typescript/node_modules/.bin:$PATH

WORKDIR /work
