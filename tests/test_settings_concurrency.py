"""The `settings` blob is read-modify-written from two threads."""

import threading
import time

import pytest

from activsync import config, db, hevy_apply
from activsync.fit_builder import DeviceIdentity


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def test_identity_write_does_not_clobber_a_concurrent_settings_save(conn, monkeypatch):
    """The poller detects a device identity while the user saves settings.

    Both sides read the whole blob, change one key and write it back. Without
    the read and the write held together, whichever writes second silently
    drops the other's change.
    """
    config.save_config(conn, {**config.load_config(conn), "hevy_grace_minutes": 120})

    real_get = db.get_config_value
    reader_entered = threading.Event()

    def slow_get(c, key, default=None):
        value = real_get(c, key, default)
        if key == "settings" and threading.current_thread().name == "poller":
            reader_entered.set()
            time.sleep(0.3)  # long enough that an unserialised save lands here
        return value

    monkeypatch.setattr(db, "get_config_value", slow_get)

    def persist_identity():
        hevy_apply._persist_identity(
            conn, DeviceIdentity(manufacturer=1, product=4534, serial=42))

    def save_settings():
        reader_entered.wait(timeout=2)
        cfg = config.load_config(conn)
        cfg["hevy_grace_minutes"] = 45
        config.save_config(conn, cfg)

    poller = threading.Thread(target=persist_identity, name="poller")
    request = threading.Thread(target=save_settings, name="request")
    poller.start()
    request.start()
    poller.join(timeout=5)
    request.join(timeout=5)

    stored = real_get(conn, "settings", default={})
    assert stored.get("hevy_device_identity") == {
        "manufacturer": 1, "product": 4534, "serial": 42}
    assert stored.get("hevy_grace_minutes") == 45


def test_settings_save_does_not_clobber_a_concurrent_identity_write(conn, monkeypatch):
    """The mirror image: the request thread reads first, the poller's identity
    write lands, and the save must not write its pre-identity copy back."""
    config.save_config(conn, {**config.load_config(conn), "hevy_grace_minutes": 120})

    real_get = db.get_config_value
    reader_entered = threading.Event()

    def slow_get(c, key, default=None):
        value = real_get(c, key, default)
        if key == "settings" and threading.current_thread().name == "request":
            reader_entered.set()
            time.sleep(0.3)
        return value

    monkeypatch.setattr(db, "get_config_value", slow_get)

    def save_settings():
        with config.editing(conn) as cfg:
            cfg["hevy_grace_minutes"] = 45

    def persist_identity():
        reader_entered.wait(timeout=2)
        hevy_apply._persist_identity(
            conn, DeviceIdentity(manufacturer=1, product=4534, serial=42))

    request = threading.Thread(target=save_settings, name="request")
    poller = threading.Thread(target=persist_identity, name="poller")
    request.start()
    poller.start()
    request.join(timeout=5)
    poller.join(timeout=5)

    stored = real_get(conn, "settings", default={})
    assert stored.get("hevy_grace_minutes") == 45
    assert stored.get("hevy_device_identity") == {
        "manufacturer": 1, "product": 4534, "serial": 42}
