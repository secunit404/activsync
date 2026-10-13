# ActivSync

**Sync your Garmin activities to Strava, and your Hevy workouts to Garmin — with review and publishing control.**

[![CI](https://github.com/secunit404/activsync/actions/workflows/ci.yml/badge.svg)](https://github.com/secunit404/activsync/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/secunit404/activsync)](https://github.com/secunit404/activsync/releases)
[![License: MIT](https://img.shields.io/github/license/secunit404/activsync)](LICENSE)

ActivSync polls your Garmin Connect account, then syncs activities to Strava on
your terms. Instead of publishing everything automatically, activities can be
**held for review** so you decide what goes public — useful for keeping private
or duplicate activities off your Strava feed.

It also syncs strength training from [Hevy](https://www.hevyapp.com/): a logged
Hevy workout is matched to the activity your watch recorded, and its exercises,
sets and reps are written onto that Garmin activity as structured strength
data.

## Features

- Automatic Garmin → Strava synchronization on a configurable poll interval.
- **Held-for-review** publishing: approve activities before they reach Strava.
- First-run setup wizard for Garmin (incl. MFA) and Strava OAuth.
- Per-activity-type rules and a configurable display timezone.
- **Hevy → Garmin** strength sync: structured exercise sets, with matched
  workouts held for your decision before anything is changed.
- Hevy workouts reach Strava with Strava's own exercises, so sets, reps,
  weights and the muscle map show correctly.
- Backfill of historical Hevy workouts, previewed before any import.
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

Then open <http://localhost:8381> and follow the setup wizard. Its first step
claims the install: it asks for a one-time setup code that only the server log
shows, and for the password you will sign in with. Read the code with:

```sh
docker compose logs activsync | grep "setup code"
```

The code looks like `XXXX-XXXX-XXXX-XXXX`. A wrong guess never changes it;
only a restart issues a new one, so if the log shows several, use the last.

State (database and Garmin tokens) is persisted under `./data` (mounted at
`/config`).

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
| `FORWARDED_ALLOW_IPS` | `127.0.0.1` | Proxy IPs trusted to set `X-Forwarded-Proto`/`-For` (see below) |
| `ACTIVSYNC_PUBLIC_URL` | _(unset)_ | Public origin, e.g. `https://activsync.example.com`. Builds OAuth redirect URIs, is the expected `Origin`, and rejects other `Host` headers |
| `ACTIVSYNC_ALLOWED_HOSTS` | _(unset)_ | Without a public URL, extra host names to accept besides IP addresses and `localhost`, comma-separated, e.g. `activsync.lan`. A leading dot (`.example.com`) also accepts subdomains |
| `ACTIVSYNC_AUTH_PROXY_HEADER` | _(unset)_ | Header an authenticating proxy signs in with, e.g. `Remote-User` (see [Signing in](#signing-in)) |
| `ACTIVSYNC_AUTH_PROXY_TRUSTED` | _(unset)_ | CIDRs of the proxy allowed to send that header, e.g. `172.18.0.0/16`. Required with the header |
| `ACTIVSYNC_AUTH_PROXY_USER` | _(unset)_ | The exact header value that is you. Required with the header |
| `ACTIVSYNC_AUTH_DISABLED` | _(unset)_ | `1` turns sign-in off entirely. LAN-only installs that accept the risk |

The in-app **Settings → Preferences → display timezone** overrides `TZ` for log
timestamps (applied live, no restart needed).

### Strava's Authorization Callback Domain

The setup wizard shows the exact value to enter. Two things it can't tell you:

- **Reachable at more than one address?** (say a public domain *and* a LAN IP)
  Only one can be registered, and subdomains of it are fine but an unrelated
  address is not. Always press **Connect with Strava** from the address you
  registered — everything else works on either.
- **Behind an HTTPS proxy?** Set `FORWARDED_ALLOW_IPS` to the proxy's address
  (or its Docker network, e.g. `172.18.0.0/16`) so the callback URL is built as
  `https://`. Avoid `*`, even when the container is only reachable through the
  proxy: it makes uvicorn take the client-supplied end of `X-Forwarded-For`, so
  clients can pick their own sign-in rate-limit address. See
  [Exposing ActivSync publicly](#exposing-activsync-publicly).

## Signing in

ActivSync has one user: you. The password you chose when claiming the install
always works, and the other methods are added on top of it in
**Settings → Access**. Sessions last 7 days without use and 30 days at most,
the same limits Grafana uses by default.

- **Password.** At least 15 characters; a few unrelated words make a good
  passphrase. After five wrong guesses each further attempt waits longer, up
  to 15 minutes. A browser that has signed in before keeps its own count of
  wrong guesses, so someone guessing from elsewhere cannot lock you out of it.
  When single sign-on is set to redirect to the provider automatically,
  `/login?local=1` still shows the password form.
- **Single sign-on (OIDC).** Any standards-compliant provider: Authentik,
  Authelia, Keycloak, Pocket ID, Kanidm, Zitadel, Google. Create a client with
  the redirect URI Settings shows (`<your origin>/auth/oidc/callback`), enter
  its issuer URL, client ID and secret, then press **Link account**. Only the
  account you link can sign in, so another account at the same provider is
  refused. The **Provider** picker puts that provider's logo on the sign-in
  button (a key icon for Pocket ID, Kanidm, Zitadel and Other).
- **Sign in with Strava.** Uses the Strava app you already connected, so
  nothing changes on Strava's side. The sign-in page shows it as the
  **Connect with Strava** button. Only the athlete ActivSync is connected to
  can sign in. Signing out never disconnects Strava.
- **Trusted proxy header.** For an authenticating reverse proxy. Set the three
  `ACTIVSYNC_AUTH_PROXY_*` variables; ActivSync refuses to start if one is
  missing or the trusted range is everything. The proxy's own sign-out signs
  you out here too.

| Proxy | Header |
|---|---|
| Authelia | `Remote-User` |
| authentik outpost | `X-authentik-username` |
| oauth2-proxy | `X-Forwarded-User` or `X-Forwarded-Preferred-Username` |
| Tailscale serve | `Tailscale-User-Login` |

The proxy must also remove any copy of that header a client sends. Authelia,
authentik and Tailscale do. oauth2-proxy trusts every address by default; set
`--trusted-proxy-ip` to your proxy's range.

**Sign out everywhere** in Settings and changing your password also make every
browser sign in from scratch, as if it had never signed in before.

### Locked out?

Run the recovery commands inside the container:

```sh
docker compose exec activsync activsync-auth reset-password       # new password, signs everyone out
docker compose exec activsync activsync-auth revoke-sessions      # sign everyone out
docker compose exec activsync activsync-auth disable-oidc         # remove a broken SSO setup
docker compose exec activsync activsync-auth disable-strava-login
docker compose exec activsync activsync-auth status               # which methods are on; no secrets
```

`reset-password` and `revoke-sessions` also forget every browser that has
signed in before.

## Exposing ActivSync publicly

- **Put it behind an HTTPS reverse proxy.** Sessions use a `__Host-` cookie
  marked `Secure` once requests arrive as HTTPS. Set `FORWARDED_ALLOW_IPS` to
  the proxy's address so ActivSync sees the original scheme and each client's
  own address. Without it every visitor shares the proxy's address and its
  sign-in rate limit; with `*` a client can pick the address it is limited
  under.
- **Claim it before you publish it.** The setup code is in the server log only,
  so finding a fresh install is not enough to take it over, but there is no
  reason to leave it unclaimed.
- **Set `ACTIVSYNC_PUBLIC_URL`.** OIDC and Strava redirect URIs are then built
  from it, it becomes the only accepted `Host`, and cross-site requests are
  checked against it. Without it, ActivSync accepts only IP addresses,
  `localhost`, and the names listed in `ACTIVSYNC_ALLOWED_HOSTS`, so a web page
  cannot reach it through a domain that merely resolves to your server (DNS
  rebinding). Opening it by a LAN name such as `activsync.lan` therefore needs
  that name in `ACTIVSYNC_ALLOWED_HOSTS`.
- **Running without Docker?** Start uvicorn with `--no-proxy-headers`, as the
  image does. ActivSync applies `FORWARDED_ALLOW_IPS` itself, after recording
  the real connecting address that proxy-header sign-in trusts.

Upgrading from a version without sign-in: the first start stops at the claim
step, even though setup was finished before. Read the code from the log as
above. ActivSync now also asks Strava for the `read` scope, so the first
**Connect with Strava** in **Settings → Connections** after upgrading shows
Strava's consent screen once.

## Hevy workouts

Connect Hevy under **Settings → Connections** with an API key from your Hevy
account — the first-run wizard also offers it as an optional step. The Hevy API
requires a Hevy Pro subscription. ActivSync then polls for new workouts and
tries to match each one to the Garmin activity your watch recorded for the same
session.

**A matched workout waits for you.** Before anything on Garmin changes, the
Hevy page offers four choices:

| Choice | What it does |
|---|---|
| **Merge** | Writes the Hevy exercise sets onto the watch activity, keeping its heart rate and duration |
| **Replace** | Rebuilds the activity from the Hevy workout, carrying the watch's heart rate across |
| **Description only** | Leaves the activity's data alone and writes the rendered summary |
| **Skip** | Leaves the workout alone; it returns to the picker |

Settings can switch match handling to **Automatic** and pick the default
strategy, in which case matched workouts apply without pausing.

An **unmatched** workout — a session your watch never recorded — uploads as
its own Garmin activity once the configured grace period has passed.

**Backfill** imports historical workouts. Give it a start date and it previews
exactly what it would do to each workout, including any exercise it has no
Garmin mapping for, before writing anything. Exercises that ActivSync cannot
place are surfaced so you can map them yourself under **Hevy → Exercise
mappings**.

On Strava, a Hevy-linked activity is uploaded with each set named from
Strava's exercise library; heart rate, calories and timing still come from
the watch. **Hevy → Exercise mappings** shows both the Garmin and the Strava
exercise for every Hevy exercise, and either can be overridden. Strava
does not show the device name on these uploads.

The **description template** is editable in Settings, with placeholders for the
workout title, duration, calories, average heart rate, exercise summary, and
the ActivSync marker; a live sample renders beside the editor. The shared
Garmin/Strava description format is plain text — line breaks, Unicode, emoji
and bullets all work, while Markdown and HTML stay literal. **Description
only** always writes the summary; for **Merge** and **Replace** it can be
switched off while still applying the structured sets.

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
`data/activsync-dev.db`, signs in with the dev password
`activsync-dev-password`, and never contacts Garmin or Strava — your real
`data/activsync.db` is never touched. To walk the first-run wizard from a clean
slate, including the claim step, use `make dev-fresh`; its setup code is printed
in the terminal.

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
and update-check background jobs. The Hevy polling leg stays active so the
queue receives new workouts, and this mode always pauses on a match for your
decision regardless of the Automatic setting — see
[Hevy workouts](#hevy-workouts) for what the choices do.

Matched workouts remain publish-blocked while waiting. The normal Strava
publishing and status polls stay off, although the Hevy leg may still refresh
metadata on an already-linked Strava copy. The command refuses to start if the real database is missing or ActivSync is
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
