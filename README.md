# ActivSync

Activity synchronization and publishing control.

## Development

Standard Python workflow — a local virtualenv, no Docker required.

```sh
make install   # create .venv and install the app with dev extras
make dev       # run the dev server on http://localhost:8382
make test      # run the test suite
```

`make dev` runs with auto-reload and **mock data**: it seeds an isolated
`data/activsync-dev.db` (36 sample activities plus 16 Garmin categories),
requires no password, and never contacts Garmin or Strava. Your real
`data/activsync.db` is never touched. Changes under `src/` reload automatically.

To reset the sample data, stop the server, delete `data/activsync-dev.db`, and
start it again.

### Testing the first-run setup wizard

To walk the whole onboarding flow (Garmin login → MFA → Strava → initial sync)
from a clean slate:

```sh
make dev-fresh   # wipes data/activsync-dev.db, then starts the dev server
```

In mock mode the wizard is fully faked — no real accounts or network calls:

- **Garmin:** any email/password connects. Use the password `mfa` to trigger the
  MFA modal; then any code works except `000000`, which is rejected so you can
  test the error state.
- **Strava:** enter any client ID/secret; the OAuth step loops straight back and
  connects without leaving the app.
- **Initial sync:** completes against the seeded sample data and lands you on the
  dashboard.

Prefer not to use `make`? The equivalent commands are:

```sh
python3 -m venv .venv
.venv/bin/pip install -e ".[dev]"
ACTIVSYNC_DEV_MOCK_DATA=1 ACTIVSYNC_DB_PATH=data/activsync-dev.db \
  .venv/bin/uvicorn activsync.main:app --reload --reload-dir src --port 8382
```

### Logs

The app logs to stdout (captured by `docker logs` in production). Lines are
timestamped in the timezone you pick under **Settings → Preferences**
(`display_timezone`), updated live — no restart needed. Set
`ACTIVSYNC_LOG_LEVEL=DEBUG` for verbose output; the default is `INFO`. If no
in-app timezone is set, the `TZ` environment variable is used, falling back to
`Europe/Stockholm`.

## Running in production (Docker)

For deployment, run the app in Docker with the real database:

```sh
docker compose up -d
```

This serves the real-data ActivSync app on <http://localhost:8381>, persisting
state to `./data` (mounted at `/config` in the container). It starts the
Garmin/Strava poller and uses `data/activsync.db`.
