.PHONY: install dev dev-api dev-web dev-fresh dev-real dev-real-check test test-python test-web dev-rust dev-rust-fresh dev-api-rust dev-real-rust dev-api-real-rust test-rust docker-rust

# Create the virtualenv and install the app with dev extras (editable).
install:
	python3 -m venv .venv
	.venv/bin/pip install -e ".[dev]"
	npm install

# Run the React app on :8382 and the mock-only API on :8383.
# Never touches the real database or contacts Garmin/Strava.
dev:
	npm run dev

dev-api:
	npm run dev:api

dev-web:
	npm run dev:web

# Wipe the dev database first, then run the dev server. Starts at a clean
# first-run setup wizard, claim step included (the setup code is printed in
# this terminal), so the whole onboarding flow can be tested.
dev-fresh:
	rm -f data/activsync-dev.db data/activsync-dev.db-shm data/activsync-dev.db-wal
	ACTIVSYNC_DEV_SEED_OWNER=0 $(MAKE) dev

# Run the development branch against real account state. Garmin and Strava
# polling stay disabled; the Hevy leg runs with matched workouts held for an
# explicit per-workout strategy choice by default. The normal app must be
# stopped so two instances cannot act on the same database/accounts.
REAL_DB_PATH ?= data/activsync.db

dev-real: dev-real-check
	REAL_DB_PATH="$(REAL_DB_PATH)" npm run dev:real

dev-real-check:
	@test -f "$(REAL_DB_PATH)" || (echo "Refusing to start: real database not found at $(REAL_DB_PATH)"; exit 1)
	@if curl -fsS --max-time 1 http://127.0.0.1:8381/health >/dev/null 2>&1; then \
		echo "Refusing to start: ActivSync is already running on :8381. Stop it first to avoid two instances using your real accounts."; \
		exit 1; \
	fi

# Run the test suite.
test: test-python test-web

test-python:
	.venv/bin/python -m pytest

test-web:
	npm run typecheck
	npm run lint
	npm test

# The Rust port of the server. `dev-rust` uses mock data and its own
# data/activsync-rust-dev.db; `dev-real-rust` is the Rust twin of `dev-real`.
CARGO ?= $(shell command -v cargo 2>/dev/null || echo $(HOME)/.cargo/bin/cargo)

dev-rust:
	npx concurrently --kill-others --names api-rs,web --prefix-colors magenta,green "$(MAKE) dev-api-rust" "npm run dev:web"

dev-api-rust:
	ACTIVSYNC_DEV_MOCK_DATA=1 $(CARGO) run --manifest-path rust/Cargo.toml

# Wipe the Rust dev database and start at the first step of the setup wizard.
dev-rust-fresh:
	rm -f data/activsync-rust-dev.db data/activsync-rust-dev.db-shm data/activsync-rust-dev.db-wal
	ACTIVSYNC_DEV_FRESH=1 $(MAKE) dev-rust

# Real accounts, as `dev-real`: same database and checks, Garmin and Strava
# polling off, Hevy matches held for review.
dev-real-rust: dev-real-check
	npx concurrently --kill-others --names api-rs-real,web --prefix-colors yellow,green "$(MAKE) dev-api-real-rust" "npm run dev:web"

dev-api-real-rust: dev-real-check
	ACTIVSYNC_DEV_MOCK_DATA=0 ACTIVSYNC_MANUAL_ONLY=1 ACTIVSYNC_DB_PATH="$(REAL_DB_PATH)" ACTIVSYNC_GARMIN_TOKEN_DIR=data/.garminconnect $(CARGO) run --manifest-path rust/Cargo.toml

docker-rust:
	docker build -f rust/Dockerfile -t activsync-rust .

test-rust:
	$(CARGO) test --manifest-path rust/Cargo.toml
