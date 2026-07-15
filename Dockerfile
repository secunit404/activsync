# syntax=docker/dockerfile:1.7
FROM python:3.14-slim

ARG VERSION=dev
ARG REVISION=unknown
LABEL org.opencontainers.image.title="ActivSync" \
      org.opencontainers.image.description="Sync Garmin activities to Strava with review and publishing control" \
      org.opencontainers.image.source="https://github.com/secunit404/activsync" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"

WORKDIR /app

COPY pyproject.toml ./
COPY src ./src

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
