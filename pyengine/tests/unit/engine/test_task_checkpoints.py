"""Tests for the long-task checkpoint store (P2-11, phase 1)."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from opensquilla.engine.task_checkpoints import (
    CheckpointStore,
    TaskCheckpoint,
)


def _sample(task_id: str = "task-1", status: str = "running") -> TaskCheckpoint:
    return TaskCheckpoint(
        task_id=task_id,
        session_id="sess-1",
        title="build report",
        status=status,
        completed_steps=[{"step": 1, "tool": "read_file", "summary": "read input"}],
        side_effects=[{"kind": "file", "ref": "out/report.txt"}],
        total_steps=3,
    )


def test_record_and_roundtrip(tmp_path: Path) -> None:
    store = CheckpointStore(tmp_path)
    cp = _sample()
    store.record(cp)
    loaded = store.get("task-1")
    assert loaded is not None
    assert loaded.title == "build report"
    assert loaded.completed_steps == cp.completed_steps
    assert loaded.side_effects == cp.side_effects
    assert loaded.total_steps == 3


def test_missing_checkpoint_returns_none(tmp_path: Path) -> None:
    store = CheckpointStore(tmp_path)
    assert store.get("nope") is None


def test_list_interrupted_identifies_restarted_tasks(tmp_path: Path) -> None:
    store = CheckpointStore(tmp_path)
    store.record(_sample("task-running", status="running"))
    store.record(_sample("task-replay", status="queued_for_replay"))
    store.record(_sample("task-done", status="succeeded"))
    store.record(_sample("task-abandoned", status="abandoned"))

    interrupted = [c.task_id for c in store.list_interrupted()]
    assert sorted(interrupted) == ["task-replay", "task-running"]


def test_abandon_marks_terminal(tmp_path: Path) -> None:
    store = CheckpointStore(tmp_path)
    store.record(_sample("task-1"))
    out = store.abandon("task-1")
    assert out is not None and out.status == "abandoned"
    assert [c.task_id for c in store.list_interrupted()] == []
    assert store.abandon("missing") is None


def test_replay_keeps_history_and_sets_flag(tmp_path: Path) -> None:
    store = CheckpointStore(tmp_path)
    cp = _sample("task-1")
    store.record(cp)
    out = store.mark_for_replay("task-1")
    assert out is not None
    assert out.status == "queued_for_replay"
    # History preserved for audit.
    assert out.completed_steps == cp.completed_steps


def test_atomic_write_leaves_no_tmp(tmp_path: Path) -> None:
    store = CheckpointStore(tmp_path)
    store.record(_sample("task-1"))
    leftovers = list(tmp_path.glob("*.tmp"))
    assert leftovers == []


def test_corrupt_checkpoint_is_treated_as_absent(tmp_path: Path) -> None:
    store = CheckpointStore(tmp_path)
    (tmp_path / "task-bad.json").write_text("{not json", encoding="utf-8")
    assert store.get("task-bad") is None
    assert store.list_all() == []


def test_unsafe_task_id_is_rejected(tmp_path: Path) -> None:
    store = CheckpointStore(tmp_path)
    with pytest.raises(ValueError):
        store.get("../escape")
    with pytest.raises(ValueError):
        store.record(_sample("../escape"))


def test_updated_at_is_refreshed(tmp_path: Path) -> None:
    store = CheckpointStore(tmp_path)
    first = store.record(_sample("task-1"))
    second = store.record(_sample("task-1"))
    assert second.updated_at >= first.updated_at


def test_default_root_honors_state_dir_env(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("OPENSQUILLA_STATE_DIR", str(tmp_path))
    from opensquilla.engine.task_checkpoints import default_checkpoint_root

    root = default_checkpoint_root()
    # ``state_dir`` already appends the "state" segment under the env root.
    assert root == Path(tmp_path) / "state" / "checkpoints"
