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
| `ACTIVSYNC_MANUAL_ONLY` | _(unset)_ | Disable Garmin/Strava polling; keep Hevy polling with reviewed matches |
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

Local development uses the React dev server on port 8382 and the mock-only
FastAPI backend on port 8383. A local virtualenv and Node.js 22+ are required;
Docker is not:

```sh
make install   # install Python and frontend dependencies
make dev       # React UI on http://localhost:8382 (mock data, isolated DB)
make test      # run the Python and frontend test suites
```

`make dev` runs with auto-reload and **mock data**: it seeds an isolated
`data/activsync-dev.db`, requires no password, and never contacts Garmin or
Strava — your real `data/activsync.db` is never touched. To walk the first-run
wizard from a clean slate, use `make dev-fresh`.

In mock mode the wizard is fully faked: any Garmin email/password connects (use
password `mfa` to trigger the MFA modal; any code except `000000` is accepted);
any Strava client ID/secret connects via a looped-back OAuth step.

### Testing a branch with real accounts

Stop the normal app on port 8381, then run:

```sh
make dev-real
```

This uses the real `data/activsync.db` and Garmin token directory, serves the
hot-reloading UI at <http://localhost:8382>, and disables the Garmin, Strava,
and update-check background jobs. The Hevy polling leg remains active so the
queue receives new workouts. When one Hevy workout matches one Garmin workout,
development modes default to pausing before any change and offering **Merge**,
**Replace**, **Description only**, or **Skip** on the Hevy page. Settings can
switch match handling to **Automatic** and choose its default strategy. A
historical backfill that finds one overlapping Garmin activity now enters this
same review state immediately instead of being labelled as a new activity and
waiting for another Hevy poll.

The Hevy description template is editable in Settings with placeholders for
the workout title, duration, calories, average heart rate, exercise summary,
and ActivSync marker. A live sample preview renders beside the editor. The
shared Garmin/Strava format is plain text: line breaks, Unicode, emoji, and
bullets are suitable, while Markdown and HTML markup remains literal text.
**Description only** always writes the rendered summary;
for **Merge** and **Replace**, writing it can be switched off while still
applying the structured exercise sets. The queue's workout detail view shows
the exact rendered preview before a match decision.

An unmatched Hevy workout can still upload as a separate Garmin activity after
the configured grace period. Matched workouts remain publish-blocked while
waiting for your decision. The normal Strava publishing/status poll stays off,
although the Hevy leg may refresh metadata on an already-linked Strava copy.
The command refuses to start if the real database is missing or ActivSync is
still responding on port 8381. To use the legacy database filename explicitly,
run `make dev-real REAL_DB_PATH=data/garmin2strava.db`.

This mode contacts real services and can change real account data. Keep mock
`make dev` as the default for ordinary development and automated agent checks.

## Releases

Versioning and changelogs are automated with
[release-please](https://github.com/googleapis/release-please): merging its
Release PR tags the version, updates [`CHANGELOG.md`](CHANGELOG.md), and
publishes a multi-arch image to
[`ghcr.io/secunit404/activsync`](https://github.com/secunit404/activsync/pkgs/container/activsync).

## License

[MIT](LICENSE)
