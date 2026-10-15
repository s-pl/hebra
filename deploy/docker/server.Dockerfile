# syntax=docker/dockerfile:1.18
# hebra-server: a static-ish Rust binary on distroless (no shell, no package
# manager), running as an unprivileged user with a read-only filesystem.
#
#   docker build -f deploy/docker/server.Dockerfile -t hebra-server .

FROM rust:1.95.0-slim-trixie AS build
WORKDIR /src
ENV CARGO_TERM_COLOR=never
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked -p hebra-server \
 && cp target/release/hebra-server /hebra-server

FROM gcr.io/distroless/cc-debian13:nonroot
LABEL org.opencontainers.image.source="https://github.com/s-pl/hebra" \
      org.opencontainers.image.description="Hebra server: version control for people and many agents" \
      org.opencontainers.image.licenses="Apache-2.0"
COPY --from=build /hebra-server /usr/local/bin/hebra-server
USER 65532:65532
ENV HEBRA_ADDR=0.0.0.0:4600 \
    HEBRA_DATA=/data \
    HEBRA_LOG_JSON=true
EXPOSE 4600 9090
VOLUME ["/data"]
ENTRYPOINT ["/usr/local/bin/hebra-server"]
CMD ["serve"]
