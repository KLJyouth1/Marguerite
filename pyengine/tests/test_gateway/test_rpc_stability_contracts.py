"""RPC dispatch contract tests for the P2-10 / P2-11 stability methods.

These prove the methods are registered, dispatchable, and return the shape Rust
/ the frontend rely on. Pure buffer/store behaviour is covered under
tests/unit; here we exercise the real RPC surface.
"""

from __future__ import annotations

import pytest

from opensquilla.gateway.auth import Principal
from opensquilla.gateway.event_resync import (
    get_event_resync_buffer,
    reset_event_resync_buffer,
)
from opensquilla.gateway.rpc import RpcContext, get_dispatcher
from opensquilla.gateway.scopes import READ_SCOPE, WRITE_SCOPE


def _admin_ctx() -> RpcContext:
    return RpcContext(
        conn_id="t",
        principal=Principal(
            role="operator",
            scopes=frozenset({"operator.admin"}),
            is_owner=True,
            authenticated=True,
        ),
    )


@pytest.mark.asyncio
async def test_events_resync_replay_path() -> None:
    reset_event_resync_buffer()
    buf = get_event_resync_buffer()
    for i in range(3):
        buf.record("session.event.task.update", {"i": i}, channel="sess-a")

    res = await get_dispatcher().dispatch(
        "r1", "events.resync", {"last_seq": 1}, _admin_ctx()
    )
    assert res.error is None, res.error
    assert res.payload["resync_mode"] == "replay"
    assert [e["seq"] for e in res.payload["events"]] == [2, 3]
    assert res.payload["last_seq"] == 3
    assert res.payload["buffer_capacity"] > 0


@pytest.mark.asyncio
async def test_events_resync_snapshot_path_when_evicted() -> None:
    reset_event_resync_buffer()
    buf = get_event_resync_buffer()
    # Overflow a small ring: record well past the default capacity.
    for i in range(buf.capacity + 5):
        buf.record("session.event.task.update", {"i": i}, channel="sess-a")

    res = await get_dispatcher().dispatch(
        "r2", "events.resync", {"last_seq": 1}, _admin_ctx()
    )
    assert res.error is None, res.error
    # Far behind the evicted ring -> snapshot alignment, not fabricated replay.
    assert res.payload["resync_mode"] == "snapshot"
    assert res.payload["events"], "snapshot must carry the current window"
    reset_event_resync_buffer()


@pytest.mark.asyncio
async def test_tasks_checkpoint_list_abandon_replay_roundtrip(
    tmp_path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("OPENSQUILLA_STATE_DIR", str(tmp_path))
    dispatcher = get_dispatcher()

    # Seed an interrupted checkpoint directly through the store.
    from opensquilla.engine.task_checkpoints import (
        TaskCheckpoint,
        default_checkpoint_store,
    )

    store = default_checkpoint_store()
    store.record(
        TaskCheckpoint(
            task_id="task-x",
            session_id="s1",
            title="long build",
            status="running",
            completed_steps=[{"step": 1, "tool": "read", "summary": "done"}],
            side_effects=[{"kind": "file", "ref": "out.txt"}],
            total_steps=3,
        )
    )

    listed = await dispatcher.dispatch("r3", "tasks.checkpoint.list", {}, _admin_ctx())
    assert listed.error is None, listed.error
    assert listed.payload["count"] == 1
    assert listed.payload["interrupted"][0]["task_id"] == "task-x"
    assert listed.payload["phase"] == 1

    abandoned = await dispatcher.dispatch(
        "r4", "tasks.checkpoint.abandon", {"task_id": "task-x"}, _admin_ctx()
    )
    assert abandoned.error is None, abandoned.error
    assert abandoned.payload["ok"] is True
    assert abandoned.payload["checkpoint"]["status"] == "abandoned"

    after = await dispatcher.dispatch("r5", "tasks.checkpoint.list", {}, _admin_ctx())
    assert after.payload["count"] == 0


def test_new_methods_classified_in_scope_table() -> None:
    # Guard: these methods must be explicitly classified (no drift).
    from opensquilla.gateway.scopes import METHOD_SCOPES

    assert METHOD_SCOPES["events.resync"] == READ_SCOPE
    assert METHOD_SCOPES["tasks.checkpoint.list"] == READ_SCOPE
    assert METHOD_SCOPES["tasks.checkpoint.abandon"] == WRITE_SCOPE
    assert METHOD_SCOPES["tasks.checkpoint.replay"] == WRITE_SCOPE
