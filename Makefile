.PHONY: install dev dev-fresh test

# Create the virtualenv and install the app with dev extras (editable).
install:
	python3 -m venv .venv
	.venv/bin/pip install -e ".[dev]"

# Run the local dev server: auto-reload, mock data, isolated dev database.
# Never touches the real database or contacts Garmin/Strava.
dev:
	ACTIVSYNC_DEV_MOCK_DATA=1 \
	ACTIVSYNC_DB_PATH=data/activsync-dev.db \
	.venv/bin/uvicorn activsync.main:app --reload --reload-dir src --port 8382

# Wipe the dev database first, then run the dev server. Starts at a clean
# first-run setup wizard so the whole onboarding flow can be tested.
dev-fresh:
	rm -f data/activsync-dev.db data/activsync-dev.db-shm data/activsync-dev.db-wal
	$(MAKE) dev

# Run the test suite.
test:
	.venv/bin/python -m pytest
