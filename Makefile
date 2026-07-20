.PHONY: install dev dev-api dev-web dev-fresh test test-python test-web

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

# Run the test suite.
test: test-python test-web

test-python:
	.venv/bin/python -m pytest

test-web:
	npm run typecheck
	npm run lint
	npm test
