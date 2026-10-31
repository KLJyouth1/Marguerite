"""Agent Loop — the Codex-style think→act→observe→repair state machine.

Implements 支柱C of docs/UPGRADE_PLAN_V2.md:

    [task] → PLAN → ACT(tool) → OBSERVE ──ok──→ …next step… → FINALIZE
                        ↑                        │
                        └──── REPAIR (≤max_repairs) ←── failure

Hard guarantees:
* ``max_rounds`` bounds total steps — no unbounded loops;
* a failing tool enters REPAIR with a corrective re-dispatch, at most
  ``max_repairs`` times, after which the run fails *loudly* (never silently);
* every transition is published on the EventBus and appended to the session
  JSONL (append-only, replayable).

Tools are a **table-driven registry**: each entry declares its argument
normaliser and validator, so adding a capability is one dict entry (anti R1).
All file-touching tools funnel through ``security.safe_resolve``.
"""

from __future__ import annotations

import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Callable

from marguerite_workbench.events import EventBus, SessionLog, now_ms
from marguerite_workbench.security import safe_resolve

STATE_PLAN = "PLAN"
STATE_ACT = "ACT"
STATE_OBSERVE = "OBSERVE"
STATE_REPAIR = "REPAIR"
STATE_FINALIZE = "FINALIZE"

DEFAULT_MAX_ROUNDS = 8
DEFAULT_MAX_REPAIRS = 3


# ---------------------------------------------------------------------------
# Tool registry — real capabilities only, sandboxed paths always.
# ---------------------------------------------------------------------------
def _tool_read_file(ctx: dict, args: dict) -> dict:
    rel = str(args.get("path", ""))
    resolved = safe_resolve(ctx["workspace"], rel)
    text = resolved.read_text(encoding="utf-8", errors="replace")
    return {"path": rel, "bytes": len(text), "preview": text[:400]}


def _tool_list_dir(ctx: dict, args: dict) -> dict:
    rel = str(args.get("path", "."))
    root = safe_resolve(ctx["workspace"], rel)
    entries = sorted(
        (p.name + ("/" if p.is_dir() else "") for p in root.iterdir()),
    )[:100]
    return {"path": rel, "entries": entries}


def _tool_kb_search(ctx: dict, args: dict) -> dict:
    hits = ctx["kb"].search(str(args.get("q", "")), int(args.get("k", 3)))
    return {"hits": [{"source": h["source"], "score": round(h["score"], 4)} for h in hits]}


def _tool_memory_query(ctx: dict, args: dict) -> dict:
    items = ctx["memory"].query(
        str(args.get("layer", "L3")), str(args.get("q", "")), int(args.get("k", 3))
    )
    return {"items": [{"layer": i["layer"], "content": i["content"][:160]} for i in items]}


@dataclass(frozen=True)
class ToolSpec:
    fn: Callable[[dict, dict], dict]
    required: tuple[str, ...]


TOOL_REGISTRY: dict[str, ToolSpec] = {
    # declared once; the loop and the planner both read this single table
    "read_file": ToolSpec(_tool_read_file, ("path",)),
    "list_dir": ToolSpec(_tool_list_dir, ()),
    "kb_search": ToolSpec(_tool_kb_search, ("q",)),
    "memory_query": ToolSpec(_tool_memory_query, ("q",)),
}

# Deterministic keyword→plan mapping: the offline planner (repo invariant #3:
# 离线可运行). Each intent lists ordered (tool, args) steps.
_PLANNERS: tuple[tuple[str, tuple[tuple[str, dict], ...]], ...] = (
    (
        ("read", "open", "show", "读"),
        (("list_dir", {"path": "."}), ("read_file", {})),  # path filled from task below
    ),
    (
        ("find", "search", "knowledge", "检索", "查"),
        (("kb_search", {}),),
    ),
    (
        ("remember", "memory", "回忆", "记忆"),
        (("memory_query", {}),),
    ),
)
_DEFAULT_PLAN: tuple[tuple[str, dict], ...] = (("list_dir", {"path": "."}), ("kb_search", {}))


def plan_steps(task: str) -> list[dict]:
    """Derive an ordered, dependency-honest plan from lexical intent signals."""
    low = task.lower()
    steps: list[dict] = []
    chosen: tuple[tuple[str, dict], ...] | None = None
    for keywords, plan in _PLANNERS:
        if any(k in low for k in keywords):
            chosen = plan
            break
    plan = chosen or _DEFAULT_PLAN
    for tool, template in plan:
        args = dict(template)
        if tool == "read_file":
            # First whitespace-token that looks like a path in the workspace.
            candidate = next((w for w in task.split() if "." in w and "/" not in w[:1]), "")
            args["path"] = candidate or "."
        if tool == "kb_search" and not args.get("q"):
            args["q"] = task
        if tool == "memory_query" and not args.get("q"):
            args["q"] = task
        steps.append({"tool": tool, "args": args})
    return steps


# ---------------------------------------------------------------------------
# Records & result
# ---------------------------------------------------------------------------
@dataclass
class StepRecord:
    index: int
    state: str
    tool: str | None
    args: dict = field(default_factory=dict)
    ok: bool = False
    detail: dict = field(default_factory=dict)
    error: str | None = None
    duration_ms: int = 0
    """``state`` is the closing phase of the record (always OBSERVE); ``is_repair``
    distinguishes corrective re-dispatches from first attempts; ``repaired_index``
    links a repair back to the failed attempt it recovers."""
    is_repair: bool = False
    repaired_index: int | None = None


@dataclass
class AgentRunResult:
    task: str
    status: str                 # completed | failed
    answer: str
    steps: list[StepRecord] = field(default_factory=list)
    session_id: str = ""
    events_file: str = ""
    rounds_used: int = 0

    def to_dict(self) -> dict:
        d = asdict(self)
        d["steps"] = [asdict(s) for s in self.steps]
        return d


class AgentLoop:
    def __init__(
        self,
        workspace: Path,
        kb,
        memory,
        sessions_dir: Path,
        *,
        bus: EventBus | None = None,
        max_rounds: int = DEFAULT_MAX_ROUNDS,
        max_repairs: int = DEFAULT_MAX_REPAIRS,
    ) -> None:
        if max_rounds < 1 or max_repairs < 0:
            raise ValueError("max_rounds >= 1 and max_repairs >= 0 are required")
        self.ctx = {"workspace": Path(workspace), "kb": kb, "memory": memory}
        self.log = SessionLog(sessions_dir)
        self.bus = bus or EventBus()
        self.max_rounds = max_rounds
        self.max_repairs = max_repairs

    # ------------------------------------------------------------------ run
    def run(self, task: str) -> AgentRunResult:
        if not task.strip():
            raise ValueError("task must not be empty")
        started = now_ms()
        result = AgentRunResult(task=task, status="failed", answer="", session_id=self.log.session_id,
                                events_file=str(self.log.path))

        def emit(state: str, payload: dict) -> None:
            event = self.bus.publish(state, {"session": self.log.session_id, **payload})
            self.log.append(event)

        emit("run_started", {"task": task, "plan_tools": [s["tool"] for s in plan_steps(task)]})

        steps: list[dict] = plan_steps(task)
        repairs_used = 0
        details: list[dict] = []
        idx = 0
        for round_no, step_spec in enumerate(steps, start=1):
            if round_no > self.max_rounds:
                emit("round_budget_exhausted", {"max_rounds": self.max_rounds})
                break
            record = StepRecord(index=idx, state=STATE_ACT, tool=step_spec["tool"], args=step_spec["args"])
            idx += 1
            t0 = time.perf_counter()
            emit(STATE_ACT, {"step": record.index, "tool": record.tool, "args": record.args})
            try:
                spec = TOOL_REGISTRY[record.tool]
                missing = [k for k in spec.required if k not in record.args]
                if missing:
                    raise ValueError(f"missing required args {missing} for tool {record.tool!r}")
                detail = spec.fn(self.ctx, record.args)
                record.ok, record.detail = True, detail
                record.duration_ms = int((time.perf_counter() - t0) * 1000)
                record.state = STATE_OBSERVE
                emit(STATE_OBSERVE, {"step": record.index, "ok": True})
                details.append(detail)
                result.steps.append(record)
            except Exception as exc:  # noqa: BLE001 — converted into the repair protocol
                record.state = STATE_OBSERVE  # the failure *was* observed
                record.error = f"{type(exc).__name__}: {exc}"
                record.duration_ms = int((time.perf_counter() - t0) * 1000)
                result.steps.append(record)
                emit(STATE_OBSERVE, {"step": record.index, "ok": False, "error": record.error})
                if repairs_used >= self.max_repairs:
                    emit(STATE_FINALIZE, {"status": "failed", "reason": "repair budget exhausted"})
                    result.rounds_used = round_no
                    self._finalize(result, started)
                    return result
                repairs_used += 1
                repair_tool, repair_args = self._plan_repair(record.tool, record.args, record.error or "")
                repair = StepRecord(index=idx, state=STATE_REPAIR, tool=repair_tool, args=repair_args,
                                    is_repair=True, repaired_index=record.index)
                idx += 1
                rt0 = time.perf_counter()
                emit(STATE_REPAIR, {"step": repair.index, "attempt": repairs_used,
                                    "tool": repair.tool, "args": repair.args})
                try:
                    spec = TOOL_REGISTRY[repair.tool]
                    detail = spec.fn(self.ctx, repair.args)
                    repair.state = STATE_OBSERVE
                    repair.ok, repair.detail = True, detail
                    repair.duration_ms = int((time.perf_counter() - rt0) * 1000)
                    details.append(detail)
                    emit(STATE_OBSERVE, {"step": repair.index, "ok": True, "repaired": True})
                    result.steps.append(repair)
                except Exception as exc2:  # noqa: BLE001
                    repair.state = STATE_OBSERVE
                    repair.error = f"{type(exc2).__name__}: {exc2}"
                    repair.duration_ms = int((time.perf_counter() - rt0) * 1000)
                    result.steps.append(repair)

        result.answer = self._summarize(task, details)
        # A run is *completed* only if every failure was recovered by a successful
        # repair linked to it; unresolved failures fail loudly.
        resolved_indices = {
            r.repaired_index for r in result.steps if r.is_repair and r.ok
        }
        unresolved: list[StepRecord] = [
            rec for rec in result.steps if not rec.ok and rec.index not in resolved_indices
        ]
        result.status = "completed" if not unresolved else "failed"
        if unresolved:
            first_error = unresolved[0].error or "unknown error"
            result.answer = (
                f"[unresolved] {len(unresolved)} step(s) failed; first error: {first_error}\n"
                f"{result.answer}"
            )
        result.rounds_used = min(len(steps), self.max_rounds)
        emit(STATE_FINALIZE, {"status": result.status, "steps": len(result.steps),
                              "unresolved": len(unresolved)})
        self._finalize(result, started)
        return result

    @staticmethod
    def _plan_repair(tool: str, args: dict, error: str) -> tuple[str, dict]:
        """Table-driven corrective re-dispatch: (repaired_tool, args).

        Policy: a *missing file* read degrades to a root directory listing
        (still a real observation); empty/invalid arguments fall back to safe
        defaults.  Retrying the identical failing call is never the answer.
        """
        lower = error.lower()
        if tool == "read_file":
            if "no such file" in lower or "filenotfound" in lower or "directory" in lower:
                return "list_dir", {"path": "."}
            path = str(args.get("path", "")).strip()
            return tool, {**args, "path": path or "."}
        if tool == "list_dir":
            path = str(args.get("path", ".")).strip()
            return tool, {**args, "path": path or "."}
        if tool in ("kb_search", "memory_query") and not str(args.get("q", "")).strip():
            return tool, {"q": "workspace overview"}
        return tool, dict(args)

    @staticmethod
    def _summarize(task: str, details: list[dict]) -> str:
        parts = [f"task: {task}", f"observations: {len(details)}"]
        for i, d in enumerate(details, 1):
            if "entries" in d:
                parts.append(f"{i}. listed {d['path']}: {', '.join(d['entries'][:8])}")
            elif "preview" in d:
                parts.append(f"{i}. read {d['path']} ({d['bytes']}B): {d['preview'][:120]}")
            elif "hits" in d:
                parts.append(f"{i}. kb hits: {d['hits'][:3]}")
            elif "items" in d:
                parts.append(f"{i}. memories: {len(d['items'])} item(s)")
        return "\n".join(parts)

    def _finalize(self, result: AgentRunResult, started: int) -> None:
        self.log.append({
            "type": "run_finished",
            "ts": now_ms(),
            "seq": 0,
            "payload": {
                "session": self.log.session_id,
                "status": result.status,
                "duration_ms": now_ms() - started,
                "steps": len(result.steps),
            },
        })
        # W8: the transcript handle stays open across a run (one open, not one
        # per event); release it at the run boundary so long-lived servers do
        # not accumulate fds.  A late append simply reopens — nothing is lost.
        self.log.close()
