"""Lightweight in-process pub/sub for real-time SSE updates.

Thread-safe: ``publish()`` may be called from any thread (the poller runs in a
background thread while the SSE endpoint runs in the asyncio event loop).
"""

from __future__ import annotations

import asyncio
import logging
import threading

logger = logging.getLogger("activsync.events")


class EventBus:
    """Broadcasts string events to all currently-connected listeners.

    Each listener is an ``asyncio.Queue[str]``; when :meth:`publish` is
    called the event is pushed onto every active queue.  There is no
    persistence — late-joining listeners only see future events.
    """

    def __init__(self):
        self._queues: set[asyncio.Queue[str]] = set()
        self._lock = threading.Lock()
        self._loop: asyncio.AbstractEventLoop | None = None

    def _set_loop(self, loop: asyncio.AbstractEventLoop) -> None:
        self._loop = loop

    def subscribe(self) -> asyncio.Queue[str]:
        q: asyncio.Queue[str] = asyncio.Queue()
        with self._lock:
            self._queues.add(q)
        return q

    def unsubscribe(self, q: asyncio.Queue[str]) -> None:
        with self._lock:
            self._queues.discard(q)

    def publish(self, event: str) -> None:
        with self._lock:
            if not self._queues:
                return
            queues = list(self._queues)
        logger.debug("event bus: %s (%d listeners)", event, len(queues))
        for q in queues:
            try:
                q.put_nowait(event)
            except asyncio.QueueFull:
                pass


# module-level singleton — the server and poller share the same process
bus = EventBus()
