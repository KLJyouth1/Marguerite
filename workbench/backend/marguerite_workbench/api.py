"""Marguerite File Intelligence Workbench — FastAPI application (port 8011).

Wires the contract's endpoints to the engine modules.  A single
:class:`Workbench` (workspace + engines) is built at startup and stored on
``app.state``; every route pulls it via the ``wb`` dependency.  File paths are
always funnelled through ``security.safe_resolve``.
"""

from __future__ import annotations

import logging
import os
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Optional

from fastapi import Depends, FastAPI, HTTPException, Path as ApiPath, Query, Request
from fastapi.middleware.cors import CORSMiddleware

# Must run BEFORE any FastAPI app is constructed: translates the legacy
# on_startup/on_shutdown kwargs (still emitted by fastapi 0.115) for
# starlette >= 1.0 where they were removed.  No-op on older Starlette.
from marguerite_workbench import _compat

_compat.install()

from marguerite_workbench import config, research
from marguerite_workbench.engine import Workbench
from marguerite_workbench.experts import analyze as expert_analyze
from marguerite_workbench.experts import (
    get_role,
    list_roles,
    list_modes,
    list_teams,
    analyze_with_mode,
    chat_with_experts,
)
from marguerite_workbench.file_intel import FileIntel  # noqa: F401 — re-exported pre-existing symbol
from marguerite_workbench.harness import adapters as harness_adapters
from marguerite_workbench.harness import dispatch as harness_dispatch
from marguerite_workbench.kb import KnowledgeBase  # noqa: F401 — re-exported pre-existing symbol
from marguerite_workbench.memory import MemoryEngine  # noqa: F401 — re-exported pre-existing symbol
from marguerite_workbench.models import (
    AskResult, DisplayResult, ExpertAnalysis, ExpertRole, FileCard, FileSection,
    HarnessAdapter, HarnessResult, InteractResult, KbIngestResult, KbSearchHit,
    LayerStatus, MemoryItem, MemoryQueryResult, ModifyResult, ResearchResult,
)

logger = logging.getLogger("marguerite.api")
logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s")


@asynccontextmanager
async def lifespan(app: FastAPI):
    from marguerite_workbench.events import EventBus

    app.state.wb = Workbench(config.workspace_root(), config.data_dir())
    app.state.bus = EventBus()
    logger.info("Workbench ready: workspace=%s data=%s", app.state.wb.workspace, app.state.wb.data_dir)
    try:
        yield
    finally:
        # 关闭原生记忆桥接（释放底层连接）。
        try:
            await app.state.wb.memory.close()
        except Exception as exc:  # pragma: no cover — 防御性，关闭失败不影响退出
            logger.warning("memory bridge close failed: %s", exc)


app = FastAPI(title="Marguerite File Intelligence Workbench", version="1.1.0", lifespan=lifespan)
_compat.ensure_app_attributes(app)

# CORS policy — restrictive by default (localhost-only).  The dangerous
# ``allow_origins=["*"]`` + ``allow_credentials=True`` combination is avoided in
# BOTH branches: widening requires the explicit ``MARGERITE_CORS_ALLOW_ALL`` opt-in
# flag AND drops credentials, so credentialed cross-origin requests stay
# localhost-scoped (CSRF / localhost-CSRF / DNS-rebinding safe).
_CORS_ALLOW_ALL = os.environ.get("MARGERITE_CORS_ALLOW_ALL", "").lower() in ("1", "true", "yes")
if _CORS_ALLOW_ALL:
    _CORS_ORIGINS: list[str] = ["*"]
    _CORS_CREDENTIALS = False
else:
    _CORS_ORIGINS = [
        "http://localhost", "http://127.0.0.1",
        "http://localhost:8011", "http://127.0.0.1:8011",
    ]
    _CORS_CREDENTIALS = True

app.add_middleware(
    CORSMiddleware,
    allow_origins=_CORS_ORIGINS, allow_credentials=_CORS_CREDENTIALS,
    allow_methods=["*"], allow_headers=["*"],
)


def wb(request: Request) -> Workbench:
    w = getattr(request.app.state, "wb", None)
    if w is None:  # pragma: no cover — defensive; lifespan always sets it
        w = Workbench(config.workspace_root(), config.data_dir())
        request.app.state.wb = w
    return w


# ---------------------------------------------------------------------------
# Files
# ---------------------------------------------------------------------------
@app.post("/api/files/load", response_model=FileCard)
def load_file(body: dict, workbench: Workbench = Depends(wb)):
    path = body.get("path")
    if not path:
        raise HTTPException(status_code=400, detail="missing 'path'")
    try:
        return workbench.files.load(path)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc


@app.get("/api/files/{file_id:path}", response_model=DisplayResult)
def display_file(file_id: str = ApiPath(...), workbench: Workbench = Depends(wb)):
    try:
        return workbench.files.display(file_id)
    except KeyError as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc


@app.post("/api/files/{file_id:path}/modify", response_model=ModifyResult)
def modify_file(
    file_id: str = ApiPath(...),
    body: dict = {},
    workbench: Workbench = Depends(wb),
):
    instruction = body.get("instruction", "")
    if not instruction:
        raise HTTPException(status_code=400, detail="missing 'instruction'")
    try:
        return workbench.files.modify(file_id, instruction)
    except KeyError as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc


@app.post("/api/files/{file_id:path}/collapse", response_model=FileCard)
def collapse_file(
    file_id: str = ApiPath(...),
    body: dict = {},
    workbench: Workbench = Depends(wb),
):
    section_ids = body.get("section_ids")
    try:
        return workbench.files.collapse(file_id, section_ids)
    except KeyError as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc


@app.post("/api/files/{file_id:path}/ask", response_model=AskResult)
def ask_file(
    file_id: str = ApiPath(...),
    body: dict = {},
    workbench: Workbench = Depends(wb),
):
    question = body.get("question", "")
    if not question:
        raise HTTPException(status_code=400, detail="missing 'question'")
    try:
        return workbench.files.ask(file_id, question)
    except KeyError as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc


@app.post("/api/files/{file_id:path}/interact", response_model=InteractResult)
def interact_file(
    file_id: str = ApiPath(...),
    body: dict = {},
    workbench: Workbench = Depends(wb),
):
    message = body.get("message", "")
    if not message:
        raise HTTPException(status_code=400, detail="missing 'message'")
    try:
        return workbench.files.interact(file_id, message, body.get("session_id"))
    except KeyError as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc


# ---------------------------------------------------------------------------
# Memory (7 layers)
# ---------------------------------------------------------------------------
@app.get("/api/memory/layers", response_model=list[LayerStatus])
def memory_layers(workbench: Workbench = Depends(wb)):
    return [LayerStatus(**s) for s in workbench.memory.layers_status()]


@app.post("/api/memory/{layer}/store", response_model=MemoryItem)
def memory_store(layer: str, item: MemoryItem, workbench: Workbench = Depends(wb)):
    try:
        stored = workbench.memory.store(
            layer=layer, kind=item.kind or "note", content=item.content,
            refs=item.refs or None, embedding=item.embedding,
        )
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    return MemoryItem(**stored)


@app.get("/api/memory/{layer}/query", response_model=MemoryQueryResult)
def memory_query(
    layer: str,
    q: str = Query(..., min_length=1),
    k: int = Query(5, ge=1, le=50),
    workbench: Workbench = Depends(wb),
):
    try:
        hits = workbench.memory.query(layer, q, k)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    # Drop the heavy embedding vector from query responses (keep payloads light).
    items = [MemoryItem(**{**h, "embedding": None}) for h in hits]
    return MemoryQueryResult(items=items)


@app.post("/memory/sync")
async def memory_sync(workbench: Workbench = Depends(wb)):
    """Local → Core 冲刷：仅当存在可达的 Marguerite 核心库时生效。

    把当前 Local 模式下的记忆条目导入核心库；若已是 CoreSqlite 或发现不了核心库，
    则安全 no-op 并返回 200（符合「no-op / 200 otherwise」契约，不伪造成功）。
    """
    result = await workbench.memory.sync_to_core()
    return result


# ---------------------------------------------------------------------------
# Knowledge base
# ---------------------------------------------------------------------------
@app.post("/api/kb/ingest", response_model=KbIngestResult)
def kb_ingest(body: dict, workbench: Workbench = Depends(wb)):
    text = body.get("text")
    path = body.get("path")
    doc_id = body.get("doc_id") or f"doc_{abs(hash(text or path or '')) % 10**9}"
    if path:
        try:
            from marguerite_workbench.security import safe_resolve

            resolved = safe_resolve(workbench.workspace, path)
            text = resolved.read_text(encoding="utf-8", errors="replace")
        except (ValueError, FileNotFoundError) as exc:
            raise HTTPException(status_code=400, detail=f"kb ingest path error: {exc}") from exc
    if not text:
        raise HTTPException(status_code=400, detail="provide 'text' or 'path'")
    n = workbench.kb.ingest(text, doc_id)
    return KbIngestResult(indexed_chunks=n)


@app.get("/api/kb/search", response_model=list[KbSearchHit])
def kb_search(
    q: str = Query(..., min_length=1),
    k: int = Query(5, ge=1, le=50),
    workbench: Workbench = Depends(wb),
):
    hits = workbench.kb.search(q, k)
    return [KbSearchHit(**h) for h in hits]


# ---------------------------------------------------------------------------
# Experts
# ---------------------------------------------------------------------------
@app.get("/api/experts", response_model=list[ExpertRole])
def list_experts():
    return list_roles()


@app.get("/api/experts/teams", response_model=list[dict])
def list_expert_teams():
    """Return the eight expert teams (plus the core group) with member role ids."""
    return list_teams()


@app.post("/api/experts/{role}/analyze", response_model=dict)
def analyze_expert(role: str, body: dict, workbench: Workbench = Depends(wb)):
    if get_role(role) is None:
        raise HTTPException(status_code=404, detail=f"unknown expert role: {role!r}")
    payload = body.get("payload")
    if not isinstance(payload, dict):
        raise HTTPException(status_code=400, detail="'payload' must be an object")
    mode = (body.get("mode") or "standard_ask") if isinstance(body, dict) else "standard_ask"
    try:
        result = analyze_with_mode(role, payload, mode)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc))
    # Persist the consultation into procedural (L6) memory for future recall.
    verdict = result.get("verdict")
    confidence = result.get("confidence")
    workbench.memory.store(
        "L6", "expert_analysis",
        f"{role} [{mode}]: verdict={verdict} confidence={confidence}",
        refs=[role],
    )
    return result


@app.get("/api/experts/modes", response_model=list[dict])
def list_expert_modes():
    """Return the 9 canonical expert modes (3 intensities x 3 working modes)."""
    return list_modes()


@app.post("/api/experts/chat")
def expert_chat(body: dict):
    """Resolve @mentions in chat text and analyze each under the chosen mode."""
    text = body.get("text", "") if isinstance(body, dict) else ""
    mode = body.get("mode", "standard_ask") if isinstance(body, dict) else "standard_ask"
    default_role = body.get("default_role") if isinstance(body, dict) else None
    return chat_with_experts(text, mode=mode, default_role=default_role)


# ---------------------------------------------------------------------------
# Harness
# ---------------------------------------------------------------------------
@app.get("/api/harness/adapters", response_model=list[HarnessAdapter])
def harness_list():
    return harness_adapters()


@app.get("/harness/capabilities")
async def harness_capabilities(workbench: Workbench = Depends(wb)):
    """返回原生能力清单 + 当前活跃后端（CoreSqlite | Local）。"""
    from marguerite_workbench.experts import list_roles

    return workbench.memory.capabilities(expert_count=len(list_roles()))


@app.post("/api/harness/dispatch", response_model=HarnessResult)
def harness_run(body: dict, workbench: Workbench = Depends(wb)):
    task = body.get("task")
    if not task:
        raise HTTPException(status_code=400, detail="missing 'task'")
    return harness_dispatch(task, body.get("adapter"), workbench.workspace, workbench.data_dir)


# ---------------------------------------------------------------------------
# Research (联网)
# ---------------------------------------------------------------------------
@app.post("/api/research", response_model=ResearchResult)
def research_endpoint(body: dict, workbench: Workbench = Depends(wb)):
    query = body.get("query")
    if not query:
        raise HTTPException(status_code=400, detail="missing 'query'")
    return research.research(query)


# ---------------------------------------------------------------------------
# Health
# ---------------------------------------------------------------------------
@app.get("/api/health")
def health():
    return {"status": "ok", "service": "marguerite-file-intel"}


# ---------------------------------------------------------------------------
# LLMLab (支柱A) / TTC (支柱B) / Agent Loop (支柱C) — UPGRADE_PLAN_V2
# ---------------------------------------------------------------------------
_DEFAULT_CORPUS = (
    "the quick brown fox jumps over the lazy dog. "
    "attention is all you need. transformers learn representations. "
    "marguerite is a terminal-first agent workbench. "
) * 4


@app.post("/api/llmlab/tokenize")
def llmlab_tokenize(body: dict):
    from marguerite_workbench.llmlab import BPETokenizer

    text = body.get("text")
    if not text:
        raise HTTPException(status_code=400, detail="missing 'text'")
    vocab_size = int(body.get("vocab_size", 276))
    corpus = str(body.get("corpus") or (_DEFAULT_CORPUS + text))
    tok = BPETokenizer.train(corpus, vocab_size=vocab_size)
    result = tok.encode_trace(text)
    return {
        "ids": result["ids"],
        "n_tokens": len(result["ids"]),
        "trace": result["trace"][:64],
        "compression": result["compression"],
        "vocab_size": tok.vocab_size,
        "decoded_roundtrip_ok": tok.decode(result["ids"]) == text,
    }


@app.post("/api/llmlab/forward")
def llmlab_forward(body: dict):
    from marguerite_workbench.llmlab import BPETokenizer, Config, TinyGPT

    text = body.get("text")
    if not text:
        raise HTTPException(status_code=400, detail="missing 'text'")
    try:
        cfg = Config(
            vocab_size=int(body.get("vocab_size", 256)),
            dim=int(body.get("dim", 32)),
            n_heads=int(body.get("n_heads", 4)),
            n_layers=int(body.get("n_layers", 2)),
            block_size=int(body.get("block_size", 64)),
        )
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    model = TinyGPT(cfg, seed=int(body.get("seed", 42)))
    ids = list(text.encode("utf-8"))[: cfg.block_size]
    if not ids:
        raise HTTPException(status_code=400, detail="text encodes to zero tokens")
    targets = ids[1:] + [ids[0]]  # toy next-token supervision for a real CE value
    res = model.forward(ids, targets=targets, collect_attn=bool(body.get("collect_attn", True)))
    attn = res.telemetry.get("attention", [])
    return {
        "config": {"vocab_size": cfg.vocab_size, "dim": cfg.dim, "n_heads": cfg.n_heads,
                   "n_layers": cfg.n_layers, "block_size": cfg.block_size},
        "tokens": ids,
        "loss": res.loss,
        "logits_head": res.logits[-1][: min(16, cfg.vocab_size)],
        "attention": [
            [[row[: min(len(row), 32)] for row in head] for head in layer]
            for layer in attn
        ],
    }


@app.post("/api/llmlab/sample")
def llmlab_sample(body: dict):
    from marguerite_workbench.llmlab import BPETokenizer, Config, TinyGPT, make_sampler

    text = body.get("text")
    if not text:
        raise HTTPException(status_code=400, detail="missing 'text'")
    max_new = max(1, min(int(body.get("max_new_tokens", 16)), 128))
    cfg = Config(
        vocab_size=int(body.get("vocab_size", 256)),
        dim=int(body.get("dim", 32)),
        n_heads=int(body.get("n_heads", 4)),
        n_layers=int(body.get("n_layers", 2)),
        block_size=max(int(body.get("block_size", 64)), len(text.encode('utf-8')[:64])),
    )
    model = TinyGPT(cfg, seed=int(body.get("seed", 42)))
    draw = make_sampler(
        seed=int(body.get("sample_seed", 7)),
        temperature=float(body.get("temperature", 1.0)),
        top_k=body.get("top_k"),
        top_p=body.get("top_p"),
        frequency_penalty=float(body.get("frequency_penalty", 0.0)),
        presence_penalty=float(body.get("presence_penalty", 0.0)),
    )
    ids = list(text.encode("utf-8"))[: cfg.block_size]
    out_ids = model.generate(ids, max_new, draw)
    raw = bytes(out_ids).decode("utf-8", errors="replace")
    return {"prompt_tokens": ids, "generated_ids": out_ids[max(0, len(ids)):],
            "generated_text_preview": raw, "max_new_tokens": max_new}


@app.post("/api/ttc/run")
def ttc_run(body: dict, workbench: Workbench = Depends(wb)):
    from marguerite_workbench import ttc as ttc_mod

    prompt = body.get("prompt")
    if not prompt:
        raise HTTPException(status_code=400, detail="missing 'prompt'")
    strategy = body.get("strategy", "self_consistency")
    n = int(body.get("n", 3))
    try:
        result = ttc_mod.run_strategy(strategy, prompt, n=n, kb=workbench.kb)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    return result.to_dict()


@app.get("/api/ttc/budget")
def ttc_budget(task: str = Query(..., min_length=1)):
    from marguerite_workbench import ttc as ttc_mod

    return ttc_mod.budget_for(task)


@app.post("/api/agent/run")
def agent_run(body: dict, workbench: Workbench = Depends(wb)):
    from marguerite_workbench.agentloop import DEFAULT_MAX_REPAIRS, DEFAULT_MAX_ROUNDS, AgentLoop

    task = body.get("task")
    if not task:
        raise HTTPException(status_code=400, detail="missing 'task'")
    loop = AgentLoop(
        workspace=workbench.workspace,
        kb=workbench.kb,
        memory=workbench.memory,
        sessions_dir=workbench.data_dir / "sessions",
        bus=getattr(app.state, "bus", None),
        max_rounds=int(body.get("max_rounds", DEFAULT_MAX_ROUNDS)),
        max_repairs=int(body.get("max_repairs", DEFAULT_MAX_REPAIRS)),
    )
    try:
        return loop.run(str(task)).to_dict()
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc


@app.get("/api/events/recent")
def events_recent(n: int = Query(20, ge=1, le=200)):
    from marguerite_workbench.events import EventBus

    bus = getattr(app.state, "bus", None)
    if not isinstance(bus, EventBus):
        return {"events": []}
    return {"events": bus.recent(n)}


@app.get("/api/events/stream")
async def events_stream(request: Request):
    """Server-Sent Events feed of the live workbench event bus.

    Closes the SSE item declared NOT IMPLEMENTED in UPGRADE_PLAN_V2 §2.
    Cross-thread delivery uses the dependency-free PollingTap (sync agent-loop
    publishers → async generator); a ``: ping`` heartbeat every poll interval
    keeps proxies and the gateway from reaping idle connections.
    """
    import asyncio
    import json as json_mod

    from fastapi.responses import StreamingResponse

    from marguerite_workbench.events import EventBus, PollingTap

    bus = getattr(app.state, "bus", None)
    if not isinstance(bus, EventBus):
        raise HTTPException(status_code=503, detail="event bus not initialised")

    tap = PollingTap()
    bus.subscribe(tap.push)

    async def generator():
        try:
            yield ": connected\n\n"
            while True:
                if await request.is_disconnected():
                    break
                batch = await asyncio.to_thread(tap.wait, 1.0)
                if not batch:
                    yield ": ping\n\n"  # heartbeat
                    continue
                for evt in batch:
                    yield f"data: {json_mod.dumps(evt, ensure_ascii=False, default=str)}\n\n"
        finally:
            bus.unsubscribe(tap.push)
            tap.close()

    return StreamingResponse(generator(), media_type="text/event-stream", headers={
        "Cache-Control": "no-cache",
        "X-Accel-Buffering": "no",
    })


@app.get("/api/agent/tools")
def agent_tools():
    """Machine-readable manifest of the sandboxed tool registry."""
    from marguerite_workbench.agentloop import TOOL_REGISTRY

    return {
        "tools": [
            {"name": name, "required_args": list(spec.required), "safe_resolve_guarded": name in ("read_file", "list_dir")}
            for name, spec in sorted(TOOL_REGISTRY.items())
        ],
    }


if __name__ == "__main__":
    import uvicorn

    # Bind localhost by default; only expose on 0.0.0.0 via explicit opt-in.
    _host = os.environ.get("MARGERITE_BIND_HOST", "127.0.0.1")
    _port = int(os.environ.get("MARGERITE_BIND_PORT", "8011"))
    uvicorn.run(app, host=_host, port=_port)


# ---------------------------------------------------------------------------
# Single-port SPA hosting (packaged deployments).
# When a built webapp directory is present (env ``MARGERITE_WEBAPP_DIR`` or
# the conventional ``webapp/`` sibling of the package root) it is mounted as
# static files AFTER every API route, so /api, /docs and openapi.json keep
# priority and everything else resolves to the SPA.  This removes CORS from
# the packaged topology entirely — same-origin by construction — and keeps the
# loopback-only invariant (``serve`` still binds 127.0.0.1 by default).
# ---------------------------------------------------------------------------
ENV_WEBAPP_DIR = "MARGERITE_WEBAPP_DIR"


def mount_webapp(fastapi_app: FastAPI, directory: str | Path) -> bool:
    """Mount a built SPA at "/" when it contains index.html. Idempotent-safe."""
    from fastapi.staticfiles import StaticFiles

    webapp = Path(directory)
    if not (webapp / "index.html").is_file():
        logger.info("SPA hosting disabled: no index.html under %s", webapp)
        return False
    fastapi_app.mount("/", StaticFiles(directory=str(webapp), html=True), name="webapp")
    logger.info("SPA hosting enabled: %s", webapp)
    return True


_default_webapp_dir = os.environ.get(ENV_WEBAPP_DIR) or (
    Path(__file__).resolve().parent.parent / "webapp"
)
mount_webapp(app, _default_webapp_dir)
