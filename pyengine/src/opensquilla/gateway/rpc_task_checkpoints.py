"""RPC handlers for long-task checkpoints (P2-11, phase 1).

* ``tasks.checkpoint.list``    — list interrupted (in-flight) checkpoints.
* ``tasks.checkpoint.abandon`` — mark an interrupted task abandoned.
* ``tasks.checkpoint.replay``  — mark an interrupted task for re-issue.

Phase 1 replay resets the checkpoint so the task can be re-issued; it does not
resume mid-turn. Side-effect idempotency and step-exact resumption are phase 2.
"""

from __future__ import annotations

from typing import Any

from opensquilla.engine.task_checkpoints import (
    CheckpointStore,
    default_checkpoint_store,
)
from opensquilla.gateway.rpc import RpcContext, get_dispatcher

_d = get_dispatcher()


def _store() -> CheckpointStore:
    return default_checkpoint_store()


def _task_id(params: dict | None) -> str:
    params = params or {}
    task_id = str(params.get("task_id") or params.get("taskId") or "").strip()
    if not task_id:
        raise ValueError("tasks.checkpoint.* requires a non-empty task_id")
    return task_id


@_d.method("tasks.checkpoint.list", scope="operator.read")
async def _handle_tasks_checkpoint_list(params: dict | None, ctx: RpcContext) -> dict[str, Any]:
    """Return checkpoints left in flight after a restart (running / queued_for_replay)."""

    interrupted = _store().list_interrupted()
    return {
        "interrupted": [c.to_dict() for c in interrupted],
        "count": len(interrupted),
        "phase": 1,
        "replay_semantics": "reset_and_reissue",
    }


@_d.method("tasks.checkpoint.abandon", scope="operator.write")
async def _handle_tasks_checkpoint_abandon(params: dict | None, ctx: RpcContext) -> dict[str, Any]:
    cp = _store().abandon(_task_id(params))
    if cp is None:
        return {"ok": False, "reason": "not_found"}
    return {"ok": True, "checkpoint": cp.to_dict()}


@_d.method("tasks.checkpoint.replay", scope="operator.write")
async def _handle_tasks_checkpoint_replay(params: dict | None, ctx: RpcContext) -> dict[str, Any]:
    cp = _store().mark_for_replay(_task_id(params))
    if cp is None:
        return {"ok": False, "reason": "not_found"}
    return {
        "ok": True,
        "checkpoint": cp.to_dict(),
        "replay_semantics": "reset_and_reissue",
        "note": "phase 1: re-issues the task from scratch; side-effect idempotency is phase 2",
    }


__all__ = []
