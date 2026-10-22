# The Linux host of the window launcher's check: WebKitGTK, the tray's indicator library, and the pinned TypeScript compiler, so a Linux host needs none of their development packages.
# A virtual display, a session bus, and xdotool let the window's test type into a real webview without a desktop.
FROM scratch AS nextest-amd64
ADD --checksum=sha256:682c21b777c333e96fd532e114d3a5a894e0729ab88d94c0a9f20f8419695428 https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-0.9.146/cargo-nextest-0.9.146-x86_64-unknown-linux-gnu.tar.gz /nextest.tar.gz

FROM scratch AS nextest-arm64
ADD --checksum=sha256:b2e33d7c72de7ade0ff7b3a948ac37516b24f8a836b7a8870c1f634a94be9de9 https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-0.9.146/cargo-nextest-0.9.146-aarch64-unknown-linux-gnu.tar.gz /nextest.tar.gz

FROM nextest-${TARGETARCH} AS nextest

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

COPY --from=nextest /nextest.tar.gz /tmp/nextest.tar.gz
RUN rustup component add clippy rustfmt \
  && tar --extract --gzip --file /tmp/nextest.tar.gz --directory /usr/local/cargo/bin cargo-nextest \
  && rm /tmp/nextest.tar.gz \
  && cargo nextest --version

ENV CARGO_PROFILE_DEV_DEBUG=line-tables-only \
    CARGO_PROFILE_TEST_DEBUG=line-tables-only

# The lockfile pins the compiler and its native binaries by integrity hash.
COPY package.json package-lock.json /opt/typescript/
RUN cd /opt/typescript && npm ci --no-audit --no-fund
ENV PATH=/opt/typescript/node_modules/.bin:$PATH

WORKDIR /work
