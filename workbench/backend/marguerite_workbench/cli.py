"""Marguerite File Intelligence Workbench — command line interface.

Invoked as ``python -m marguerite_workbench <command> <verb> [options]`` (or via
the ``marguerite_workbench.cli:main`` entry point).  The CLI talks to the *same*
engine objects as the FastAPI app (:class:`marguerite_workbench.api.Workbench`),
so every result is the real engine output — no HTTP hop required, no mocks.

Output contract (mirrors the Marguerite core CLI invariants)
------------------------------------------------------------
* ``--json``  → single JSON object ``{"ok": bool, "data": ..., "meta": {...}}``
  on stdout; ``meta`` always carries ``cmd`` and ``duration_ms``.
* text mode   → human-readable rendering with ASCII status markers
  ``[OK] / [WARN] / [FAIL]`` — never emoji.
* ``NO_COLOR`` is honoured: colour escapes are emitted only on a TTY without it.
* errors      → human/JSON message on **stderr** + non-zero exit code:

    0  success
    1  runtime failure (engine error, failed check)
    2  usage error (unknown command / bad arguments)
    3  not found (unknown file id / expert role)
    4  validation or safety rejection (path escape, bad payload, …)

Dispatch is table-driven (``VERBS`` registries) per Bloom R1 — adding a verb
means registering one handler pair, not growing an if-ladder.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import sys
import time
from pathlib import Path
from typing import Any, Callable, Optional

from marguerite_workbench.models import (
    ExpertAnalysis,
    FileCard,
    HarnessResult,
    LayerStatus,
    MemoryQueryResult,
)

# ---------------------------------------------------------------------------
# Named constants (anti R5)
# ---------------------------------------------------------------------------
EXIT_OK = 0
EXIT_RUNTIME = 1
EXIT_USAGE = 2
EXIT_NOT_FOUND = 3
EXIT_REJECTED = 4

ENV_NO_COLOR = "NO_COLOR"
DEFAULT_HOST = "127.0.0.1"
DEFAULT_PORT = 8011
DOCTOR_MIN_PYTHON = (3, 10)
QUERY_K_DEFAULT = 5

_MARKER_OK = "[OK]"
_MARKER_WARN = "[WARN]"
_MARKER_FAIL = "[FAIL]"

_ANSI_GREEN = "\033[32m"
_ANSI_YELLOW = "\033[33m"
_ANSI_RED = "\033[31m"
_ANSI_DIM = "\033[2m"
_ANSI_RESET = "\033[0m"


def _color_enabled() -> bool:
    """Colour only on a real TTY and only when NO_COLOR is unset/empty."""
    return sys.stdout.isatty() and not os.environ.get(ENV_NO_COLOR)


def _paint(text: str, code: str) -> str:
    if not _color_enabled():
        return text
    return f"{code}{text}{_ANSI_RESET}"


class CliError(Exception):
    """Raised by handlers to produce a clean stderr message + exit code."""

    def __init__(self, message: str, exit_code: int = EXIT_RUNTIME) -> None:
        super().__init__(message)
        self.exit_code = exit_code


class Session:
    """One invocation's shared state (engines + output mode + meta)."""

    def __init__(self, as_json: bool) -> None:
        self.as_json = as_json
        self._workbench: Any = None
        self.started = time.perf_counter()

    @property
    def workbench(self) -> Any:
        # Imported lazily (and from the framework-free engine module) so
        # `--help` paths stay fast and the CLI never pays for FastAPI unless
        # it actually serves HTTP.
        if self._workbench is None:
            try:
                from marguerite_workbench.engine import Workbench
                from marguerite_workbench import config

                self._workbench = Workbench(config.workspace_root(), config.data_dir())
            except Exception as exc:  # pragma: no cover - environment specific
                raise CliError(f"failed to initialise workbench engines: {exc}") from exc
        return self._workbench


# ---------------------------------------------------------------------------
# Output helpers
# ---------------------------------------------------------------------------
def emit_json(session: Session, data: Any) -> None:
    payload = {
        "ok": True,
        "data": data,
        "meta": {
            "cmd": " ".join(sys.argv[1:]),
            "duration_ms": int((time.perf_counter() - session.started) * 1000),
        },
    }
    json.dump(payload, sys.stdout, ensure_ascii=False, indent=2, default=str)
    sys.stdout.write("\n")


def _pydantic(obj: Any) -> Any:
    return obj.model_dump(exclude_none=False) if hasattr(obj, "model_dump") else obj


def done(session: Session, data: Any, render_text: Optional[Callable[[], None]] = None) -> None:
    """Emit ``data`` (JSON mode) or run the text renderer, then exit OK."""
    if session.as_json:
        emit_json(session, [_pydantic(d) for d in data] if isinstance(data, list) else _pydantic(data))
        return
    if render_text is not None:
        render_text()
    else:  # graceful fallback: pretty-print the raw structure
        json.dump(_pydantic(data), sys.stdout, ensure_ascii=False, indent=2, default=str)
        sys.stdout.write("\n")


def _marker(kind: str) -> str:
    paint_map = {
        "ok": (_MARKER_OK, _ANSI_GREEN),
        "warn": (_MARKER_WARN, _ANSI_YELLOW),
        "fail": (_MARKER_FAIL, _ANSI_RED),
    }
    text, color = paint_map[kind]
    return _paint(text, color)


# ---------------------------------------------------------------------------
# Text renderers (ASCII-only decorations, no emoji)
# ---------------------------------------------------------------------------
def _rule(char: str = "-", width: int = 62) -> str:
    return char * width


def _render_file_card(card: FileCard) -> None:
    print(_paint("file", _ANSI_DIM), card.path)
    print(f"  id       {card.file_id}")
    print(f"  language {card.language or '-'}")
    print(f"  size     {card.size_bytes} bytes")
    print(f"  sha256   {card.sha256[:16]}…")
    print(f"  sections {len(card.sections)}")
    for s in card.sections:
        state = "collapsed" if s.collapsed else "open"
        title = s.title or s.id
        print(f"    {s.id:<14} L{s.start_line:<5}-{s.end_line:<5} {s.kind:<9} {state:<9} {title}")


def _strip_html(html: str) -> str:
    """De-emphasise pygments HTML into readable plain text for terminals."""
    import html as html_mod
    import re

    without_tags = re.sub(r"<[^>]+>", "", html)
    return html_mod.unescape(without_tags)


def _print_kv(rows: list[tuple[str, Any]]) -> None:
    width = max(len(k) for k, _ in rows)
    for key, value in rows:
        print(f"  {_paint(key.ljust(width), _ANSI_DIM)}  {value}")


def _render_analysis(a: ExpertAnalysis) -> None:
    verdict_paint = {
        "pass": _ANSI_GREEN,
        "conditional": _ANSI_YELLOW,
        "fail": _ANSI_RED,
    }.get(a.verdict, "")
    header = f"{a.role}: verdict={a.verdict} confidence={a.confidence:.2f}"
    print(_marker("ok"), _paint(header, verdict_paint))
    if a.findings:
        print("findings:")
        for f in a.findings:
            print(f"  - {f}")
    if a.recommendations:
        print("recommendations:")
        for r in a.recommendations:
            print(f"  - {r}")


def _render_harness_result(r: HarnessResult) -> None:
    kind = "ok" if r.status == "ok" else ("warn" if r.status == "partial" else "fail")
    print(_marker(kind), f"adapter={r.adapter} status={r.status} duration={r.duration_ms}ms")
    if r.steps:
        print("steps:")
        for i, step in enumerate(r.steps, 1):
            print(f"  {i:>2}. {step}")
    for d in r.diffs:
        print(_rule())
        print(d)


def _render_layers(layers: list[LayerStatus]) -> None:
    print(f"{'layer':<6} {'name':<22} {'count':>7} {'capacity':>10}")
    print(_rule())
    for l in layers:
        print(f"{l.layer:<6} {l.name:<22} {l.count:>7} {l.capacity:>10}")


def _render_memory_hits(result: MemoryQueryResult) -> None:
    if not result.items:
        print("(no matching memories)")
        return
    for item in result.items:
        created = item.created_at or "-"
        refs = ",".join(item.refs) if item.refs else "-"
        print(f"{_paint(item.layer, _ANSI_DIM)} {created} [{item.kind}] refs=({refs})")
        print(f"  {item.content}")


# ---------------------------------------------------------------------------
# Command handlers — files
# ---------------------------------------------------------------------------
def cmd_files_load(s: Session, ns: argparse.Namespace) -> None:
    card = s.workbench.files.load(ns.path)
    done(
        s,
        card,
        lambda: (
            _render_file_card(card),
            print(_marker("ok"), f"loaded {card.path} ({len(card.sections)} sections)"),
        ),
    )


def cmd_files_display(s: Session, ns: argparse.Namespace) -> None:
    try:
        result = s.workbench.files.display(ns.file_id)
    except KeyError as exc:
        raise CliError(str(exc), EXIT_NOT_FOUND) from exc
    done(
        s,
        result,
        lambda: (
            print(_paint(f"language: {result.language}", _ANSI_DIM)),
            print(_rule()),
            print(_strip_html(result.html).rstrip()),
        ),
    )


def cmd_files_modify(s: Session, ns: argparse.Namespace) -> None:
    try:
        result = s.workbench.files.modify(ns.file_id, ns.instruction)
    except KeyError as exc:
        raise CliError(str(exc), EXIT_NOT_FOUND) from exc
    done(
        s,
        result,
        lambda: (
            print(_marker("ok" if result.applied else "warn"),
                  f"applied={result.applied} backup={result.backup_ref or '-'}"),
            print(result.diff or "(no diff)"),
        ),
    )


def cmd_files_collapse(s: Session, ns: argparse.Namespace) -> None:
    try:
        card = s.workbench.files.collapse(ns.file_id, ns.section)
    except KeyError as exc:
        raise CliError(str(exc), EXIT_NOT_FOUND) from exc
    def render() -> None:
        _render_file_card(card)
        if ns.section:
            print(_marker("ok"), f"collapsed {len(ns.section)} named section(s)")
        else:
            print(_marker("ok"), "auto-collapsed sections above the length threshold")

    done(s, card, render)


def cmd_files_ask(s: Session, ns: argparse.Namespace) -> None:
    try:
        result = s.workbench.files.ask(ns.file_id, ns.question)
    except KeyError as exc:
        raise CliError(str(exc), EXIT_NOT_FOUND) from exc

    def render() -> None:
        print(result.answer)
        print()
        _print_citations(result.citations)
        print(_marker("ok"), f"{len(result.citations)} citation(s)")

    done(s, result, render)


def _print_citations(citations: list[dict]) -> None:
    """Print citations as an indented ASCII list."""
    for c in citations:
        source = c.get("source", "-")
        line = c.get("line", "?")
        print(_paint(f"  ^ {source}:{line}", _ANSI_DIM))
        text = c.get("text", "")
        if text:
            print(f"      {text}")


def cmd_files_interact(s: Session, ns: argparse.Namespace) -> None:
    try:
        result = s.workbench.files.interact(ns.file_id, ns.message, ns.session)
    except KeyError as exc:
        raise CliError(str(exc), EXIT_NOT_FOUND) from exc
    done(
        s,
        result,
        lambda: (
            print(result.reply),
            print(),
            print(_marker("ok"), f"session={result.session_id}"),
        ),
    )


# ---------------------------------------------------------------------------
# memory
# ---------------------------------------------------------------------------
def cmd_memory_layers(s: Session, ns: argparse.Namespace) -> None:
    layers = [LayerStatus(**st) for st in s.workbench.memory.layers_status()]
    done(s, layers, lambda: _render_layers(layers))


def cmd_memory_store(s: Session, ns: argparse.Namespace) -> None:
    try:
        stored = s.workbench.memory.store(
            layer=ns.layer.upper(), kind=ns.kind, content=ns.content, refs=ns.ref or None,
        )
    except ValueError as exc:
        raise CliError(str(exc), EXIT_REJECTED) from exc
    done(
        s,
        stored,
        lambda: print(_marker("ok"), f"stored {stored['id']} in {stored['layer']}"),
    )


def cmd_memory_query(s: Session, ns: argparse.Namespace) -> None:
    try:
        hits = s.workbench.memory.query(ns.layer.upper(), ns.q, ns.k)
    except ValueError as exc:
        raise CliError(str(exc), EXIT_REJECTED) from exc
    result = MemoryQueryResult(items=[{**h, "embedding": None} for h in hits])
    done(s, result, lambda: _render_memory_hits(result))


def cmd_memory_sync(s: Session, ns: argparse.Namespace) -> None:
    result = asyncio.run(s.workbench.memory.sync_to_core())
    detail = result.get("detail") or result.get("target") or ""
    done(
        s,
        result,
        lambda: print(_marker("ok"), f"sync status={result['status']} synced={result['synced']} {detail}"),
    )


# ---------------------------------------------------------------------------
# kb
# ---------------------------------------------------------------------------
def cmd_kb_ingest(s: Session, ns: argparse.Namespace) -> None:
    from marguerite_workbench.security import safe_resolve

    text: Optional[str] = ns.text
    if ns.path:
        try:
            resolved = safe_resolve(s.workbench.workspace, ns.path)
            text = resolved.read_text(encoding="utf-8", errors="replace")
        except (ValueError, FileNotFoundError) as exc:
            raise CliError(f"kb ingest path error: {exc}", EXIT_REJECTED) from exc
    if not text:
        raise CliError("provide --text or --path", EXIT_USAGE)
    doc_id = ns.doc_id or f"cli_{int(time.time() * 1000)}"
    n = s.workbench.kb.ingest(text, doc_id)
    done(
        s,
        {"indexed_chunks": n, "doc_id": doc_id},
        lambda: print(_marker("ok"), f"indexed {n} chunk(s) as {doc_id}"),
    )


def cmd_kb_search(s: Session, ns: argparse.Namespace) -> None:
    hits = s.workbench.kb.search(ns.q, ns.k)
    done(
        s,
        hits,
        lambda: (
            print(_marker("ok"), f"{len(hits)} hit(s)") if hits else print("(no hits)"),
            [
                print(
                    f"  {i:>2}. score={h['score']:.4f} src={h['source']}\n"
                    f"      {h['text'][:160]}"
                )
                for i, h in enumerate(hits, 1)
            ],
        ),
    )


# ---------------------------------------------------------------------------
# experts
# ---------------------------------------------------------------------------
def cmd_experts_list(s: Session, ns: argparse.Namespace) -> None:
    from marguerite_workbench.experts import list_roles

    roles = list_roles()
    done(
        s,
        roles,
        lambda: (
            print(_marker("ok"), f"{len(roles)} distilled role(s)"),
            [
                print(f"  {r.role:<20} {r.title}")
                for r in roles
            ],
        ),
    )


def cmd_experts_analyze(s: Session, ns: argparse.Namespace) -> None:
    from marguerite_workbench.experts import analyze as expert_analyze
    from marguerite_workbench.experts import get_role

    if get_role(ns.role) is None:
        raise CliError(f"unknown expert role: {ns.role!r}", EXIT_NOT_FOUND)
    if ns.payload_file:
        try:
            payload = json.loads(Path(ns.payload_file).read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            raise CliError(f"--payload-file unreadable/invalid: {exc}", EXIT_REJECTED) from exc
    else:
        try:
            payload = json.loads(ns.payload) if ns.payload else {}
        except json.JSONDecodeError as exc:
            raise CliError(f"--payload is not valid JSON: {exc}", EXIT_REJECTED) from exc
    if not isinstance(payload, dict):
        raise CliError("payload must decode to a JSON object", EXIT_REJECTED)
    result = expert_analyze(ns.role, payload)
    # Mirror the API behaviour: consultations land in procedural (L6) memory.
    s.workbench.memory.store(
        "L6", "expert_analysis",
        f"{ns.role}: verdict={result.verdict} confidence={result.confidence}",
        refs=[ns.role],
    )
    done(s, result, lambda: _render_analysis(result))


# ---------------------------------------------------------------------------
# harness
# ---------------------------------------------------------------------------
def cmd_harness_adapters(s: Session, ns: argparse.Namespace) -> None:
    from marguerite_workbench.harness import adapters as harness_adapters

    rows = harness_adapters()
    done(
        s,
        rows,
        lambda: (
            print(_marker("ok"), f"{len(rows)} adapter(s)"),
            [
                print(
                    f"  {a.id:<12} {'available' if a.available else 'absent':<10} {a.name}"
                )
                for a in rows
            ],
        ),
    )


def cmd_harness_capabilities(s: Session, ns: argparse.Namespace) -> None:
    from marguerite_workbench.experts import list_roles

    caps = s.workbench.memory.capabilities(expert_count=len(list_roles()))
    backend = caps["memory_backend"]
    native_caps = caps["native_capabilities"]
    done(
        s,
        caps,
        lambda: (
            print(_marker("ok"), f"service={caps['service']} bridge v{native_caps['bridge_version']}"),
            _print_kv([
                ("memory_backend", backend["active"]),
                ("is_core_store", backend["is_core_store"]),
                ("db_path", backend["db_path"] or "-"),
                ("expert_count", native_caps["expert_count"]),
                ("memory_7layer", native_caps["memory_7layer"]),
                ("file_intel", native_caps["file_intel"]),
                ("knowledge_base", native_caps["knowledge_base"]),
                ("distilled_experts", native_caps["distilled_experts"]),
                ("research", native_caps["research"]),
            ]),
        ),
    )


def cmd_harness_dispatch(s: Session, ns: argparse.Namespace) -> None:
    from marguerite_workbench.harness import dispatch as harness_dispatch

    result = harness_dispatch(ns.task, ns.adapter, s.workbench.workspace, s.workbench.data_dir)
    done(s, result, lambda: _render_harness_result(result))


# ---------------------------------------------------------------------------
# research
# ---------------------------------------------------------------------------
def cmd_research(s: Session, ns: argparse.Namespace) -> None:
    from marguerite_workbench import research as research_mod

    research_mod.set_kb(s.workbench.kb)
    result = research_mod.research(ns.query)
    done(
        s,
        result,
        lambda: (
            print(_marker("ok"), f"query={result.query!r} sources={len(result.results)}"),
            print(_rule("=")),
            print(result.synthesized or "(no synthesis)"),
            [
                print(f"\n{i:>2}. {entry['title']}\n    {entry['url']}\n    {(entry.get('snippet') or '')[:140]}")
                for i, entry in enumerate(result.results, 1)
            ],
        ),
    )


# ---------------------------------------------------------------------------
# llmlab (支柱A) / ttc (支柱B) / agent (支柱C) — UPGRADE_PLAN_V2
# ---------------------------------------------------------------------------
def cmd_llmlab_tokenize(s: Session, ns: argparse.Namespace) -> None:
    from marguerite_workbench.llmlab import BPETokenizer

    tok = BPETokenizer.train(ns.corpus or ns.text, vocab_size=ns.vocab)
    trace = tok.encode_trace(ns.text)
    data = {
        "ids": trace["ids"], "n_tokens": len(trace["ids"]),
        "trace": trace["trace"], "compression": trace["compression"],
        "vocab_size": tok.vocab_size,
        "roundtrip_ok": tok.decode(trace["ids"]) == ns.text,
    }
    done(
        s, data,
        lambda: (
            print(_marker("ok" if data["roundtrip_ok"] else "fail"),
                  f"{len(data['ids'])} tokens (vocab={tok.vocab_size}, compression={data['compression']}x, "
                  f"roundtrip={'ok' if data['roundtrip_ok'] else 'FAILED'})"),
            print("ids:", data["ids"][:48], "…" if len(data["ids"]) > 48 else ""),
            print(f"merges applied: {len(data['trace'])}"),
        ),
    )


def cmd_llmlab_forward(s: Session, ns: argparse.Namespace) -> None:
    from marguerite_workbench.llmlab import Config, TinyGPT

    try:
        cfg = Config(vocab_size=ns.vocab, dim=ns.dim, n_heads=ns.heads,
                     n_layers=ns.layers, block_size=ns.block)
    except ValueError as exc:
        raise CliError(str(exc), EXIT_REJECTED) from exc
    ids = list(ns.text.encode("utf-8"))[: cfg.block_size]
    model = TinyGPT(cfg, seed=ns.seed)
    targets = ids[1:] + [ids[:1][0]]
    res = model.forward(ids, targets=targets, collect_attn=True)
    entropies = res.telemetry.get("entropy", [])
    mean_entropy = sum(entropies) / len(entropies) if entropies else 0.0

    def render() -> None:
        print(f"config dim={cfg.dim} heads={cfg.n_heads} layers={cfg.n_layers} T={len(ids)}")
        print(f"loss (next-token CE): {res.loss:.4f}")
        print(f"mean attention entropy: {mean_entropy:.4f} nats  ({len(entropies)} heads)")
        print("logits[last][:8]:", [round(v, 3) for v in res.logits[-1][:8]])

    payload: dict[str, Any] = {
        "tokens": ids, "loss": res.loss, "attn_entropy_mean": mean_entropy,
        "config": {"dim": cfg.dim, "n_heads": cfg.n_heads, "n_layers": cfg.n_layers},
    }
    if ns.attention:
        payload["attention"] = res.telemetry.get("attention", [])
    done(s, payload, render)


def cmd_llmlab_sample(s: Session, ns: argparse.Namespace) -> None:
    from marguerite_workbench.llmlab import TinyGPT, make_sampler

    cfg_kwargs: dict[str, Any] = {}
    if ns.vocab:
        cfg_kwargs["vocab_size"] = ns.vocab
    from marguerite_workbench.llmlab import Config

    block = max(64, len(ns.text.encode('utf-8')))
    cfg = Config(dim=ns.dim, n_heads=ns.heads, n_layers=ns.layers, block_size=block, **cfg_kwargs)
    model = TinyGPT(cfg, seed=ns.seed)
    draw = make_sampler(seed=ns.sample_seed, temperature=ns.temperature,
                        top_k=ns.top_k, top_p=ns.top_p,
                        frequency_penalty=ns.freq_penalty, presence_penalty=ns.pres_penalty)
    ids = list(ns.text.encode("utf-8"))
    out = model.generate(ids, ns.max_new, draw)
    generated = out[len(ids):]
    preview = bytes(out).decode("utf-8", errors="replace")
    done(
        s, {"prompt_tokens": ids, "generated_ids": generated, "preview": preview},
        lambda: (
            print(_marker("ok"), f"generated {len(generated)} token(s): {generated[:24]}"),
            print(_paint(preview, _ANSI_DIM)),
        ),
    )


def cmd_ttc_run(s: Session, ns: argparse.Namespace) -> None:
    from marguerite_workbench import ttc as ttc_mod

    result = ttc_mod.run_strategy(ns.strategy, ns.prompt, n=ns.n, kb=s.workbench.kb)
    d = result.to_dict()
    done(
        s, d,
        lambda: (
            print(_marker("ok"), f"[{d['strategy']}/{d['engine']}] agreement={d['agreement']}"),
            print(d["answer"][:400]),
            if_votes_print(d),
        ),
    )


def if_votes_print(d: dict) -> None:
    if d.get("votes"):
        ranked = sorted(d["votes"].items(), key=lambda kv: kv[1], reverse=True)[:3]
        for key, count in ranked:
            print(_paint(f"  vote x{count}: {key[:80]}", _ANSI_DIM))


def cmd_ttc_budget(s: Session, ns: argparse.Namespace) -> None:
    from marguerite_workbench import ttc as ttc_mod

    budget = ttc_mod.budget_for(ns.task)
    preset_hint = ttc_mod.preset_for("reasoning")
    budget["suggested_sampling"] = preset_hint
    done(
        s, budget,
        lambda: print(_marker("ok"), f"difficulty={budget['difficulty']} budget={budget}"),
    )


def cmd_agent_run(s: Session, ns: argparse.Namespace) -> None:
    from marguerite_workbench.agentloop import AgentLoop
    from marguerite_workbench.events import EventBus

    loop = AgentLoop(
        workspace=s.workbench.workspace, kb=s.workbench.kb, memory=s.workbench.memory,
        sessions_dir=s.workbench.data_dir / "sessions",
        bus=EventBus(),  # per-invocation bus; persistence lives in the session log
        max_rounds=ns.max_rounds, max_repairs=ns.max_repairs,
    )
    result = loop.run(ns.task)
    d = result.to_dict()

    def render() -> None:
        kind = "ok" if d["status"] == "completed" else "fail"
        print(_marker(kind), f"status={d['status']} session={d['session_id']} steps={len(d['steps'])}")
        for st in d["steps"]:
            glyph = "[OK]" if st["ok"] else "[FAIL]"
            tool = (st.get("tool") or "-") + (" [repair]" if st.get("is_repair") else "")
            err = f" error={st['error']}" if st.get("error") else ""
            print(f"  {st['index']:>2} {st['state']:<9} {glyph} {tool:<22} {st['duration_ms']:>4}ms{err}")
        print(_rule())
        print(d["answer"])

    done(s, d, render)


def cmd_agent_tools(s: Session, ns: argparse.Namespace) -> None:
    from marguerite_workbench.agentloop import TOOL_REGISTRY

    rows = [
        {"name": name, "required_args": list(spec.required),
         "safe_resolve_guarded": name in ("read_file", "list_dir")}
        for name, spec in sorted(TOOL_REGISTRY.items())
    ]
    done(
        s, {"tools": rows},
        lambda: (
            print(_marker("ok"), f"{len(rows)} sandboxed tool(s)"),
            [print(f"  {r['name']:<14} required={r['required_args'] or '-'}") for r in rows],
        ),
    )


def cmd_agent_log(s: Session, ns: argparse.Namespace) -> None:
    path = Path(s.workbench.data_dir) / "sessions" / f"{ns.session}.jsonl"
    if not path.exists():
        raise CliError(f"no session log {path}", EXIT_NOT_FOUND)
    entries = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    done(
        s, {"session": ns.session, "file": str(path), "entries": entries},
        lambda: [
            print(f"{e.get('seq', '-'):>4} {e.get('type', '?'):<22} "
                  f"{json.dumps(e.get('payload', {}), ensure_ascii=False)[:110]}")
            for e in entries
        ],
    )


# ---------------------------------------------------------------------------
# serve / doctor
# ---------------------------------------------------------------------------
def cmd_serve(s: Session, ns: argparse.Namespace) -> None:
    # Loopback-first binding mirrors the core invariant: never public by default.
    import uvicorn

    from marguerite_workbench.api import app

    print(_marker("ok"), f"serving on http://{ns.host}:{ns.port}  (docs at /docs)")
    uvicorn.run(app, host=ns.host, port=ns.port, log_level="info")


_CORE_DEPS = ("fastapi", "uvicorn", "pydantic", "httpx", "pygments")


def _check_writable_dir(path: Any) -> tuple[str, str]:
    probe = path / ".doctor_write_probe"
    try:
        probe.write_text("probe", encoding="utf-8")
        probe.unlink()
        return "ok", str(path)
    except OSError as exc:
        return "fail", f"{path} ({exc})"


def doctor_checks() -> list[dict[str, str]]:
    """Real environment probes — each entry is executed, nothing is assumed."""
    import importlib.util
    import inspect as inspect_mod
    import platform

    from marguerite_workbench import config

    checks: list[dict[str, str]] = []

    if sys.version_info >= DOCTOR_MIN_PYTHON:
        checks.append({"name": "python", "status": "ok",
                       "detail": platform.python_version()})
    else:
        checks.append({"name": "python", "status": "fail",
                       "detail": f"{platform.python_version()} < {DOCTOR_MIN_PYTHON}"})

    missing = [d for d in _CORE_DEPS if importlib.util.find_spec(d) is None]
    checks.append({
        "name": "dependencies",
        "status": "ok" if not missing else "fail",
        "detail": ", ".join(_CORE_DEPS) if not missing else f"missing: {', '.join(missing)}",
    })

    try:
        from starlette.routing import Router

        needs_shim = (
            "on_startup" not in inspect_mod.signature(Router.__init__).parameters
        )
        checks.append({
            "name": "fastapi_starlette_compat",
            "status": "ok",
            "detail": "shim active (starlette>=1.0)" if needs_shim else "native kwargs supported",
        })
    except Exception as exc:  # pragma: no cover - defensive, starlette is a dep
        checks.append({"name": "fastapi_starlette_compat", "status": "warn",
                       "detail": f"probe failed: {exc}"})

    status, detail = _check_writable_dir(config.workspace_root())
    checks.append({"name": "workspace_dir", "status": status, "detail": detail})
    status, detail = _check_writable_dir(config.data_dir())
    checks.append({"name": "data_dir", "status": status, "detail": detail})

    llm_ready = bool(os.environ.get("UNIV_LLM_BASE_URL")) and bool(os.environ.get("UNIV_LLM_KEY"))
    checks.append({
        "name": "llm_provider",
        "status": "ok" if llm_ready else "warn",
        "detail": "UNIV_LLM_* configured" if llm_ready
        else "not configured - deterministic fallbacks active (offline-first)",
    })
    search_key = os.environ.get("SERPER_API_KEY") or os.environ.get("UNIV_SEARCH_KEY")
    checks.append({
        "name": "web_search",
        "status": "ok" if search_key else "warn",
        "detail": "provider key present" if search_key
        else "no key - research falls back to local KB",
    })

    try:
        from marguerite_workbench.memory import MemoryEngine

        engine = MemoryEngine(config.data_dir() / "memory.jsonl")
        backend = engine.bridge.active_backend().value
        native = engine.is_native()
        checks.append({
            "name": "memory_bridge",
            "status": "ok",
            "detail": f"backend={backend} native={'yes' if native else 'no (local store)'}",
        })
    except Exception as exc:
        checks.append({"name": "memory_bridge", "status": "fail", "detail": str(exc)})

    from marguerite_workbench.harness import adapters as harness_adapters

    external = [a.id for a in harness_adapters() if a.available and a.command]
    checks.append({
        "name": "harness_clis",
        "status": "ok" if external else "warn",
        "detail": ", ".join(external) if external else "no external harness CLIs - internal loop only",
    })
    return checks


def cmd_doctor(s: Session, ns: argparse.Namespace) -> None:
    results = doctor_checks()
    failed = [c for c in results if c["status"] == "fail"]
    healthy = not failed
    data = {"checks": results, "healthy": healthy}

    def render() -> None:
        print(_paint("Marguerite Workbench doctor", _ANSI_DIM))
        print(_rule("="))
        for c in results:
            print(f"{_marker(c['status'])} {c['name']:<26} {c['detail']}")

    if s.as_json:
        # Doctor owns its envelope: a failing probe is *data* (ok:false with the
        # full check list), not a missing result — so both shapes stay honest.
        envelope = {
            "ok": healthy,
            "data": data,
            "meta": {
                "cmd": " ".join(sys.argv[1:]),
                "duration_ms": int((time.perf_counter() - s.started) * 1000),
            },
        }
        if not healthy:
            envelope["error"] = {"message": f"{len(failed)} check(s) failed", "exit_code": EXIT_RUNTIME}
        json.dump(envelope, sys.stdout, ensure_ascii=False, indent=2, default=str)
        sys.stdout.write("\n")
    else:
        render()

    return EXIT_OK if healthy else EXIT_RUNTIME


# ---------------------------------------------------------------------------
# Argument wiring (table-driven)
# ---------------------------------------------------------------------------
def _add_files(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("files", help="load & act on workspace files")
    fsub = p.add_subparsers(dest="verb", required=True)

    load = fsub.add_parser("load", help="load a file into the intelligence engine")
    load.add_argument("--path", required=True)
    load.set_defaults(func=cmd_files_load)

    disp = fsub.add_parser("display", help="show the highlighted display artifact")
    disp.add_argument("--id", dest="file_id", required=True)
    disp.set_defaults(func=cmd_files_display)

    mod = fsub.add_parser("modify", help="apply a rule/LLM edit with automatic backup")
    mod.add_argument("--id", dest="file_id", required=True)
    mod.add_argument("--instruction", required=True)
    mod.set_defaults(func=cmd_files_modify)

    col = fsub.add_parser("collapse", help="collapse sections (default: auto-select long ones)")
    col.add_argument("--id", dest="file_id", required=True)
    col.add_argument("--section", nargs="*", default=None, metavar="SECTION_ID")
    col.set_defaults(func=cmd_files_collapse)

    ask = fsub.add_parser("ask", help="ask a grounded question about a loaded file")
    ask.add_argument("--id", dest="file_id", required=True)
    ask.add_argument("--question", required=True)
    ask.set_defaults(func=cmd_files_ask)

    inter = fsub.add_parser("interact", help="multi-turn chat scoped to one file")
    inter.add_argument("--id", dest="file_id", required=True)
    inter.add_argument("--message", required=True)
    inter.add_argument("--session", default=None)
    inter.set_defaults(func=cmd_files_interact)


def _add_memory(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("memory", help="operate the 7-layer memory engine")
    msub = p.add_subparsers(dest="verb", required=True)

    layers = msub.add_parser("layers", help="list layer statuses")
    layers.set_defaults(func=cmd_memory_layers)

    store = msub.add_parser("store", help="store one memory item")
    store.add_argument("layer", help="L1..L7")
    store.add_argument("--content", required=True)
    store.add_argument("--kind", default="note")
    store.add_argument("--ref", action="append", default=None)
    store.set_defaults(func=cmd_memory_store)

    query = msub.add_parser("query", help="semantic query within a layer")
    query.add_argument("layer", help="L1..L7")
    query.add_argument("--q", required=True)
    query.add_argument("-k", type=int, default=QUERY_K_DEFAULT)
    query.set_defaults(func=cmd_memory_query)

    sync = msub.add_parser("sync", help="flush local memories into the core store when reachable")
    sync.set_defaults(func=cmd_memory_sync)


def _add_kb(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("kb", help="BM25+vector hybrid knowledge base")
    ksub = p.add_subparsers(dest="verb", required=True)

    ingest = ksub.add_parser("ingest", help="index text or a workspace file")
    ingest.add_argument("--text", default=None)
    ingest.add_argument("--path", default=None, help="workspace-relative path")
    ingest.add_argument("--doc-id", default=None)
    ingest.set_defaults(func=cmd_kb_ingest)

    search = ksub.add_parser("search", help="hybrid retrieval")
    search.add_argument("--q", required=True)
    search.add_argument("-k", type=int, default=QUERY_K_DEFAULT)
    search.set_defaults(func=cmd_kb_search)


def _add_experts(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("experts", help="distilled expert roles")
    esub = p.add_subparsers(dest="verb", required=True)

    lst = esub.add_parser("list", help="list roles")
    lst.set_defaults(func=cmd_experts_list)

    ana = esub.add_parser("analyze", help="run a real analysis with one role")
    ana.add_argument("role")
    ana.add_argument("--payload", default="{}", help="inline JSON object")
    ana.add_argument("--payload-file", default=None, help="read JSON object from file instead")
    ana.set_defaults(func=cmd_experts_analyze)


def _add_harness(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("harness", help="codex-class execution adapters")
    hsub = p.add_subparsers(dest="verb", required=True)

    ad = hsub.add_parser("adapters", help="list adapters and availability")
    ad.set_defaults(func=cmd_harness_adapters)

    caps = hsub.add_parser("capabilities", help="native capability manifest + active backend")
    caps.set_defaults(func=cmd_harness_capabilities)

    dis = hsub.add_parser("dispatch", help="run a task through plan→act→observe")
    dis.add_argument("task")
    dis.add_argument("--adapter", default=None)
    dis.set_defaults(func=cmd_harness_dispatch)


def build_parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(
        prog="python -m marguerite_workbench",
        description="Marguerite File Intelligence Workbench CLI",
    )
    root.add_argument("--json", action="store_true", help="emit {ok,data,meta} JSON envelope")
    root.add_argument("--workspace", default=None, help="override MARGERITE_WORKSPACE")
    root.add_argument("--data-dir", default=None, help="override MARGERITE_DATA_DIR")

    sub = root.add_subparsers(dest="command", required=True)
    _add_files(sub)
    _add_memory(sub)
    _add_kb(sub)
    _add_experts(sub)
    _add_harness(sub)

    res = sub.add_parser("research", help="online research with local fallback")
    res.add_argument("query")
    res.set_defaults(func=cmd_research)

    serve = sub.add_parser("serve", help="run the FastAPI server (loopback by default)")
    serve.add_argument("--host", default=DEFAULT_HOST)
    serve.add_argument("--port", type=int, default=DEFAULT_PORT)
    serve.set_defaults(func=cmd_serve)

    doc = sub.add_parser("doctor", help="probe dependencies, directories and bridges")
    doc.set_defaults(func=cmd_doctor)

    lab = sub.add_parser("llmlab", help="first-principles LLM lab (BPE / transformer / sampling)")
    lsub = lab.add_subparsers(dest="verb", required=True)

    tok = lsub.add_parser("tokenize", help="byte-level BPE with merge trace")
    tok.add_argument("--text", required=True)
    tok.add_argument("--vocab", type=int, default=276)
    tok.add_argument("--corpus", default=None, help="training corpus (defaults to --text)")
    tok.set_defaults(func=cmd_llmlab_tokenize)

    fwd = lsub.add_parser("forward", help="tiny transformer forward pass + attention telemetry")
    fwd.add_argument("--text", required=True)
    fwd.add_argument("--dim", type=int, default=32)
    fwd.add_argument("--heads", type=int, default=4)
    fwd.add_argument("--layers", type=int, default=2)
    fwd.add_argument("--block", type=int, default=64)
    fwd.add_argument("--vocab", type=int, default=256)
    fwd.add_argument("--seed", type=int, default=42)
    fwd.add_argument("--attention", action="store_true", help="include full attention maps in --json")
    fwd.set_defaults(func=cmd_llmlab_forward)

    smp = lsub.add_parser("sample", help="sampling-policy playground over the tiny model")
    smp.add_argument("--text", required=True)
    smp.add_argument("--max-new", type=int, default=16)
    smp.add_argument("--temperature", type=float, default=1.0)
    smp.add_argument("--top-k", type=int, default=None)
    smp.add_argument("--top-p", type=float, default=None)
    smp.add_argument("--freq-penalty", type=float, default=0.0)
    smp.add_argument("--pres-penalty", type=float, default=0.0)
    smp.add_argument("--dim", type=int, default=32)
    smp.add_argument("--heads", type=int, default=4)
    smp.add_argument("--layers", type=int, default=2)
    smp.add_argument("--vocab", type=int, default=None)
    smp.add_argument("--seed", type=int, default=42)
    smp.add_argument("--sample-seed", type=int, default=7)
    smp.set_defaults(func=cmd_llmlab_sample)

    tc = sub.add_parser("ttc", help="test-time compute strategies (SC / BoN / budgets)")
    tsub = tc.add_subparsers(dest="verb", required=True)

    run = tsub.add_parser("run", help="run a strategy")
    run.add_argument("--strategy", default="self_consistency", choices=["self_consistency", "best_of_n"])
    run.add_argument("--prompt", required=True)
    run.add_argument("-n", type=int, default=3)
    run.set_defaults(func=cmd_ttc_run)

    bud = tsub.add_parser("budget", help="difficulty heuristic → compute budget")
    bud.add_argument("task")
    bud.set_defaults(func=cmd_ttc_budget)

    ag = sub.add_parser("agent", help="Codex-style agent loop (plan→act→observe→repair)")
    asub = ag.add_subparsers(dest="verb", required=True)

    arun = asub.add_parser("run", help="execute a task through the loop")
    arun.add_argument("task")
    arun.add_argument("--max-rounds", type=int, default=8)
    arun.add_argument("--max-repairs", type=int, default=3)
    arun.set_defaults(func=cmd_agent_run)

    atools = asub.add_parser("tools", help="show the sandboxed tool registry manifest")
    atools.set_defaults(func=cmd_agent_tools)

    alog = asub.add_parser("log", help="dump an append-only session transcript")
    alog.add_argument("session")
    alog.set_defaults(func=cmd_agent_log)
    return root


def main(argv: Optional[list[str]] = None) -> int:
    parser = build_parser()
    ns = parser.parse_args(argv)

    if getattr(ns, "workspace", None):
        os.environ["MARGERITE_WORKSPACE"] = ns.workspace
    if getattr(ns, "data_dir", None):
        os.environ["MARGERITE_DATA_DIR"] = ns.data_dir

    session = Session(as_json=bool(ns.json))
    try:
        # Handlers either return an explicit exit code or None (= success).
        return int(ns.func(session, ns) or EXIT_OK)
    except CliError as exc:
        _report_error(session, str(exc), exc.exit_code)
        return exc.exit_code
    except KeyError as exc:
        _report_error(session, f"not found: {exc}", EXIT_NOT_FOUND)
        return EXIT_NOT_FOUND
    except ValueError as exc:
        _report_error(session, str(exc), EXIT_REJECTED)
        return EXIT_REJECTED
    except Exception as exc:  # noqa: BLE001 — single boundary converts to contract
        _report_error(session, f"{type(exc).__name__}: {exc}", EXIT_RUNTIME)
        return EXIT_RUNTIME


def _report_error(session: Session, message: str, exit_code: int) -> None:
    if session.as_json:
        payload = {
            "ok": False,
            "error": {"message": message, "exit_code": exit_code},
            "meta": {"cmd": " ".join(sys.argv[1:])},
        }
        json.dump(payload, sys.stderr, ensure_ascii=False, indent=2, default=str)
        sys.stderr.write("\n")
    else:
        print(f"{_marker('fail')} {message}", file=sys.stderr)


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
