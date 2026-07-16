# ActivSync

**Sync your Garmin activities to Strava — with review and publishing control.**

[![CI](https://github.com/secunit404/activsync/actions/workflows/ci.yml/badge.svg)](https://github.com/secunit404/activsync/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/secunit404/activsync)](https://github.com/secunit404/activsync/releases)
[![License: MIT](https://img.shields.io/github/license/secunit404/activsync)](LICENSE)

ActivSync polls your Garmin Connect account, then syncs activities to Strava on
your terms. Instead of publishing everything automatically, activities can be
**held for review** so you decide what goes public — useful for keeping private
or duplicate activities off your Strava feed.

## Features

- Automatic Garmin → Strava synchronization on a configurable poll interval.
- **Held-for-review** publishing: approve activities before they reach Strava.
- First-run setup wizard for Garmin (incl. MFA) and Strava OAuth.
- Per-activity-type rules and a configurable display timezone.
- Single self-contained container; state persisted to a mounted volume.

## Quick start (Docker)

Run the published image with Docker Compose:

```yaml
# docker-compose.yml
name: activsync
services:
  activsync:
    image: ghcr.io/secunit404/activsync:latest
    ports:
      - "8381:8381"
    volumes:
      - ./data:/config
    restart: unless-stopped
    environment:
      - TZ=Europe/Stockholm
```

```sh
docker compose up -d
```

Then open <http://localhost:8381> and follow the setup wizard. State (database
and Garmin tokens) is persisted under `./data` (mounted at `/config`).

> The container runs as a non-root user (UID 1000). On Linux, ensure `./data`
> is writable by UID 1000 (e.g. `chown -R 1000:1000 data`), or add
> `user: "${UID}:${GID}"` to the service. On Docker Desktop (macOS/Windows)
> this is handled automatically.

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `TZ` | `Europe/Stockholm` | Timezone for logs when no in-app timezone is set |
| `ACTIVSYNC_LOG_LEVEL` | `INFO` | Log verbosity (`DEBUG` for verbose) |
| `ACTIVSYNC_DB_PATH` | `/config/activsync.db` | SQLite database path |
| `ACTIVSYNC_GARMIN_TOKEN_DIR` | `/config/.garminconnect` | Garmin token storage |
| `ACTIVSYNC_DEV_MOCK_DATA` | _(unset)_ | Dev only: seed fake data, no network |
| `FORWARDED_ALLOW_IPS` | `127.0.0.1` | Proxy IPs trusted to set `X-Forwarded-Proto` (see below) |

The in-app **Settings → Preferences → display timezone** overrides `TZ` for log
timestamps (applied live, no restart needed).

### Strava's Authorization Callback Domain

The setup wizard shows the exact value to enter. Two things it can't tell you:

- **Reachable at more than one address?** (say a public domain *and* a LAN IP)
  Only one can be registered, and subdomains of it are fine but an unrelated
  address is not. Always start **Connect Strava** from the address you
  registered — everything else works on either.
- **Behind an HTTPS proxy?** Set `FORWARDED_ALLOW_IPS` to the proxy's address,
  or `*` if the container is only reachable through it, so the callback URL is
  built as `https://`. `*` trusts `X-Forwarded-Proto` from any client, so don't
  use it on a directly reachable container.

## Development

Standard Python workflow — a local virtualenv, no Docker required:

```sh
make install   # create .venv and install with dev extras
make dev       # dev server on http://localhost:8382 (mock data, isolated DB)
make test      # run the test suite
```

`make dev` runs with auto-reload and **mock data**: it seeds an isolated
`data/activsync-dev.db`, requires no password, and never contacts Garmin or
Strava — your real `data/activsync.db` is never touched. To walk the first-run
wizard from a clean slate, use `make dev-fresh`.

In mock mode the wizard is fully faked: any Garmin email/password connects (use
password `mfa` to trigger the MFA modal; any code except `000000` is accepted);
any Strava client ID/secret connects via a looped-back OAuth step.

## Releases

Versioning and changelogs are automated with
[release-please](https://github.com/googleapis/release-please): merging its
Release PR tags the version, updates [`CHANGELOG.md`](CHANGELOG.md), and
publishes a multi-arch image to
[`ghcr.io/secunit404/activsync`](https://github.com/secunit404/activsync/pkgs/container/activsync).

## License

[MIT](LICENSE)
