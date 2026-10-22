# The Linux host of the window launcher's check: WebKitGTK, the tray's indicator library, and the pinned TypeScript compiler, so a Linux host needs none of their development packages.
# A virtual display, a session bus, and xdotool let the window's test type into a real webview without a desktop.
FROM docker.io/library/node:22-bookworm-slim@sha256:c3de60bf2f9dd0ac6370e6117950ff62d6e339527e7472301c9c78a017978392 AS node

FROM rust:1.99.1-slim-trixie@sha256:d5928c68f8909c141df1bef47ac2c99c025fc9480d09870e08c3783b270eedec

COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
  && apt-get update \
  && apt-get install --yes --no-install-recommends build-essential ca-certificates pkg-config rsync \
    libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev \
    xvfb xdotool dbus-x11 at-spi2-core \
  && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt \
  && cargo install --locked cargo-nextest@0.9.146 \
  && rm -rf /usr/local/cargo/registry

# The lockfile pins the compiler and its native binaries by integrity hash.
COPY package.json package-lock.json /opt/typescript/
RUN cd /opt/typescript && npm ci --no-audit --no-fund
ENV PATH=/opt/typescript/node_modules/.bin:$PATH

WORKDIR /work
