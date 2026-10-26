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
    cargo build --release --locked --bin chek-server \
    && cp target/release/chek-server /chek

# The admin panel at /admin is the Chek web app's build, from readchek/app at
# CHEK_WEB_REF, pinned so a release always bundles the same panel; raise it to
# ship a newer one. It's plain files, so it builds once on the build
# machine's platform for every image.
FROM --platform=$BUILDPLATFORM oven/bun:1.4.3-slim AS admin
ARG CHEK_WEB_REF=32b7b13e500067f1a10d79a0fdd199beb67e2ef7
ADD https://github.com/readchek/app.git#${CHEK_WEB_REF}:web /web
WORKDIR /web
RUN bun install --frozen-lockfile && bun run build:server

FROM debian:trixie-slim AS runtime
# libvips makes cover thumbnails; its poppler module renders PDF covers.
# libarchive reads RAR and 7-Zip comics. CA certificates let Chek reach
# Komga servers and extension repositories over HTTPS.
RUN apt-get update \
    && apt-get install -y --no-install-recommends libvips42t64 libarchive13t64 ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --home-dir /data chek \
    && mkdir -p /data && chown chek /data
COPY --from=build /chek /usr/local/bin/chek
COPY --from=admin /web/dist-server /usr/share/chek/admin
USER chek
ENV CHEK_BIND=0.0.0.0:8080 \
    CHEK_DATA_DIR=/data \
    CHEK_ADMIN_DIR=/usr/share/chek/admin
VOLUME ["/data"]
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/chek"]

# Opt-in browser runtime. The default image below stays lightweight.
FROM runtime AS browser
USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends chromium xvfb xauth tini fonts-liberation \
    && rm -rf /var/lib/apt/lists/*
COPY --chmod=755 deploy/server/entrypoint.sh /usr/local/bin/chek-browser
USER chek
ENV HOME=/data \
    CHEK_BROWSER_EXECUTABLE=/usr/bin/chromium \
    CHEK_BROWSER_NATIVE=true \
    CHEK_BROWSER_ARGS='["--no-sandbox","--disable-blink-features=AutomationControlled"]'
ENTRYPOINT ["/usr/bin/tini", "-g", "--", "/usr/local/bin/chek-browser"]

FROM runtime AS default
