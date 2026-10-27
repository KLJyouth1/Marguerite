"""Global bounded event ring buffer for WebSocket reconnect resync (P2-10).

When a WS client reconnects it hands back the last event ``seq`` it consumed.
The server then either:

* **replays** the missing frames, when the requested gap is still inside the
  bounded ring buffer; or
* **aligns to a snapshot**, when the client is so far behind that the gap has
  already been evicted from the buffer — we do not fabricate old frames, we hand
  back the most recent window so the client can re-sync state.

This sits *alongside* the per-session ``SessionStreamRegistry`` (which owns
per-session ``stream_seq`` replay). The resync buffer here keeps its own
monotonic global ``seq`` and a channel tag (the session key) so a client can
filter to the sessions it cares about.

Pure decision helpers (:func:`gap_range`, :func:`should_snapshot`) are extracted
so the gap/snapshot policy is unit-testable without a gateway.
"""

from __future__ import annotations

import time
from collections import deque
from dataclasses import dataclass, field
from typing import Iterable, Mapping

DEFAULT_CAPACITY = 1024


def _epoch_ms() -> int:
    return time.time_ns() // 1_000_000


def should_snapshot(last_seq: int, first_buffered_seq: int) -> bool:
    """Return True when the requested gap has been evicted from the buffer.

    ``last_seq == first_buffered_seq - 1`` is exactly recoverable (the client is
    one frame behind the oldest retained frame, and the replay starts at
    ``first_buffered_seq``). Anything further back means frames were dropped and
    we must align to a snapshot instead of fabricating history.
    """

    if first_buffered_seq <= 0:
        # Buffer empty but a stream exists: a reset happened; align to snapshot.
        return last_seq > 0
    return last_seq < first_buffered_seq - 1


def gap_range(
    last_seq: int,
    current_seq: int,
    first_buffered_seq: int,
) -> tuple[int, int, str]:
    """Classify the gap between ``last_seq`` and the current head.

    Returns ``(from_seq, to_seq, mode)`` where ``mode`` is ``"replay"`` or
    ``"snapshot"``. ``from_seq == to_seq + 1`` means there is nothing to replay
    (client already caught up).
    """

    if last_seq >= current_seq:
        # Ahead of (or exactly at) the head: nothing to do.
        return current_seq + 1, current_seq, "replay"
    if should_snapshot(last_seq, first_buffered_seq):
        return last_seq + 1, current_seq, "snapshot"
    return last_seq + 1, current_seq, "replay"


@dataclass(frozen=True)
class ResyncFrame:
    seq: int
    type: str
    channel: str
    data: Mapping[str, object]
    ts: int = field(default_factory=_epoch_ms)

    def to_dict(self) -> dict[str, object]:
        return {
            "seq": self.seq,
            "type": self.type,
            "channel": self.channel,
            "data": dict(self.data),
            "ts": self.ts,
        }


class EventResyncBuffer:
    """Bounded global ring of recent event frames.

    Control/handshake frames and high-frequency lossy deltas are *not* buffered
    (they are safe to drop and would just evict useful frames). Business event
    frames are retained until the ring evicts them.
    """

    #: Event name prefixes that must never enter the resync ring.
    _SKIP_PREFIXES = (
        "session.event.text_delta",
        "session.event.run_heartbeat",
        "session.event.tool_use_delta",
    )

    def __init__(self, *, capacity: int = DEFAULT_CAPACITY) -> None:
        self._capacity = max(1, int(capacity))
        self._buf: deque[ResyncFrame] = deque(maxlen=self._capacity)
        self._seq = 0

    @property
    def current_seq(self) -> int:
        return self._seq

    @property
    def capacity(self) -> int:
        return self._capacity

    def _is_bufferable(self, event_name: str) -> bool:
        return not event_name.startswith(self._SKIP_PREFIXES)

    def record(
        self,
        event_name: str,
        payload: Mapping[str, object] | None,
        *,
        channel: str | None = None,
    ) -> ResyncFrame | None:
        """Append one business frame. Returns the frame, or None if skipped."""

        if not event_name or not self._is_bufferable(event_name):
            return None
        self._seq += 1
        frame = ResyncFrame(
            seq=self._seq,
            type=event_name,
            channel=str(channel or "*"),
            data=dict(payload or {}),
        )
        self._buf.append(frame)
        return frame

    def _channel_ok(self, frame: ResyncFrame, channels: Iterable[str] | None) -> bool:
        if not channels:
            return True
        wanted = {str(c) for c in channels}
        return frame.channel in wanted

    def resync(
        self,
        last_seq: int,
        *,
        channels: Iterable[str] | None = None,
    ) -> dict[str, object]:
        """Return the resync result for a reconnecting client.

        Response contract (for Rust/frontend):

        * replay mode: ``{ok, resync_mode:"replay", from_seq, to_seq, last_seq,
          events:[...], has_more:false}``
        * snapshot mode: ``{ok, resync_mode:"snapshot", last_seq, events:[...snapshot
          window...], has_more:false}``
        """

        last_seq = max(0, int(last_seq))
        current = self._seq
        if last_seq >= current:
            return {
                "ok": True,
                "resync_mode": "replay",
                "from_seq": current + 1,
                "to_seq": current,
                "last_seq": current,
                "events": [],
                "has_more": False,
            }

        first_buffered = self._buf[0].seq if self._buf else 0
        from_seq, to_seq, mode = gap_range(last_seq, current, first_buffered)

        if mode == "snapshot":
            # Align to the most recent retained window for the requested channels.
            snap = [
                frame.to_dict()
                for frame in self._buf
                if self._channel_ok(frame, channels)
            ]
            return {
                "ok": True,
                "resync_mode": "snapshot",
                "from_seq": from_seq,
                "to_seq": current,
                "last_seq": current,
                "events": snap,
                "has_more": False,
            }

        events = [
            frame.to_dict()
            for frame in self._buf
            if frame.seq > last_seq and frame.seq <= current and self._channel_ok(frame, channels)
        ]
        return {
            "ok": True,
            "resync_mode": "replay",
            "from_seq": from_seq,
            "to_seq": current,
            "last_seq": current,
            "events": events,
            "has_more": False,
        }


_buffer: EventResyncBuffer | None = None


def get_event_resync_buffer() -> EventResyncBuffer:
    global _buffer
    if _buffer is None:
        _buffer = EventResyncBuffer()
    return _buffer


def reset_event_resync_buffer() -> None:
    """Test hook: drop the singleton so tests do not share state."""

    global _buffer
    _buffer = None


__all__ = [
    "DEFAULT_CAPACITY",
    "EventResyncBuffer",
    "ResyncFrame",
    "gap_range",
    "get_event_resync_buffer",
    "reset_event_resync_buffer",
    "should_snapshot",
]
