# The host of the relay's Worker test: it builds the Worker with worker-build, serves it with `wrangler dev --local`, and runs the relay's end-to-end test against it.
# Nothing here deploys the Worker.
FROM docker.io/library/node:22-bookworm-slim@sha256:c3de60bf2f9dd0ac6370e6117950ff62d6e339527e7472301c9c78a017978392 AS node

FROM rust:1.99.1-slim-trixie@sha256:d5928c68f8909c141df1bef47ac2c99c025fc9480d09870e08c3783b270eedec

COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
  && apt-get update \
  && apt-get install --yes --no-install-recommends build-essential ca-certificates curl libssl-dev pkg-config rsync \
  && rm -rf /var/lib/apt/lists/*

RUN rustup target add wasm32-unknown-unknown \
  && rustup component add clippy \
  && cargo install --locked worker-build@0.8.7 cargo-nextest@0.9.146 \
  && rm -rf /usr/local/cargo/registry

# The lockfile pins wrangler and every package it loads by integrity hash, workerd among them.
COPY package.json package-lock.json /opt/wrangler/
RUN cd /opt/wrangler && npm ci --no-audit --no-fund
ENV PATH=/opt/wrangler/node_modules/.bin:$PATH

WORKDIR /work
