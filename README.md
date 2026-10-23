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
| `ACTIVSYNC_AUTH_PROXY_HEADER` | _(unset)_ | Username header from an authenticating proxy, e.g. `Remote-User` (see [Through an authenticating proxy](#through-an-authenticating-proxy)) |
| `ACTIVSYNC_AUTH_PROXY_TRUSTED` | _(unset)_ | The proxy's IP, e.g. `172.18.0.10/32`. Required with the header |
| `ACTIVSYNC_AUTH_PROXY_TRUST_GATEWAY` | _(unset)_ | `1` when the proxy runs on the Docker host itself |
| `ACTIVSYNC_AUTH_PROXY_USER` | _(unset)_ | The exact header value that is you. Required with the header |
| `ACTIVSYNC_AUTH_DISABLED` | _(unset)_ | `1` turns sign-in off entirely. LAN-only installs that accept the risk |

The in-app **Settings → Preferences → display timezone** overrides `TZ` for log
timestamps (applied live, no restart needed).

### Strava's Authorization Callback Domain

The setup wizard shows the exact value to enter. Only one domain can be
registered (its subdomains work too), so if you reach ActivSync at more than
one address, press **Connect with Strava** from the registered one. Behind an
HTTPS proxy, set `FORWARDED_ALLOW_IPS` so the callback is built as `https://`
(see [Exposing ActivSync publicly](#exposing-activsync-publicly)).

## Signing in

ActivSync has one user: you. Your password always works. Single sign-on and
proxy sign-in are optional extras on top of it. Sessions last 7 days without
use and 30 days at most.

### Password

Chosen when you claim the install; at least 15 characters. Repeated wrong
guesses are slowed down, up to 15 minutes, but a browser you have signed in
from before is never locked out by someone guessing elsewhere.

### Single sign-on (OIDC)

Works with any OIDC provider: Authentik, Authelia, Keycloak, Pocket ID, Kanidm,
Zitadel, Google.

1. At your provider, create a client with the redirect URI shown in
   **Settings → Access** (`<your origin>/auth/oidc/callback`).
2. In **Settings → Access**, enter the issuer URL, client ID and secret.
3. Press **Link account**. Only the account you link can sign in.

**Go straight to the provider** skips the sign-in page and sends you to your
provider right away; `/login?local=1` still shows the password form.
**Upload logo** puts the provider's logo on the sign-in button.

### Through an authenticating proxy

If your reverse proxy already signs you in, ActivSync can trust the username
header it forwards:

```yaml
    environment:
      - ACTIVSYNC_AUTH_PROXY_HEADER=Remote-User
      - ACTIVSYNC_AUTH_PROXY_USER=you              # the header value that is you
      - ACTIVSYNC_AUTH_PROXY_TRUSTED=172.18.0.10/32  # your proxy's IP, nothing wider
```

| Proxy | Header |
|---|---|
| Authelia | `Remote-User` |
| authentik outpost | `X-authentik-username` |
| Pangolin | `Remote-User` (see [below](#pangolin)) |
| oauth2-proxy | `X-Forwarded-User` (also set `--trusted-proxy-ip`) |
| Tailscale serve | `Tailscale-User-Login` |

**Only the proxy may reach ActivSync.** Trust just the proxy's own IP (a
`/32`) and keep port `8381` away from everyone else: remove `ports:`, use
`127.0.0.1:8381:8381`, or firewall it to the proxy. ActivSync refuses to start if the trusted range
includes Docker's gateway, since every connection from outside Docker arrives
from it. If the proxy runs on the Docker host itself, set
`ACTIVSYNC_AUTH_PROXY_TRUST_GATEWAY=1` and bind the port to `127.0.0.1`.

Not sure what value the proxy sends? Sign in through it once: ActivSync logs
the value it received when it doesn't match `ACTIVSYNC_AUTH_PROXY_USER`.

#### Pangolin

Protect the resource with Pangolin SSO and set the header to `Remote-User`.
Pangolin sends it only after an SSO sign-in; with a PIN, password or email code
you get ActivSync's own sign-in page instead.

Trust the address of the machine that runs your Pangolin connector (Newt), not
the Pangolin server: the connector is what opens the connection to ActivSync.

- **Connector on another machine or VM:** trust that machine's IP, e.g.
  `ACTIVSYNC_AUTH_PROXY_TRUSTED=192.168.1.20/32`, and set
  `FORWARDED_ALLOW_IPS` to the same IP. Port `8381` has to stay published, so
  firewall it to that machine.
- **Connector in Docker next to ActivSync:** give the connector container a
  fixed IP on a shared network, trust that, and drop `ports:`.

If you are unsure which address it connects from, sign in through Pangolin
once: ActivSync logs `ignored Remote-User from untrusted peer <address>`.

### Locked out?

```sh
docker compose exec activsync activsync-auth reset-password    # new password, signs everyone out
docker compose exec activsync activsync-auth revoke-sessions   # sign everyone out
docker compose exec activsync activsync-auth disable-oidc      # remove a broken SSO setup
docker compose exec activsync activsync-auth status            # which methods are on
```

**Sign out everywhere** in Settings, or changing your password, also signs out
every other browser.

## Exposing ActivSync publicly

1. **Claim it first.** The setup code is only in the server log, but there's
   no reason to leave a fresh install unclaimed online.
2. **Put it behind an HTTPS reverse proxy** and set:
   - `ACTIVSYNC_PUBLIC_URL=https://activsync.example.com`: the only host name
     ActivSync answers to, also used for SSO and Strava redirects.
   - `FORWARDED_ALLOW_IPS=<your proxy's IP>`: so ActivSync sees HTTPS and each
     visitor's real IP for sign-in rate limits. Don't use `*`; it lets
     visitors pick their own IP.
3. **Opening it by a LAN name** (`activsync.lan`, a `.local` or Tailscale name)
   without a public URL? Add it to `ACTIVSYNC_ALLOWED_HOSTS`, or every page
   returns `400 Unknown host`. IP addresses and `localhost` always work.

Running without Docker? Start uvicorn with `--no-proxy-headers`, as the image
does; ActivSync applies `FORWARDED_ALLOW_IPS` itself.

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
