"""Body cap for the public POST routes. They parse JSON before any check can
reject the caller, so an anonymous request body is bounded here instead."""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Iterable, Mapping
from typing import Any

from starlette.datastructures import Headers

PUBLIC_BODY_LIMIT = 16 * 1024
TOO_LARGE = "That request is too large."

Message = Mapping[str, Any]
Receive = Callable[[], Awaitable[Message]]


def _replay(messages: Iterable[Message], receive: Receive) -> Receive:
    pending = iter(messages)

    async def replayed() -> Message:
        message = next(pending, None)
        return message if message is not None else await receive()

    return replayed


async def capped_receive(scope: Mapping, receive: Receive, limit: int) -> Receive | None:
    """The `receive` to hand the app, or None when the body is over `limit`.
    A declared length is judged up front; without one the body is read only as
    far as `limit` and replayed."""
    declared = Headers(scope=scope).get("content-length", "")
    if declared.isdigit():
        return None if int(declared) > limit else receive
    chunks: list[bytes] = []
    size = 0
    while True:
        message = await receive()
        if message["type"] != "http.request":
            return _replay([message], receive)
        size += len(message.get("body", b""))
        if size > limit:
            return None
        chunks.append(message.get("body", b""))
        if not message.get("more_body", False):
            break
    body = {"type": "http.request", "body": b"".join(chunks), "more_body": False}
    return _replay([body], receive)
