"""Long-task turn-level checkpoint store (P2-11, phase 1).

Phase-1 scope (explicitly NOT done here):
* step-exact resumption (re-running only the *remaining* steps);
* an idempotency ledger for already-applied side effects;
* distributed/cross-process locking.

What phase 1 does:
* persist one checkpoint per long task to a durable directory
  (``<state>/checkpoints`` by default, overridable by ``OPENSQUILLA_STATE_DIR``);
* on process restart, surface tasks whose checkpoint is still ``running``
  (i.e. they were interrupted mid-turn);
* let an operator list / abandon / mark-for-replay those interrupted tasks.

Replay in phase 1 means "reset the checkpoint so the task can be re-issued".
It does NOT resume mid-way — side-effect idempotency is a documented phase-2 item.
"""

from __future__ import annotations

import json
import os
import re
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any

# Checkpoint statuses that count as "still in flight" after a restart.
INTERRUPTED_STATUSES = frozenset({"running", "queued_for_replay"})


def _epoch_ms() -> int:
    return int(time.time() * 1000)


_SAFE = re.compile(r"[^A-Za-z0-9_.-]")


def _safe_name(task_id: str) -> str:
    raw = str(task_id)
    # Reject anything that could traverse the checkpoint directory, BEFORE
    # sanitization (sanitizing "../escape" into ".._escape" would hide a bad id).
    if (
        not raw
        or raw in {".", ".."}
        or "/" in raw
        or "\\" in raw
        or "\x00" in raw
        or raw.startswith(".")
    ):
        raise ValueError(f"unsafe task_id: {raw!r}")
    cleaned = _SAFE.sub("_", raw)
    if cleaned in {"", ".", ".."}:
        raise ValueError(f"unsafe task_id: {raw!r}")
    return cleaned


@dataclass(frozen=True)
class TaskCheckpoint:
    task_id: str
    session_id: str
    title: str
    status: str = "running"
    completed_steps: list[dict[str, Any]] = field(default_factory=list)
    side_effects: list[dict[str, Any]] = field(default_factory=list)
    total_steps: int = 0
    updated_at: int = field(default_factory=_epoch_ms)

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)

    @classmethod
    def from_dict(cls, raw: dict[str, Any]) -> "TaskCheckpoint":
        known = {f for f in cls.__dataclass_fields__}  # type: ignore[attr-defined]
        clean = {k: v for k, v in raw.items() if k in known}
        return cls(**clean)


class CheckpointStore:
    """File-backed checkpoint store. One JSON file per task.

    Writes are atomic (tmp + ``os.replace``) so a crash mid-write cannot leave a
    half-written checkpoint behind.
    """

    def __init__(self, root: str | Path) -> None:
        self.root = Path(root)
        self.root.mkdir(parents=True, exist_ok=True)

    def _path(self, task_id: str) -> Path:
        return self.root / f"{_safe_name(task_id)}.json"

    def record(self, checkpoint: TaskCheckpoint) -> TaskCheckpoint:
        """Upsert a checkpoint atomically."""

        checkpoint = TaskCheckpoint(
            **{**checkpoint.to_dict(), "updated_at": _epoch_ms()}
        )
        target = self._path(checkpoint.task_id)
        tmp = target.with_suffix(".json.tmp")
        payload = json.dumps(checkpoint.to_dict(), ensure_ascii=False, indent=2)
        with open(tmp, "w", encoding="utf-8") as fh:
            fh.write(payload)
        os.replace(tmp, target)
        return checkpoint

    def get(self, task_id: str) -> TaskCheckpoint | None:
        path = self._path(task_id)
        if not path.exists():
            return None
        try:
            with open(path, encoding="utf-8") as fh:
                return TaskCheckpoint.from_dict(json.load(fh))
        except (OSError, json.JSONDecodeError):
            # A corrupt checkpoint must not strand recovery; treat as absent.
            return None

    def list_all(self) -> list[TaskCheckpoint]:
        out: list[TaskCheckpoint] = []
        for path in sorted(self.root.glob("*.json")):
            try:
                with open(path, encoding="utf-8") as fh:
                    out.append(TaskCheckpoint.from_dict(json.load(fh)))
            except (OSError, json.JSONDecodeError):
                continue
        return out

    def list_interrupted(self) -> list[TaskCheckpoint]:
        """Checkpoints that were in flight when the process restarted."""

        return [c for c in self.list_all() if c.status in INTERRUPTED_STATUSES]

    def abandon(self, task_id: str) -> TaskCheckpoint | None:
        existing = self.get(task_id)
        if existing is None:
            return None
        return self.record(
            TaskCheckpoint(
                **{**existing.to_dict(), "status": "abandoned"}
            )
        )

    def mark_for_replay(self, task_id: str) -> TaskCheckpoint | None:
        existing = self.get(task_id)
        if existing is None:
            return None
        # Phase 1: replay = re-issue the whole task from scratch. We keep the
        # completed-step history for audit but reset the in-flight status.
        return self.record(
            TaskCheckpoint(
                **{**existing.to_dict(), "status": "queued_for_replay"}
            )
        )

    def finalize(self, task_id: str, status: str = "succeeded") -> TaskCheckpoint | None:
        existing = self.get(task_id)
        if existing is None:
            return None
        return self.record(
            TaskCheckpoint(
                **{**existing.to_dict(), "status": status}
            )
        )


def default_checkpoint_root() -> Path:
    """Default durable checkpoint directory under the OpenSquilla state root."""

    from opensquilla.paths import state_dir

    return state_dir("checkpoints")


def default_checkpoint_store() -> CheckpointStore:
    return CheckpointStore(default_checkpoint_root())


__all__ = [
    "CheckpointStore",
    "INTERRUPTED_STATUSES",
    "TaskCheckpoint",
    "default_checkpoint_root",
    "default_checkpoint_store",
]
