"""Offline CLI tests — exercise ``marguerite_workbench.cli.main`` in-process.

Every test runs against the hermetic fixtures from ``conftest.py`` (temp
workspace + data dir, no API keys), mirroring the API test suite's discipline:
deterministic, key-free, no network.
"""

from __future__ import annotations

import json

import pytest

from marguerite_workbench.cli import (
    EXIT_NOT_FOUND,
    EXIT_OK,
    EXIT_REJECTED,
    EXIT_RUNTIME,
    EXIT_USAGE,
    main,
)


def run_json(capsys, *argv: str) -> dict:
    """Run the CLI with --json and return the decoded success envelope."""
    code = main(["--json", *argv])
    out = capsys.readouterr().out
    assert out.strip(), "expected JSON on stdout"
    envelope = json.loads(out)
    assert envelope["ok"] is True
    assert set(envelope) == {"ok", "data", "meta"}
    assert "cmd" in envelope["meta"] and "duration_ms" in envelope["meta"]
    assert code == EXIT_OK
    return envelope


# ---------------------------------------------------------------------------
# doctor
# ---------------------------------------------------------------------------
def test_doctor_json_reports_checks(capsys):
    envelope = run_json(capsys, "doctor")
    checks = envelope["data"]["checks"]
    names = {c["name"] for c in checks}
    assert {"python", "dependencies", "workspace_dir", "data_dir", "memory_bridge"} <= names
    assert envelope["data"]["healthy"] is True
    statuses = {c["status"] for c in checks}
    assert statuses <= {"ok", "warn"}, "isolated env must not produce failing probes"


def test_doctor_text_uses_ascii_markers(capsys):
    assert main(["doctor"]) == EXIT_OK
    out = capsys.readouterr().out
    assert "[OK]" in out
    assert "\U0001f300" not in out  # no emoji anywhere in CLI output


# ---------------------------------------------------------------------------
# knowledge base
# ---------------------------------------------------------------------------
def test_kb_ingest_and_search_roundtrip(capsys):
    run_json(capsys, "kb", "ingest", "--text", "Marguerite is a terminal-first agent workbench.", "--doc-id", "doc_intro")
    envelope = run_json(capsys, "kb", "search", "--q", "terminal agent workbench")
    hits = envelope["data"]
    assert len(hits) >= 1
    assert hits[0]["score"] > 0
    assert hits[0]["source"] == "doc_intro"


def test_kb_ingest_requires_text_or_path(capsys):
    code = main(["kb", "ingest"])
    captured = capsys.readouterr()
    assert code == EXIT_USAGE
    assert "provide --text or --path" in captured.err


# ---------------------------------------------------------------------------
# memory
# ---------------------------------------------------------------------------
def test_memory_store_and_query(capsys):
    envelope = run_json(capsys, "memory", "store", "L3", "--content", "user prefers concise answers", "--kind", "preference", "--ref", "session-1")
    assert envelope["data"]["layer"] == "L3"
    assert envelope["data"]["id"]

    found = run_json(capsys, "memory", "query", "L3", "--q", "concise answers")
    assert any("concise" in item["content"] for item in found["data"]["items"])


def test_memory_query_unknown_layer_rejected(capsys):
    code = main(["--json", "memory", "query", "L9", "--q", "anything"])
    captured = capsys.readouterr()
    assert code == EXIT_REJECTED
    err = json.loads(captured.err)
    assert err["ok"] is False
    assert err["error"]["exit_code"] == EXIT_REJECTED


def test_memory_layers_lists_seven(capsys):
    envelope = run_json(capsys, "memory", "layers")
    assert len(envelope["data"]) == 7


# ---------------------------------------------------------------------------
# files
# ---------------------------------------------------------------------------
def test_files_load_display_ask_roundtrip(capsys, sample_file):
    envelope = run_json(capsys, "files", "load", "--path", sample_file)
    card = envelope["data"]
    assert card["path"].endswith("sample.py")
    assert card["language"] == "python"
    assert len(card["sections"]) >= 3
    file_id = card["file_id"]

    main(["files", "display", "--id", file_id])
    text_out = capsys.readouterr().out
    assert "def add" in text_out, "text mode renders de-emphasised source"

    asked = run_json(capsys, "files", "ask", "--id", file_id, "--question", "where is division by zero handled")
    assert asked["data"]["answer"]
    assert isinstance(asked["data"]["citations"], list)


def test_files_display_unknown_id_not_found(capsys):
    code = main(["files", "display", "--id", "nope"])
    captured = capsys.readouterr()
    assert code == EXIT_NOT_FOUND
    assert "unknown file_id" in captured.err.lower()


def test_files_load_rejects_path_escape(capsys):
    code = main(["files", "load", "--path", "../outside.txt"])
    captured = capsys.readouterr()
    assert code == EXIT_REJECTED
    assert captured.err.strip(), "rejections must surface on stderr"


# ---------------------------------------------------------------------------
# experts
# ---------------------------------------------------------------------------
def test_experts_list_has_five_roles(capsys):
    envelope = run_json(capsys, "experts", "list")
    roles = {r["role"] for r in envelope["data"]}
    assert len(roles) == 5
    assert "security_officer" in roles


def test_experts_analyze_security_finds_eval(capsys):
    envelope = run_json(
        capsys, "experts", "analyze", "security_officer",
        "--payload", json.dumps({"code": "eval(user_input)", "language": "python"}),
    )
    analysis = envelope["data"]
    assert analysis["verdict"] == "fail"
    assert analysis["findings"]


def test_experts_analyze_unknown_role_exit_3(capsys):
    code = main(["experts", "analyze", "no_such_role", "--payload", "{}"])
    captured = capsys.readouterr()
    assert code == EXIT_NOT_FOUND
    assert "unknown expert role" in captured.err


# ---------------------------------------------------------------------------
# harness
# ---------------------------------------------------------------------------
def test_harness_adapters_and_capabilities(capsys):
    envelope = run_json(capsys, "harness", "adapters")
    ids = {a["id"] for a in envelope["data"]}
    assert "internal" in ids

    caps = run_json(capsys, "harness", "capabilities")
    assert caps["data"]["service"] == "marguerite-file-intel"
    assert caps["data"]["native_capabilities"]["expert_count"] == 5


def test_harness_dispatch_internal_loop(capsys, sample_file):
    envelope = run_json(
        capsys, "harness", "dispatch",
        "--adapter", "internal",
        f"inspect {sample_file} and summarise its structure",
    )
    result = envelope["data"]
    assert result["adapter"] == "internal"
    assert result["status"] in {"ok", "partial", "error"}
    assert result["steps"]


# ---------------------------------------------------------------------------
# research (offline fallback)
# ---------------------------------------------------------------------------
def test_research_offline_uses_local_kb(capsys):
    run_json(capsys, "kb", "ingest", "--text", "Bloom mode gates every deliverable through five quality levels.")
    envelope = run_json(capsys, "research", "Bloom mode quality gates")
    assert envelope["data"]["query"] == "Bloom mode quality gates"
    assert isinstance(envelope["data"]["results"], list)


# ---------------------------------------------------------------------------
# usage errors
# ---------------------------------------------------------------------------
def test_unknown_command_is_usage_error():
    with pytest.raises(SystemExit) as excinfo:
        main(["no_such_command"])
    assert excinfo.value.code == EXIT_USAGE


def test_files_modify_requires_known_file(capsys):
    code = main(["files", "modify", "--id", "ghost", "--instruction", "rename variables"])
    assert code == EXIT_NOT_FOUND
