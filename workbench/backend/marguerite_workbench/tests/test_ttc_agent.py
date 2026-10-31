"""TTC + AgentLoop tests — offline, deterministic, key-free (repo invariant)."""

from __future__ import annotations

import json

import pytest

from marguerite_workbench.agentloop import (
    DEFAULT_MAX_REPAIRS,
    AgentLoop,
    plan_steps,
)
from marguerite_workbench.events import EventBus, SessionLog
from marguerite_workbench.ttc import (
    best_of_n,
    budget_for,
    estimate_difficulty,
    preset_for,
    run_strategy,
    self_consistency,
)


# ---------------------------------------------------------------------------
# TTC
# ---------------------------------------------------------------------------
def test_budget_table_mapping():
    assert budget_for("please prove this theorem about concurrency")["difficulty"] == "high"
    assert budget_for("explain the difference between a and b")["difficulty"] == "medium"
    assert budget_for("hi")["difficulty"] == "low"
    high = budget_for("optimize the distributed architecture")
    assert high["n_candidates"] >= 3 and high["max_rounds"] >= 8


def test_presets_align_with_policy_table_and_reject_unknown():
    assert preset_for("code")["temperature"] == 0.0          # greedy for determinism
    assert preset_for("creative")["temperature"] > preset_for("reasoning")["temperature"]
    with pytest.raises(ValueError):
        preset_for("nope")


@pytest.mark.usefixtures("_isolated_env")
def test_self_consistency_fallback_votes_from_kb(workspace_dir):
    from marguerite_workbench.kb import KnowledgeBase

    kb = KnowledgeBase(workspace_dir + "/kb.jsonl")
    kb.ingest("Marguerite gates every deliverable through Bloom quality levels.", "doc1")
    res = self_consistency("Bloom quality", n=4, kb=kb)
    d = res.to_dict()
    assert d["engine"] == "fallback", "without keys the engine must be honestly tagged"
    assert d["candidates"], "fallback must still produce real retrieval candidates"
    assert sum(d["votes"].values()) == 4
    assert 0.0 < d["agreement"] <= 1.0


@pytest.mark.usefixtures("_isolated_env")
def test_best_of_n_scores_rank_deterministically(workspace_dir):
    from marguerite_workbench.kb import KnowledgeBase

    kb = KnowledgeBase(workspace_dir + "/kb.jsonl")
    kb.ingest("alpha beta", "d1")
    res1 = best_of_n("alpha beta", n=3, kb=kb)
    res2 = best_of_n("alpha beta", n=3, kb=kb)
    assert res1.to_dict() == res2.to_dict(), "fallback path is fully deterministic"
    assert len(res1.scores) == 3 and res1.answer in res1.candidates


def test_run_strategy_dispatch_and_error():
    assert run_strategy("self_consistency", "x", n=1).strategy == "self_consistency"
    with pytest.raises(ValueError):
        run_strategy("tree_of_thoughts", "x")  # declared NOT IMPLEMENTED in the plan


# ---------------------------------------------------------------------------
# events
# ---------------------------------------------------------------------------
def test_eventbus_ring_and_subscriber_errors_isolated():
    bus = EventBus(replay_size=4)
    seen: list[dict] = []

    def boom(_event: dict) -> None:
        raise RuntimeError("subscriber blew up")

    bus.subscribe(seen.append)
    bus.subscribe(boom)
    for i in range(6):
        bus.publish("tick", {"i": i})
    recent = bus.recent(10)
    assert [e["payload"]["i"] for e in recent] == [2, 3, 4, 5], "ring keeps only the last 4"
    assert seen, "healthy subscriber still notified despite broken one"


def test_session_log_append_only(tmp_path):
    log = SessionLog(tmp_path, session_id="t1")
    log.append({"type": "a"})
    log.append({"type": "b"})
    entries = log.read_all()
    assert [e["type"] for e in entries] == ["a", "b"]
    raw = (tmp_path / "t1.jsonl").read_text(encoding="utf-8").strip().splitlines()
    assert len(raw) == 2  # append-only, no rewrite


def test_session_log_uses_one_handle_not_one_per_event(tmp_path):
    """W8: open once per log instead of once per event, without losing durability."""
    log = SessionLog(tmp_path, session_id="bulk")
    for i in range(50):
        log.append({"type": "tick", "i": i})

    assert log.appends == 50
    assert log.opens == 1, f"每事件一次 open 正是要修的（实测 opens={log.opens}）"
    # 每行仍然即时落盘：缓冲不能把「省 syscall」变成「丢最后几行」。
    raw = (tmp_path / "bulk.jsonl").read_text(encoding="utf-8").strip().splitlines()
    assert len(raw) == 50, "flush 必须逐行发生，崩溃后不得有丢失窗口"
    assert [json.loads(line)["i"] for line in raw] == list(range(50)), "顺序即写入顺序"
    log.close()


def test_session_log_reopens_after_close_and_still_never_rewrites(tmp_path):
    log = SessionLog(tmp_path, session_id="reopen")
    log.append({"type": "a"})
    log.close()
    log.append({"type": "b"})
    assert log.opens == 2, "close 之后再 append 应自动重开，而不是丢事件"
    assert [e["type"] for e in log.read_all()] == ["a", "b"]
    # 重开走的是 append 模式：前一行不得被截断或覆盖。
    raw = (tmp_path / "reopen.jsonl").read_text(encoding="utf-8").strip().splitlines()
    assert len(raw) == 2
    log.close()
    log.close()  # 幂等：重复 close 不得抛


# ---------------------------------------------------------------------------
# agent loop
# ---------------------------------------------------------------------------
@pytest.fixture()
def loop_ctx(workspace_dir):
    from marguerite_workbench.kb import KnowledgeBase
    from pathlib import Path

    ws = Path(workspace_dir)
    kb = KnowledgeBase(ws.parent / "data" / "kb.jsonl")
    kb.ingest("The workspace contains sample sources.", "readme")

    class _Mem:
        def query(self, layer, q, k=3):
            return [{"layer": layer, "content": f"memory hit for {q}"}]

    return {
        "workspace": ws,
        "kb": kb,
        "memory": _Mem(),
        "sessions_dir": ws.parent / "data" / "sessions",
    }


def test_plan_steps_intent_routing(loop_ctx):
    read_plan = plan_steps("open notes.txt and show it")
    assert read_plan[0]["tool"] == "list_dir"
    assert read_plan[1]["tool"] == "read_file"
    kb_plan = plan_steps("search knowledge for bloom")
    assert kb_plan[0]["tool"] == "kb_search"
    default_plan = plan_steps("do something else")
    assert default_plan[0]["tool"] == "list_dir"


def test_agent_run_completes_and_logs(loop_ctx):
    bus = EventBus()
    loop = AgentLoop(**loop_ctx, bus=bus, max_rounds=6)
    result = loop.run("search knowledge about sample sources")
    d = result.to_dict()
    assert d["status"] == "completed"
    states = [s["state"] for s in d["steps"]]
    assert "OBSERVE" in states, "successful tool steps terminate in OBSERVE"
    assert all(s["ok"] for s in d["steps"]), "a clean run has zero failed steps"
    transcript = json.loads((loop_ctx["sessions_dir"] / f"{result.session_id}.jsonl")
                            .read_text(encoding="utf-8").strip().splitlines()[-1])
    assert transcript["type"] == "run_finished"


def test_agent_loop_repairs_bad_path_then_succeeds(loop_ctx):
    loop = AgentLoop(**loop_ctx)
    result = loop.run("read missing_file.py please")
    d = result.to_dict()
    failures = [s for s in d["steps"] if s.get("error")]
    assert failures and all(s["state"] == "OBSERVE" for s in failures), "failed steps terminate in OBSERVE"
    repairs = [s for s in d["steps"] if s.get("is_repair")]
    assert repairs, "a failed step must trigger a corrective re-dispatch"
    recovered = [s for s in repairs if s["ok"]]
    assert recovered and any("entries" in (s.get("detail") or {}) for s in recovered), \
        "missing-file read degrades to a root listing observation"
    assert d["status"] == "completed", "repair recovery flips the run to completed"


def test_agent_loop_repair_budget_exhaustion_fails_loudly(loop_ctx):
    class BrokenKB:
        def search(self, q, k=3):
            raise RuntimeError("kb offline")

    ctx = dict(loop_ctx)
    ctx["kb"] = BrokenKB()
    loop = AgentLoop(**ctx, max_repairs=DEFAULT_MAX_REPAIRS)
    result = loop.run("search knowledge about anything")
    d = result.to_dict()
    assert d["status"] == "failed", "exhausted repairs must fail loudly, never silently"
    assert any(s.get("is_repair") for s in d["steps"])
