# syntax=docker/dockerfile:1.7

# The Rust server's image. Build it from the repository root, which holds the
# React app and the version the server reports:
#   docker build -f rust/Dockerfile -t activsync-rust .

# The SPA is static files, the same for every image platform, so it is built
# once on the runner's own platform.
FROM --platform=$BUILDPLATFORM node:26-slim AS web
WORKDIR /web
COPY package.json package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci
COPY app ./app
COPY react-router.config.ts vite.config.ts tsconfig.json ./
RUN npm run build

FROM rust:1-slim-trixie AS server
WORKDIR /src
COPY rust ./rust
# version.rs reads the release number release-please writes here, and the
# server builds in the icons the Python app serves.
COPY src/activsync/__init__.py ./src/activsync/__init__.py
COPY src/activsync/static ./src/activsync/static
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/rust/target \
    cargo build --release --locked --manifest-path rust/Cargo.toml \
    && cp rust/target/release/activsync /usr/local/bin/activsync

FROM debian:trixie-slim

ARG VERSION=dev
ARG REVISION=unknown
LABEL org.opencontainers.image.title="ActivSync" \
      org.opencontainers.image.description="Sync Garmin activities to Strava with review and publishing control" \
      org.opencontainers.image.source="https://github.com/secunit404/activsync" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"

# The footer shows this: a release version, or a dev image's branch tag.
ENV ACTIVSYNC_BUILD="${VERSION}" \
    ACTIVSYNC_HOST=0.0.0.0 \
    PORT=8381 \
    ACTIVSYNC_WEB_DIR=/app/web \
    ACTIVSYNC_DB_PATH=/config/activsync.db \
    ACTIVSYNC_GARMIN_TOKEN_DIR=/config/.garminconnect

COPY --from=server /usr/local/bin/activsync /usr/local/bin/activsync
COPY --from=web /web/build/client /app/web

# Same UID as the Python image, so an existing ./data bind mount keeps working.
RUN groupadd --gid 1000 app \
    && useradd --uid 1000 --gid 1000 --no-create-home app \
    && mkdir -p /config \
    && chown -R app:app /config
USER app

VOLUME ["/config"]
EXPOSE 8381

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["activsync", "health-check"]

CMD ["activsync"]
