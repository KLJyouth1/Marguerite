"""Tests for the WS reconnect resync ring buffer (P2-10)."""

from __future__ import annotations

from opensquilla.gateway.event_resync import (
    EventResyncBuffer,
    gap_range,
    reset_event_resync_buffer,
    should_snapshot,
)


def _fill(n: int, *, channel: str = "sess-a") -> EventResyncBuffer:
    buf = EventResyncBuffer(capacity=16)
    for i in range(n):
        buf.record("session.event.task.update", {"i": i}, channel=channel)
    return buf


def test_should_snapshot_boundary() -> None:
    # Buffer holds frames 5..10. Client at last_seq==4 is exactly one behind
    # the oldest retained frame (5) -> recoverable by replay.
    assert should_snapshot(last_seq=4, first_buffered_seq=5) is False
    # Client at last_seq==3 has already lost frames 4 and 5 -> snapshot.
    assert should_snapshot(last_seq=3, first_buffered_seq=5) is True


def test_gap_range_modes() -> None:
    # Already caught up / ahead of head: nothing to replay.
    assert gap_range(last_seq=10, current_seq=10, first_buffered_seq=5) == (11, 10, "replay")
    # Fully inside buffer: replay.
    assert gap_range(last_seq=4, current_seq=10, first_buffered_seq=5) == (5, 10, "replay")
    # Evicted: snapshot.
    assert gap_range(last_seq=1, current_seq=10, first_buffered_seq=5) == (2, 10, "snapshot")


def test_replay_missing_frames_inside_ring() -> None:
    buf = _fill(10)  # seq 1..10
    out = buf.resync(last_seq=7)
    assert out["resync_mode"] == "replay"
    assert out["from_seq"] == 8
    assert out["to_seq"] == 10
    assert out["last_seq"] == 10
    seqs = [f["seq"] for f in out["events"]]
    assert seqs == [8, 9, 10]


def test_reconnect_exactly_at_head_is_empty_replay() -> None:
    buf = _fill(5)
    out = buf.resync(last_seq=5)
    assert out["resync_mode"] == "replay"
    assert out["events"] == []
    assert out["from_seq"] == 6 and out["to_seq"] == 5


def test_evicted_gap_aligns_to_snapshot_not_fabrication() -> None:
    buf = EventResyncBuffer(capacity=4)
    for i in range(8):  # evicts 1..4, ring holds 5..8
        buf.record("session.event.task.update", {"i": i}, channel="s")
    out = buf.resync(last_seq=1)  # far behind: 1..4 were evicted
    assert out["resync_mode"] == "snapshot"
    seqs = [f["seq"] for f in out["events"]]
    assert seqs == [5, 6, 7, 8]


def test_lossy_delta_frames_are_not_buffered() -> None:
    buf = EventResyncBuffer(capacity=16)
    buf.record("session.event.text_delta", {"chunk": "hi"}, channel="s")
    buf.record("session.event.run_heartbeat", {}, channel="s")
    buf.record("session.event.tool_use_delta", {}, channel="s")
    # Only a business frame consumes a seq slot.
    assert buf.current_seq == 0
    buf.record("session.event.task.update", {"i": 0}, channel="s")
    assert buf.current_seq == 1


def test_ring_eviction_drops_oldest() -> None:
    buf = EventResyncBuffer(capacity=3)
    for i in range(5):
        buf.record("session.event.task.update", {"i": i}, channel="s")
    out = buf.resync(last_seq=0)
    # capacity 3 -> only 3..5 retained; 1,2 evicted -> snapshot alignment.
    assert out["resync_mode"] == "snapshot"
    seqs = [f["seq"] for f in out["events"]]
    assert seqs == [3, 4, 5]


def test_channel_filtering() -> None:
    buf = EventResyncBuffer(capacity=16)
    buf.record("session.event.task.update", {"i": 1}, channel="sess-a")
    buf.record("session.event.task.update", {"i": 2}, channel="sess-b")
    buf.record("session.event.task.update", {"i": 3}, channel="sess-a")
    out = buf.resync(last_seq=0, channels=["sess-a"])
    seqs = [(f["seq"], f["channel"]) for f in out["events"]]
    assert seqs == [(1, "sess-a"), (3, "sess-a")]


def test_singleton_reset() -> None:
    # Exercise the reset hook used by tests; the singleton must not leak.
    reset_event_resync_buffer()
    from opensquilla.gateway.event_resync import get_event_resync_buffer

    buf = get_event_resync_buffer()
    assert buf.current_seq == 0
