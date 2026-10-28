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

# The web app at / and its admin panel at /admin are the build of
# readchek/app's web/ at CHEK_WEB_REF, pinned so a release always bundles the
# same app; raise it to ship a newer one. It's plain files, so it builds once
# on the build machine's platform for every image.
FROM --platform=$BUILDPLATFORM oven/bun:1.4.3-slim AS web
ARG CHEK_WEB_REF=872d892c4641ce62f890c5ed536dd258743f9249
ADD https://github.com/readchek/app.git#${CHEK_WEB_REF}:web /web
WORKDIR /web
RUN bun install --frozen-lockfile && bun run build:server
# A brotli and a gzip copy beside each file the browser can take compressed,
# which the server picks by Accept-Encoding. Nothing in front of a home server
# compresses for it, and the app's script is about 480 KB before compression.
RUN bun -e 'import { Glob } from "bun"; \
    import { brotliCompressSync, gzipSync } from "node:zlib"; \
    for (const path of new Glob("**/*.{html,js,mjs,css,svg,ico,webmanifest,bcmap,pfb,ttf}").scanSync("dist-server")) { \
        const file = `dist-server/${path}`; \
        const data = await Bun.file(file).bytes(); \
        const br = brotliCompressSync(data); \
        const gz = gzipSync(data, { level: 9 }); \
        if (br.length < data.length) await Bun.write(`${file}.br`, br); \
        if (gz.length < data.length) await Bun.write(`${file}.gz`, gz); \
    }'

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
COPY --from=web /web/dist-server /usr/share/chek/web
USER chek
ENV CHEK_BIND=0.0.0.0:8080 \
    CHEK_DATA_DIR=/data \
    CHEK_WEB_DIR=/usr/share/chek/web
VOLUME ["/data"]
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/chek"]

# Opt-in browser runtime. The default image below stays lightweight.
FROM runtime AS browser
USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends chromium xvfb xauth tini fonts-liberation curl \
    && rm -rf /var/lib/apt/lists/*
COPY --chmod=755 deploy/server/entrypoint.sh /usr/local/bin/chek-browser
USER chek
# CloakBrowser's stealth Chromium is downloaded into the data volume on first
# start, because its license doesn't allow redistributing the binary in an
# image. Stock Chromium stays as the fallback when the download fails.
ENV HOME=/data \
    CHEK_BROWSER_EXECUTABLE=/usr/bin/chromium \
    CHEK_BROWSER_NATIVE=true \
    CHEK_BROWSER_ARGS='["--no-sandbox","--disable-blink-features=AutomationControlled","--fingerprint-platform=windows"]'
ENTRYPOINT ["/usr/bin/tini", "-g", "--", "/usr/local/bin/chek-browser"]

FROM runtime AS default
