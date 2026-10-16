"""Pytest fixtures.

Isolates the workbench into a temporary workspace + data dir (via the
``MARGERITE_*`` env vars the modules read at call-time) so the suite is
hermetic, offline, and key-free.  Also seeds a sample source file.
"""

from __future__ import annotations

import os
import tempfile

import pytest


@pytest.fixture(autouse=True)
def _isolated_env(tmp_path, monkeypatch):
    """Point the workbench at a fresh temp workspace/data dir for every test."""
    workspace = tmp_path / "workspace"
    data = tmp_path / "data"
    workspace.mkdir()
    data.mkdir()
    monkeypatch.setenv("MARGERITE_WORKSPACE", str(workspace))
    monkeypatch.setenv("MARGERITE_DATA_DIR", str(data))
    # 让记忆桥接在隔离环境中回退到本地库（Local），不触碰真实 ~/.marguerite。
    monkeypatch.setenv("MARGUERITE_HOME", str(tmp_path / "no_marguerite_home"))
    # Ensure no real API keys leak into the deterministic test run.
    for var in ("UNIV_LLM_BASE_URL", "UNIV_LLM_KEY", "SERPER_API_KEY", "UNIV_SEARCH_KEY"):
        monkeypatch.delenv(var, raising=False)
    yield  # pytest's tmp_path fixture cleans up the tree automatically.


@pytest.fixture
def workspace_dir():
    return os.environ["MARGERITE_WORKSPACE"]


@pytest.fixture
def sample_file(workspace_dir):
    """Write a representative Python sample file and return its relative path."""
    path = os.path.join(workspace_dir, "sample.py")
    content = (
        "def add(a, b):\n"
        '    """Return the sum of two numbers."""\n'
        "    return a + b\n"
        "\n"
        "def subtract(a, b):\n"
        "    return a - b\n"
        "\n"
        "class Calculator:\n"
        "    def multiply(self, a, b):\n"
        "        return a * b\n"
        "\n"
        "    def divide(self, a, b):\n"
        "        if b == 0:\n"
        "            raise ValueError('division by zero')\n"
        "        return a / b\n"
    )
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(content)
    return "sample.py"


@pytest.fixture
def markdown_file(workspace_dir):
    path = os.path.join(workspace_dir, "doc.md")
    content = (
        "# Title\n"
        "Intro paragraph about the project.\n"
        "\n"
        "## Installation\n"
        "Run `pip install`.\n"
        "\n"
        "## Usage\n"
        "Call the API and read the response.\n"
    )
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(content)
    return "doc.md"
