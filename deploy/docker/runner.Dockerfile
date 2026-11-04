# syntax=docker/dockerfile:1.18
# A Hebra runner as a container: what `hb heddle up --provider docker`
# starts to stand in for a VPS, and a ready-made runner for any host.
#
#   docker build -f deploy/docker/runner.Dockerfile -t hebra-runner .
#   docker run -e HEBRA_SERVER=https://hebra.example.com -e HB_RUNNER_TOKEN=hbr_… hebra-runner
#
# HEBRA_BOOT_DELAY (seconds) waits before registering, to behave like a
# virtual machine that takes that long to boot.

FROM rust:1.95.0-slim-trixie AS build
WORKDIR /src
ENV CARGO_TERM_COLOR=never \
    CARGO_PROFILE_RELEASE_LTO=thin \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
RUN apt-get update && apt-get install -y --no-install-recommends g++ && rm -rf /var/lib/apt/lists/*
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked -p hebra-cli \
 && cp target/release/hb /hb

FROM debian:trixie-slim
LABEL org.opencontainers.image.source="https://github.com/s-pl/hebra" \
      org.opencontainers.image.description="Hebra runner: runs checks for a Hebra server" \
      org.opencontainers.image.licenses="Apache-2.0"
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates git curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 1000 runner
COPY --from=build /hb /usr/local/bin/hb
COPY --chmod=755 deploy/docker/runner-entrypoint.sh /usr/local/bin/hebra-runner
USER runner
WORKDIR /home/runner
ENV HB_CONFIG_DIR=/home/runner/.hebra
ENTRYPOINT ["/usr/local/bin/hebra-runner"]
