"""Observability primitives — an in-process event bus plus append-only session logs.

Mirrors the two DSH patterns the upgrade plan adopts (docs/UPGRADE_PLAN_V2.md §0):

* **EventBus** — synchronous pub/sub with a bounded replay buffer, so UIs can
  poll recent events without wiring a broker (SSE fan-out is explicitly
  NOT IMPLEMENTED; see plan).
* **SessionLog** — every agent run appends one JSON object per line to
  ``data_dir/sessions/<session_id>.jsonl``: crash-safe, grep-able, replayable.
  Files are never rewritten (append-only invariant).
"""

from __future__ import annotations

import json
import threading
import time
import uuid
from collections import deque
from pathlib import Path
from typing import Any, Callable


def now_ms() -> int:
    return int(time.time() * 1000)


class EventBus:
    """Publish → notify subscribers → keep a bounded ring for late readers."""

    def __init__(self, replay_size: int = 256) -> None:
        if replay_size < 1:
            raise ValueError("replay_size must be >= 1")
        self._subs: list[Callable[[dict], None]] = []
        self._ring: deque[dict] = deque(maxlen=replay_size)
        self._lock = threading.Lock()

    def subscribe(self, fn: Callable[[dict], None]) -> Callable[[dict], None]:
        self._subs.append(fn)
        return fn

    def unsubscribe(self, fn: Callable[[dict], None]) -> None:
        if fn in self._subs:
            self._subs.remove(fn)

    def publish(self, event_type: str, payload: dict[str, Any] | None = None) -> dict:
        event = {
            "type": event_type,
            "ts": now_ms(),
            "seq": next_seq(),
            "payload": payload or {},
        }
        with self._lock:
            self._ring.append(event)
            subs = list(self._subs)
        for fn in subs:
            try:
                fn(event)
            except Exception:  # noqa: BLE001 — a broken subscriber must not stall publishers
                continue
        return event

    def recent(self, n: int = 20) -> list[dict]:
        with self._lock:
            items = list(self._ring)
        return items[-max(1, n):]


_SEQ_LOCK = threading.Lock()
_SEQ = 0


def next_seq() -> int:
    global _SEQ
    with _SEQ_LOCK:
        _SEQ += 1
        return _SEQ


class SessionLog:
    """Append-only JSONL transcript for one session id.

    Durability is unchanged from the original per-event implementation: every
    ``append`` still lands on disk before returning.  What changed (W8, the
    SSD write-amplification item) is *how* it gets there: the file handle used
    to be opened and closed for every single event, which on Windows costs a
    path re-resolution plus a sharing/AV check per event on top of the write
    itself.  The handle is now opened once, kept in append mode, and flushed
    per line, so the transcript keeps its "never rewritten" invariant while
    the syscall count drops from ``2 * events`` to ``O(1)``.
    """

    def __init__(self, sessions_dir: Path, session_id: str | None = None) -> None:
        self.session_id = session_id or f"s_{time.strftime('%Y%m%d_%H%M%S')}_{uuid.uuid4().hex[:6]}"
        self.path = Path(sessions_dir) / f"{self.session_id}.jsonl"
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self._lock = threading.Lock()
        self._fh = None
        self.appends = 0
        self.opens = 0

    def _handle(self) -> Any:
        """Return the shared append handle, opening it on first use or after close."""
        if self._fh is None:
            self._fh = self.path.open("a", encoding="utf-8")
            self.opens += 1
        return self._fh

    def append(self, record: dict[str, Any]) -> dict:
        line = json.dumps(record, ensure_ascii=False, default=str) + "\n"
        with self._lock:
            fh = self._handle()
            fh.write(line)
            fh.flush()
            self.appends += 1
        return record

    def read_all(self) -> list[dict]:
        if not self.path.exists():
            return []
        out: list[dict] = []
        with self.path.open("r", encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if line:
                    out.append(json.loads(line))
        return out

    def close(self) -> None:
        """Release the handle; a later ``append`` transparently reopens it."""
        with self._lock:
            if self._fh is not None:
                try:
                    self._fh.flush()
                finally:
                    self._fh.close()
                    self._fh = None

    def __enter__(self) -> "SessionLog":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def __del__(self) -> None:  # pragma: no cover — safety net, not a contract
        try:
            self.close()
        except Exception:  # noqa: BLE001 — interpreter teardown must not raise
            pass


class PollingTap:
    """Thread-safe event buffer bridging sync publishers → async SSE consumers.

    The agent loop publishes from worker threads while the SSE generator runs
    on the event loop; a plain ``asyncio.Queue`` cannot receive cross-thread
    puts safely.  This tap uses a bounded deque + condition variable:
    ``push`` never blocks the publisher, ``wait`` blocks the (single) consumer
    up to a timeout so the generator can emit heartbeat comments.
    """

    def __init__(self, capacity: int = 128) -> None:
        if capacity < 1:
            raise ValueError("capacity must be >= 1")
        import threading

        self._buf: deque[dict] = deque(maxlen=capacity)
        self._cond = threading.Condition()
        self._closed = False

    def push(self, event: dict) -> None:
        with self._cond:
            if not self._closed:
                self._buf.append(event)
                self._cond.notify()

    def wait(self, timeout: float = 1.0) -> list[dict]:
        """Drain buffered events; empty list on timeout (drives heartbeats)."""
        with self._cond:
            if not self._buf and not self._closed:
                self._cond.wait(timeout)
            items = list(self._buf)
            self._buf.clear()
            return items

    def close(self) -> None:
        with self._cond:
            self._closed = True
            self._cond.notify()
