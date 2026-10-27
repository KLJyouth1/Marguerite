"""RPC handlers for reconnect resync (P2-10).

``events.resync`` lets a WebSocket client that just reconnected hand back the
last event ``seq`` it consumed and either receive the missing frames (replay) or
align to the most recent window (snapshot) when it has fallen out of the bounded
ring buffer.
"""

from __future__ import annotations

from typing import Any

from opensquilla.gateway.event_resync import get_event_resync_buffer
from opensquilla.gateway.rpc import RpcContext, get_dispatcher

_d = get_dispatcher()


def _as_int(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def _as_channels(value: Any) -> list[str] | None:
    if value is None:
        return None
    if isinstance(value, str):
        return [value] if value else None
    if isinstance(value, (list, tuple)):
        cleaned = [str(c) for c in value if str(c)]
        return cleaned or None
    return None


@_d.method("events.resync", scope="operator.read")
async def _handle_events_resync(params: dict | None, ctx: RpcContext) -> dict[str, Any]:
    """Replay missed event frames, or align to a snapshot after a reconnect.

    Params:
        last_seq: last event seq the client consumed (0 = start).
        channels: optional list of session keys to filter by.

    Returns the resync result shape documented in
    :meth:`opensquilla.gateway.event_resync.EventResyncBuffer.resync`, augmented
    with the buffer head for the client to continue from.
    """

    params = params or {}
    last_seq = _as_int(params.get("last_seq", params.get("lastSeq")))
    channels = _as_channels(params.get("channels"))
    buffer = get_event_resync_buffer()
    result = buffer.resync(last_seq, channels=channels)
    result["buffer_capacity"] = buffer.capacity
    return result


__all__ = []
