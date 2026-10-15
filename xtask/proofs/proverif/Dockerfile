# The offline host of the ProVerif obligations, which `just proofs` runs with `--network none`.
FROM docker.io/ocaml/opam:debian-12-ocaml-5.3@sha256:bbaac53e502f6602013d8967c3a54cfcb898b556f453ab72e8e23966c3c681df AS build

# The project publishes no checksum, so this file pins the one of the release it reviewed, and a replaced asset fails the build.
ARG PROVERIF_VERSION=2.05
ARG PROVERIF_SHA256=4871f53c32ab4a04669a060c4886ba5d9080496963fb980a9a62d2c429ceabc4
# BuildKit's own download breaks off on this server, so curl fetches the release over HTTP/1.1.
RUN curl --fail --location --proto '=https' --http1.1 --retry 3 --output /home/opam/proverif.tar.gz https://bblanche.gitlabpages.inria.fr/proverif/proverif${PROVERIF_VERSION}.tar.gz \
    && echo "${PROVERIF_SHA256}  /home/opam/proverif.tar.gz" | sha256sum -c - \
    && tar -xzf /home/opam/proverif.tar.gz -C /home/opam \
    && cd /home/opam/proverif${PROVERIF_VERSION} \
    && opam exec -- ./build -nointeract \
    && test -x proverif

FROM docker.io/library/debian:12-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
ARG PROVERIF_VERSION=2.05
COPY --from=build /home/opam/proverif${PROVERIF_VERSION}/proverif /usr/local/bin/proverif
RUN proverif -help >/dev/null 2>&1 || proverif 2>&1 | grep -q ProVerif
