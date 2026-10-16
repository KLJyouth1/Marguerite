"""API contract tests for the UPGRADE_PLAN_V2 pillars (LLMLab / TTC / Agent)."""

from __future__ import annotations

import math

import pytest


@pytest.fixture()
def client(_isolated_env):
    from fastapi.testclient import TestClient

    from marguerite_workbench.api import app

    with TestClient(app) as test_client:
        yield test_client


def test_llmlab_tokenize_roundtrip(client):
    resp = client.post("/api/llmlab/tokenize", json={"text": "hello transformer", "vocab_size": 264})
    assert resp.status_code == 200
    body = resp.json()
    assert body["decoded_roundtrip_ok"] is True
    # BPE only merges: token count can never exceed the raw byte length.
    assert body["n_tokens"] <= len("hello transformer".encode())
    assert body["compression"] >= 1.0
    assert body["trace"]


def test_llmlab_forward_returns_real_attention(client):
    resp = client.post("/api/llmlab/forward", json={"text": "attention is all you need", "dim": 16,
                                                    "n_heads": 2, "n_layers": 2})
    assert resp.status_code == 200
    body = resp.json()
    assert body["loss"] is not None and math.isfinite(body["loss"])
    assert len(body["attention"]) == 2          # two layers requested
    layer0 = body["attention"][0]
    head0 = layer0[0]
    for i, row in enumerate(head0):
        assert abs(sum(row) - 1.0) < 1e-3       # causal probability rows sum to ~1
        assert len(row) == min(i + 1, len(head0)) or len(row) <= i + 1


def test_ttc_budget_endpoint_difficulty(client):
    resp = client.get("/api/ttc/budget", params={"task": "prove concurrency safety of the design"})
    assert resp.status_code == 200
    body = resp.json()
    assert body["difficulty"] == "high"
    assert body["max_rounds"] >= 8


def test_agent_run_and_event_stream(client, workspace_dir):
    import pathlib

    pathlib.Path(workspace_dir, "note.txt").write_text("workspace note", encoding="utf-8")
    resp = client.post("/api/agent/run", json={"task": "read note.txt"})
    assert resp.status_code == 200
    run = resp.json()
    assert run["status"] == "completed"
    tools_used = [s["tool"] for s in run["steps"]]
    assert "read_file" in tools_used
    assert run["session_id"]

    recent = client.get("/api/events/recent", params={"n": 10})
    assert recent.status_code == 200
    types = [e["type"] for e in recent.json()["events"]]
    assert "run_started" in types and "FINALIZE" in types


def test_agent_tools_manifest(client):
    resp = client.get("/api/agent/tools")
    assert resp.status_code == 200
    names = {t["name"] for t in resp.json()["tools"]}
    assert {"read_file", "list_dir", "kb_search", "memory_query"} <= names
    guarded = {t["name"]: t["safe_resolve_guarded"] for t in resp.json()["tools"]}
    assert guarded["read_file"] is True and guarded["kb_search"] is False


def test_llm_parse_dual_format_thinking():
    from marguerite_workbench.llm import parse_chat_response

    deepseek = {"choices": [{"message": {
        "content": "final answer",
        "reasoning_content": "step 1… step 2…",
    }}]}
    openai_alias = {"choices": [{"message": {
        "content": "final answer",
        "reasoning": "aliased trace",
    }}]}
    plain = {"choices": [{"message": {"content": "just content"}}]}
    malformed = {"oops": True}

    assert parse_chat_response(deepseek) == ("final answer", "step 1… step 2…")
    assert parse_chat_response(openai_alias) == ("final answer", "aliased trace")
    assert parse_chat_response(plain) == ("just content", None)
    with pytest.raises(ValueError):
        parse_chat_response(malformed)


def test_polling_tap_bridge_and_close():
    from threading import Thread

    from marguerite_workbench.events import PollingTap

    tap = PollingTap()
    # Simulate a sync publisher thread pushing while consumer waits.
    def publish():
        for i in range(3):
            tap.push({"seq": i})

    worker = Thread(target=publish)
    worker.start()
    batch = tap.wait(timeout=2.0)
    worker.join()
    assert [e["seq"] for e in batch] == [0, 1, 2]
    assert tap.wait(timeout=0.05) == []          # drained → timeout drives heartbeat
    tap.close()
    tap.push({"seq": 99})                        # closed tap ignores late pushes
    assert tap.wait(timeout=0.05) == []


def test_mount_webapp_serves_spa(tmp_path):
    from fastapi import FastAPI

    from marguerite_workbench.api import mount_webapp

    (tmp_path / "assets").mkdir()
    (tmp_path / "index.html").write_text("<html><body>spa</body></html>", encoding="utf-8")

    fresh = FastAPI()

    @fresh.get("/api/health")
    def health():
        return {"ok": True}

    assert mount_webapp(fresh, tmp_path) is True
    from fastapi.testclient import TestClient

    with TestClient(fresh) as c:
        assert c.get("/").text == "<html><body>spa</body></html>"
        assert c.get("/api/health").json() == {"ok": True}, "API routes keep priority over the SPA mount"

    empty = FastAPI()
    assert mount_webapp(empty, tmp_path / "nope") is False, "missing index.html must disable hosting"
