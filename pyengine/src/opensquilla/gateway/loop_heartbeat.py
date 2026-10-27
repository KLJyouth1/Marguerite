"""Event-loop liveness heartbeat for deep readiness probes (P0-2).

The static ``/health`` endpoint answers 200 as long as TCP can answer it —
including when the gateway event loop has wedged (deadlock, a blocking call,
a stuck turn). That is the "living-but-dead" process the Rust watchdog cannot
see with a static probe.

This module provides:

- :func:`run_loop_heartbeat` — a background coroutine that writes a
  wall-clock timestamp to ``state.loop_last_tick`` once per second. It only
  progresses while the gateway event loop is actually turning, so a stopped
  loop stops the ticks.
- :func:`staleness` — a pure function that turns ``now - last_tick`` into a
  fresh/stalled judgement without any time access of its own.
- :func:`stall_threshold_secs` — reads ``MARGUERITE_SIDECAR_STALL_SECS``
  (default 15s).
"""

from __future__ import annotations

import asyncio
import os
import time
from typing import Any

DEFAULT_STALL_SECS = 15.0
HEARTBEAT_INTERVAL_SECS = 1.0
STALL_THRESHOLD_ENV = "MARGUERITE_SIDECAR_STALL_SECS"


def stall_threshold_secs() -> float:
    """Read the stall threshold (seconds) from the environment.

    ``MARGUERITE_SIDECAR_STALL_SECS`` overrides the 15s default. Missing,
    empty, unparsable, or non-positive values all fall back to the default
    rather than crashing the readiness probe.
    """
    raw = os.environ.get(STALL_THRESHOLD_ENV)
    if not raw:
        return DEFAULT_STALL_SECS
    try:
        value = float(raw)
    except ValueError:
        return DEFAULT_STALL_SECS
    return value if value > 0 else DEFAULT_STALL_SECS


def staleness(
    now: float,
    last_tick: float | None,
    threshold_secs: float,
) -> dict[str, Any]:
    """Judge how stale the last heartbeat tick is — pure, no wall-clock reads.

    Returns a dict with:

    - ``fresh``: True when the loop ticked within the threshold (or when no
      tick has been recorded yet — startup calls must not self-report stalled).
    - ``stalled``: True when ``now - last_tick`` strictly exceeds the
      threshold; the event loop is not turning.
    - ``age_secs``: seconds since the last tick (``None`` before the first
      tick).
    - ``threshold_secs``: echoed back for diagnostics.
    """
    if last_tick is None:
        return {
            "fresh": True,
            "stalled": False,
            "age_secs": None,
            "threshold_secs": threshold_secs,
        }
    age = now - last_tick
    stalled = age > threshold_secs
    return {
        "fresh": not stalled,
        "stalled": stalled,
        "age_secs": age,
        "threshold_secs": threshold_secs,
    }


async def run_loop_heartbeat(
    state: Any,
    interval_secs: float = HEARTBEAT_INTERVAL_SECS,
) -> None:
    """Write ``state.loop_last_tick`` once per interval until cancelled.

    The coroutine awaits ``asyncio.sleep`` between ticks, so its progression is
    direct evidence that the gateway event loop is schedulable. The task
    handle lives at ``app.state.loop_heartbeat_task`` and is cancelled by
    ``GatewayServer.close()`` on shutdown.
    """
    state.loop_last_tick = time.time()
    while True:
        await asyncio.sleep(interval_secs)
        state.loop_last_tick = time.time()
