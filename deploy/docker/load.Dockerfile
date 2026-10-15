# syntax=docker/dockerfile:1.18
# hebra-load: the load generator, to run next to the servers (same network).
#
#   docker build -f deploy/docker/load.Dockerfile -t hebra-load .

FROM rust:1.95.0-slim-trixie AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked -p hebra-load \
 && cp target/release/hebra-load /hebra-load

FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=build /hebra-load /usr/local/bin/hebra-load
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/hebra-load"]
