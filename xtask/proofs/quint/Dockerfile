# The offline host of the Quint obligations, which `just proofs` runs with `--network none`.
FROM docker.io/library/eclipse-temurin:21-jre@sha256:cff19e6215689161eb6162c11b86b0c60ddf802164f2eaf48d570f8fb79a36c5 AS java

FROM docker.io/library/node:22-bookworm-slim@sha256:c3de60bf2f9dd0ac6370e6117950ff62d6e339527e7472301c9c78a017978392
COPY --from=java /opt/java/openjdk /opt/java/openjdk
ENV PATH=/opt/java/openjdk/bin:/opt/quint/node_modules/.bin:$PATH
ENV QUINT_HOME=/opt/quint

# The lockfile pins Quint and every package it loads by integrity hash.
COPY package.json package-lock.json /opt/quint/
RUN cd /opt/quint && npm ci --ignore-scripts --no-audit --no-fund

# Quint compiles a model through Apalache and runs the model checker that Apalache bundles, with the modules the compiled model extends.
# The release lists the checksum in `sha256sum.txt`, and this file repeats it, so a replaced asset fails the build.
ARG APALACHE_VERSION=0.62.1
ARG APALACHE_SHA256=93fef2f381d30465eff906a2d6afa3358cb2c9adf6a8aad3bec0ded4f45432bb
ADD --checksum=sha256:${APALACHE_SHA256} https://github.com/apalache-mc/apalache/releases/download/v${APALACHE_VERSION}/apalache-${APALACHE_VERSION}.tgz /tmp/apalache.tgz
ADD https://github.com/apalache-mc/apalache/releases/download/v${APALACHE_VERSION}/sha256sum.txt /tmp/sha256sum.txt
RUN grep -qx "${APALACHE_SHA256}  apalache-${APALACHE_VERSION}.tgz" /tmp/sha256sum.txt \
    && echo "${APALACHE_SHA256}  /tmp/apalache.tgz" | sha256sum -c - \
    && mkdir -p ${QUINT_HOME}/apalache-dist-${APALACHE_VERSION}/apalache \
    && tar -xzf /tmp/apalache.tgz --strip-components=1 -C ${QUINT_HOME}/apalache-dist-${APALACHE_VERSION}/apalache \
    && rm /tmp/apalache.tgz /tmp/sha256sum.txt \
    && test -f ${QUINT_HOME}/apalache-dist-${APALACHE_VERSION}/apalache/lib/apalache.jar
