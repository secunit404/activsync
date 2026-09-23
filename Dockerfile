# syntax=docker/dockerfile:1.7

# The SPA is built here rather than copied in: build/ is git-ignored and
# .dockerignore'd, so an image that did not build it would serve 404 on every
# page (server.spa() needs web/index.html to exist).
FROM node:25-slim AS web
WORKDIR /web
COPY package.json package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci
COPY app ./app
COPY react-router.config.ts vite.config.ts tsconfig.json ./
RUN npm run build

FROM python:3.14-slim

ARG VERSION=dev
ARG REVISION=unknown
LABEL org.opencontainers.image.title="ActivSync" \
      org.opencontainers.image.description="Sync Garmin activities to Strava with review and publishing control" \
      org.opencontainers.image.source="https://github.com/secunit404/activsync" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"

# The app reports this in the footer: a release version, or a dev image's branch tag.
ENV ACTIVSYNC_BUILD="${VERSION}"

WORKDIR /app

COPY pyproject.toml ./
COPY src ./src
# Must land before pip install: setuptools package-data ships web/ into the
# installed package.
COPY --from=web /web/build/client ./src/activsync/web

RUN --mount=type=cache,target=/root/.cache/pip \
    pip install .

# Non-root runtime user that owns the /config volume.
# NOTE: with a bind mount (./data:/config) the host directory's ownership wins,
# so on Linux the host data dir must be writable by UID 1000, or override with
# `user:` in compose. Documented in README.
RUN groupadd --gid 1000 app \
    && useradd --uid 1000 --gid 1000 --no-create-home app \
    && mkdir -p /config \
    && chown -R app:app /config
USER app

VOLUME ["/config"]
EXPOSE 8381

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD python -c "import urllib.request,sys; sys.exit(0 if urllib.request.urlopen('http://localhost:8381/health', timeout=3).status == 200 else 1)"

CMD ["uvicorn", "activsync.main:app", "--host", "0.0.0.0", "--port", "8381"]
