"""End-to-end pytest suite for the Marguerite File Intelligence Workbench.

Runs fully offline with NO API keys (every LLM/network path degrades to its
deterministic fallback).  Spins up the FastAPI app via ``TestClient`` so the
real endpoints (and their lifetimes) are exercised.

Includes the 16 baseline tests plus the physical-bridge tests mandated by the
native-integration task (``CoreBridge`` Local fallback, ``/harness/capabilities``,
L3 store/query roundtrip, and CoreSqlite native discovery).
"""

from __future__ import annotations

import asyncio
import tempfile
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from marguerite_workbench.api import app
from marguerite_workbench.core_bridge import Backend, CoreBridge


@pytest.fixture
def client(_isolated_env):
    with TestClient(app) as c:
        yield c


# ---------------------------------------------------------------------------
# Health + File load/display/modify/collapse/ask/interact
# ---------------------------------------------------------------------------
def test_health(client):
    r = client.get("/api/health")
    assert r.status_code == 200
    assert r.json()["status"] == "ok"


def test_safe_resolve_blocks_symlink_escape():
    """P0-1 (security audit): a symlink inside the workspace pointing OUT must
    be rejected, not just string-prefix checked. Mirrors the gap in the live
    ``core/src/tools/fs_path.rs`` that this workbench must not repeat."""
    import os
    import tempfile

    from marguerite_workbench.security import safe_resolve

    root = tempfile.mkdtemp()
    outside = tempfile.mkdtemp()
    # traversal + absolute escapes (no symlink perms needed)
    for bad in ("../../../../etc/passwd", os.path.join(outside, "x")):
        with pytest.raises(ValueError):
            safe_resolve(root, bad)
    # legit inside path resolves fine
    assert safe_resolve(root, "sub/file.txt")

    # symlink escape (best-effort; skip if host lacks symlink perms)
    link = os.path.join(root, "escape_link")
    try:
        os.symlink(outside, link)
    except (OSError, NotImplementedError, AttributeError):
        pytest.skip("host cannot create symlinks")
    with pytest.raises(ValueError):
        safe_resolve(root, "escape_link/x")


def test_load_file_splits_sections(client, sample_file):
    r = client.post("/api/files/load", json={"path": sample_file})
    assert r.status_code == 200
    card = r.json()
    assert card["language"] == "python"
    assert card["sha256"]
    assert card["size_bytes"] > 0
    kinds = {s["kind"] for s in card["sections"]}
    assert "function" in kinds and "class" in kinds
    # every section has a preview and (when not collapsed) full text
    assert all(s["preview"] for s in card["sections"])
    assert all(s["full"] for s in card["sections"])


def test_display_returns_highlighted_html(client, sample_file):
    load = client.post("/api/files/load", json={"path": sample_file}).json()
    fid = load["file_id"]
    r = client.get(f"/api/files/{fid}")
    assert r.status_code == 200
    body = r.json()
    assert body["language"] == "python"
    assert "<pre" in body["html"]


def test_modify_fallback_rule_edit(client, sample_file):
    load = client.post("/api/files/load", json={"path": sample_file}).json()
    fid = load["file_id"]
    r = client.post(
        f"/api/files/{fid}/modify",
        json={"instruction": 'replace "def add(a, b):" with "def add_numbers(a, b):"'},
    )
    assert r.status_code == 200
    mod = r.json()
    assert mod["applied"] is True
    assert "add_numbers" in mod["new_content"]
    assert mod["backup_ref"] is not None
    assert "def add(" in mod["diff"]  # unified diff captured the change


def test_collapse_targets_sections(client, sample_file):
    load = client.post("/api/files/load", json={"path": sample_file}).json()
    fid = load["file_id"]
    target = load["sections"][0]["id"]
    r = client.post(f"/api/files/{fid}/collapse", json={"section_ids": [target]})
    assert r.status_code == 200
    card = r.json()
    collapsed = {s["id"]: s for s in card["sections"]}
    assert collapsed[target]["collapsed"] is True
    assert collapsed[target]["full"] is None
    # other sections stay expanded
    others = [s for s in card["sections"] if s["id"] != target]
    assert all(not s["collapsed"] for s in others)


def test_ask_returns_answer_and_citations(client, sample_file):
    load = client.post("/api/files/load", json={"path": sample_file}).json()
    fid = load["file_id"]
    r = client.post(f"/api/files/{fid}/ask", json={"question": "how does add work?"})
    assert r.status_code == 200
    ans = r.json()
    assert ans["answer"]
    assert isinstance(ans["citations"], list)


def test_interact_multi_turn_uses_session(client, sample_file):
    load = client.post("/api/files/load", json={"path": sample_file}).json()
    fid = load["file_id"]
    sid = "sess-abc"
    r1 = client.post(f"/api/files/{fid}/interact", json={"message": "explain add", "session_id": sid})
    assert r1.status_code == 200
    a1 = r1.json()
    assert a1["reply"]
    assert a1["session_id"] == sid
    r2 = client.post(f"/api/files/{fid}/interact", json={"message": "and subtract?", "session_id": sid})
    a2 = r2.json()
    assert a2["reply"]
    assert a2["session_id"] == sid
    # L2 short-term memory should hold the exchange
    mem = client.get("/api/memory/L2/query", params={"q": "explain add", "k": 5}).json()
    assert any("explain add" in it["content"] for it in mem["items"])


# ---------------------------------------------------------------------------
# Memory (7 layers)
# ---------------------------------------------------------------------------
def test_memory_layers_reports_seven(client):
    r = client.get("/api/memory/layers")
    assert r.status_code == 200
    layers = r.json()
    assert len(layers) == 7
    assert {l["layer"] for l in layers} == {f"L{i}" for i in range(1, 8)}
    # capacities are positive integers (named constants, anti R5)
    assert all(l["capacity"] > 0 for l in layers)


def test_memory_store_and_query_each_layer(client):
    for i in range(1, 8):
        layer = f"L{i}"
        content = f"zz{i} project memory note about {layer} workspace task"
        store = client.post(f"/api/memory/{layer}/store",
                            json={"layer": layer, "kind": "test", "content": content})
        assert store.status_code == 200
        assert store.json()["id"]
        q = client.get(
            f"/api/memory/{layer}/query",
            params={"q": f"zz{i} project memory note about {layer}", "k": 3},
        ).json()
        assert any(f"zz{i}" in it["content"] for it in q["items"]), f"L{i} query missed"


# ---------------------------------------------------------------------------
# Knowledge base
# ---------------------------------------------------------------------------
def test_kb_ingest_and_search(client):
    text = (
        "Rust emphasizes memory safety without a garbage collector. "
        "Go is simple and concurrent. Python is dynamic and readable."
    )
    ingest = client.post("/api/kb/ingest", json={"text": text, "doc_id": "langs"})
    assert ingest.status_code == 200
    assert ingest.json()["indexed_chunks"] >= 1
    r = client.get("/api/kb/search", params={"q": "memory safety systems programming", "k": 3})
    assert r.status_code == 200
    hits = r.json()
    assert hits
    assert hits[0]["score"] > 0


# ---------------------------------------------------------------------------
# Experts
# ---------------------------------------------------------------------------
def test_experts_list_includes_core_roles(client):
    r = client.get("/api/experts")
    assert r.status_code == 200
    roles = r.json()
    ids = [x["role"] for x in roles]
    # 核心 5 角色是产品契约的一部分，必须始终存在。
    # 不再断言「总数 == 5」：`experts.py` 已把 8 个专家团的 43 个团队角色合并进注册表
    # （见 experts.py 的 “Team roles — the eight expert teams” 注释），总数会随团队扩充而增长。
    core = {"product_reviewer", "security_officer", "qa_lead", "designer", "investigator"}
    assert core <= set(ids), f"缺少核心角色: {core - set(ids)}"
    assert len(ids) == len(set(ids)), "角色 id 必须唯一（合并团队时不得重名）"
    # templates + skills are real, not empty labels
    for role in roles:
        assert role["problem_solving_template"].strip()
        assert role["skills"]


def test_experts_analyze_security_finds_eval(client):
    r = client.post(
        "/api/experts/security_officer/analyze",
        json={"payload": {"code": "x = eval('1+1')\ny = os.system('ls')"}},
    )
    assert r.status_code == 200
    a = r.json()
    assert a["role"] == "security_officer"
    assert a["verdict"] in ("pass", "conditional", "fail")
    blob = " ".join(a["findings"]).lower()
    assert "eval" in blob
    assert a["confidence"] > 0


# ---------------------------------------------------------------------------
# Harness
# ---------------------------------------------------------------------------
def test_harness_adapters_listed(client):
    r = client.get("/api/harness/adapters")
    assert r.status_code == 200
    adapters = r.json()
    ids = {a["id"] for a in adapters}
    assert {"codex_cli", "opencode", "aider", "internal"} <= ids
    # internal is always available
    assert next(a for a in adapters if a["id"] == "internal")["available"] is True
    # best_practices carried through
    assert any(a["best_practices"] for a in adapters)


def test_harness_dispatch_internal_loop(client, sample_file):
    r = client.post(
        "/api/harness/dispatch",
        json={"task": f'edit file {sample_file} replace "def add" with "def sum"',
              "adapter": "internal"},
    )
    assert r.status_code == 200
    res = r.json()
    assert res["adapter"] == "internal"
    assert res["steps"]  # plan/act/observe produced steps
    assert res["status"] in ("ok", "partial", "error")
    assert res["duration_ms"] >= 0


# ---------------------------------------------------------------------------
# Research (联网) — offline fallback
# ---------------------------------------------------------------------------
def test_research_offline_fallback(client):
    r = client.post("/api/research", json={"query": "rust async runtime"})
    assert r.status_code == 200
    res = r.json()
    assert res["query"] == "rust async runtime"
    assert "离线" in res["synthesized"] or res["results"] == []


# ---------------------------------------------------------------------------
# 物理桥接（core_bridge）：Local 降级 + 能力端点 + L3 读写往返 + 原生发现
# ---------------------------------------------------------------------------
def test_core_bridge_fallback(tmp_path):
    """无核心 store 时，桥接必须降级到 Local 后端并正常工作。"""
    # 显式传入不存在的 home，确保不接入任何核心 store。
    bridge = CoreBridge(data_dir=tmp_path, marguerite_home=tmp_path / "no_home")
    try:
        # 后端应为 Local（而非 CoreSqlite）。
        assert bridge.active_backend() == Backend.LOCAL
        # Local 模式下 store/query 必须可用。
        stored = asyncio.run(bridge.store(
            "L3", {"kind": "note", "content": "fallback scratch task alpha"}
        ))
        assert stored["id"]
        assert stored["layer"] == "L3"
        hits = asyncio.run(bridge.query("L3", "fallback scratch task alpha", k=3))
        assert any("alpha" in h["content"] for h in hits)
        # capabilities 必含关键字段且可 JSON 序列化。
        caps = bridge.capabilities()
        assert caps["memory_backend"]["active"] == "local"
        assert caps["memory_backend"]["fallback"] is True
        assert "layers" in caps and "native_capabilities" in caps
    finally:
        # Windows 下释放 SQLite 文件句柄，便于 tmp_path 延迟清理。
        try:
            (tmp_path / "no_home" / "core_bridge.db").unlink(missing_ok=True)
        except OSError:
            pass


def test_capabilities_endpoint(client):
    """GET /harness/capabilities 必须返回预期键。"""
    r = client.get("/harness/capabilities")
    assert r.status_code == 200
    caps = r.json()
    # 规范契约键。
    for key in ("service", "native_capabilities", "memory_backend", "layers"):
        assert key in caps, f"missing capability key: {key}"
    # 后端声明键。
    mb = caps["memory_backend"]
    for key in ("active", "is_core_store", "fallback", "db_path"):
        assert key in mb, f"missing memory_backend key: {key}"
    # 7 层齐全。
    assert set(caps["layers"].keys()) == {f"L{i}" for i in range(1, 8)}
    # 原生能力齐全。
    assert caps["native_capabilities"]["memory_7layer"] is True
    # 后端取值必须是 local / core 两端之一。
    assert caps["memory_backend"]["active"] in ("local", "core")


def test_memory_store_query_roundtrip(client, tmp_path):
    """在 L3 写入并查询回读（异步 API + 同步 API + HTTP 端点三路径）。"""
    bridge = CoreBridge(data_dir=tmp_path, marguerite_home=tmp_path / "nh")
    try:
        stored = asyncio.run(bridge.store(
            "L3", {"kind": "scratch", "content": "roundtrip working memory item beta"}
        ))
        assert stored["layer"] == "L3"
        # 异步查询
        ahits = asyncio.run(bridge.query("L3", "roundtrip working memory item beta", k=5))
        assert any("beta" in h["content"] for h in ahits)
        # 同步查询（MemoryEngine 委托路径）
        shits = bridge.query_sync("L3", "roundtrip working memory item beta", k=5)
        assert any("beta" in h["content"] for h in shits)
    finally:
        try:
            (tmp_path / "nh" / "core_bridge.db").unlink(missing_ok=True)
        except OSError:
            pass
    # 经由既有 HTTP 端点的路径同样可用。
    r = client.post(
        "/api/memory/L3/store",
        json={"layer": "L3", "kind": "note", "content": "http roundtrip gamma working"},
    )
    assert r.status_code == 200
    q = client.get(
        "/api/memory/L3/query",
        params={"q": "http roundtrip gamma working", "k": 3},
    ).json()
    assert any("gamma" in it["content"] for it in q["items"])


def test_core_bridge_native_connects_to_discovered_store(tmp_path, monkeypatch):
    """CoreSqlite 模式：发现核心 sessions.db 后，记忆真的写进同一块物理库。"""
    import sqlite3 as _sql

    # 造一个假的 Marguerite home，内含核心 sessions.db（带 sessions 表）。
    home = tmp_path / "fake_marguerite"
    home.mkdir()
    core_db = home / "sessions.db"
    with _sql.connect(core_db) as c:
        c.execute("CREATE TABLE sessions (session_id TEXT PRIMARY KEY, summary TEXT)")

    monkeypatch.setenv("MARGUERITE_HOME", str(home))

    bridge = CoreBridge(data_dir=tmp_path / "data")
    assert bridge.active_backend() == Backend.CORE_SQLITE
    assert bridge.db_path == core_db

    asyncio.run(bridge.store(
        "L5", {"kind": "fact", "content": "native bridge fact zeta about marguerite core store"}
    ))
    back = asyncio.run(bridge.query(
        "L5", "native bridge fact zeta about marguerite core store", k=3
    ))
    assert any("zeta" in it["content"] for it in back)

    # 物理证据：直接打开核心库，确认 memory_items 表真的被写进去了。
    with _sql.connect(core_db) as c:
        rows = c.execute(
            "SELECT content FROM memory_items WHERE layer='L5'"
        ).fetchall()
    assert any("zeta" in r[0] for r in rows)
