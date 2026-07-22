.PHONY: install dev dev-api dev-web dev-fresh dev-real dev-real-check test test-python test-web

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
# first-run setup wizard so the whole onboarding flow can be tested.
dev-fresh:
	rm -f data/activsync-dev.db data/activsync-dev.db-shm data/activsync-dev.db-wal
	$(MAKE) dev

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
