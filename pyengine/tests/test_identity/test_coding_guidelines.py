from __future__ import annotations

from pathlib import Path

from opensquilla.identity.coding_guidelines import (
    _find_docs_dir,
    _summarize_sections,
    load_coding_guidelines,
)
from opensquilla.identity.prompt import assemble_system_prompt
from opensquilla.identity.types import AgentProfile, AgentIdentity


def test_summarize_sections_keeps_principle_paragraphs() -> None:
    text = (
        "# 通用后端开发规范 v1.1\n\n"
        "## 1. 离线优先\n\n"
        "**原则**：核心判定逻辑必须离线可运行；在线能力作为可插拔层。\n\n"
        "落地点：\n"
        "1. 纯规则引擎 vs 在线后端分离\n"
        "2. 依赖注入而非硬编码\n\n"
        "**Marguerite 案例**：`Orchestrator::analyze()` 是纯规则引擎。\n\n"
        "## 2. 能力注册表\n\n"
        "**原则**：任何对外宣称的能力必须在单一注册表登记。\n"
    )
    summary = _summarize_sections(text, max_chars=600)
    assert "离线优先" in summary
    assert "核心判定逻辑必须离线可运行" in summary
    assert "能力注册表" in summary
    assert "Orchestrator::analyze" not in summary  # 案例细节被裁剪


def test_find_docs_dir_prefers_env_override(tmp_path, monkeypatch) -> None:
    docs = tmp_path / "docs"
    docs.mkdir()
    (docs / "后端开发规范_v1.1.md").write_text("# 后端规范\n", encoding="utf-8")
    monkeypatch.setenv("MARGUERITE_CODING_GUIDELINES_DIR", str(docs))
    found = _find_docs_dir()
    assert found is not None
    assert found.resolve() == docs.resolve()


def test_load_coding_guidelines_returns_none_without_docs(tmp_path, monkeypatch) -> None:
    monkeypatch.chdir(tmp_path)
    monkeypatch.delenv("MARGUERITE_CODING_GUIDELINES_DIR", raising=False)
    assert load_coding_guidelines() is None


def test_load_coding_guidelines_merges_backend_and_frontend(tmp_path, monkeypatch) -> None:
    docs = tmp_path / "docs"
    docs.mkdir()
    (docs / "后端开发规范_v1.1.md").write_text(
        "# 后端\n\n## 1. 离线优先\n\n**原则**：后端核心判定必须离线可运行。\n",
        encoding="utf-8",
    )
    (docs / "前端开发规范_v1.1.md").write_text(
        "# 前端\n\n## 1. 渲染与逻辑分离\n\n**原则**：前端只做渲染。\n",
        encoding="utf-8",
    )
    monkeypatch.setenv("MARGUERITE_CODING_GUIDELINES_DIR", str(docs))
    result = load_coding_guidelines()
    assert result is not None
    assert "后端开发规范" in result
    assert "前端开发规范" in result
    assert "离线可运行" in result
    assert "只做渲染" in result


def test_system_prompt_renders_coding_guidelines_only_in_full_mode() -> None:
    profile = AgentProfile(agent_id="main", identity=AgentIdentity(name=None), prompt_mode="full")
    with_guidelines = assemble_system_prompt(profile, coding_guidelines="遵循离线优先规范。")
    assert "## Coding Guidelines" in with_guidelines
    assert "遵循离线优先规范" in with_guidelines

    minimal = AgentProfile(agent_id="main", identity=AgentIdentity(name=None), prompt_mode="minimal")
    without = assemble_system_prompt(minimal, coding_guidelines="遵循离线优先规范。")
    assert "## Coding Guidelines" not in without


def test_system_prompt_omits_section_when_not_provided() -> None:
    profile = AgentProfile(agent_id="main", identity=AgentIdentity(name=None), prompt_mode="full")
    prompt = assemble_system_prompt(profile)
    assert "## Coding Guidelines" not in prompt
