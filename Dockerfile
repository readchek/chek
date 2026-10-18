# syntax=docker/dockerfile:1
# Builds the server for the platform Docker builds on; CI builds linux/amd64
# and linux/arm64 with buildx.

FROM rust:1.97-slim-trixie AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends libvips-dev libarchive-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bin chek \
    && cp target/release/chek /chek

FROM debian:trixie-slim
# libvips makes cover thumbnails; its poppler module renders PDF covers.
# libarchive reads RAR and 7-Zip comics.
RUN apt-get update \
    && apt-get install -y --no-install-recommends libvips42t64 libarchive13t64 \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --home-dir /data chek \
    && mkdir -p /data && chown chek /data
COPY --from=build /chek /usr/local/bin/chek
USER chek
ENV CHEK_BIND=0.0.0.0:8080 \
    CHEK_DATA_DIR=/data
VOLUME ["/data"]
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/chek"]
