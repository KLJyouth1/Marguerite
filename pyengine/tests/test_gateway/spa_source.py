"""Locate this repository's Vue console for the gateway static-source contracts.

``pyengine/`` was absorbed from an upstream Python project whose console lived in
``opensquilla-webui/``. That directory was never migrated: the console this
repository actually builds, tests and serves is ``webui/`` — a sibling of
``pyengine/`` with its own vitest suite and Playwright e2e specs. The contracts in
this directory pin the *served* console, so they must read ``webui/``; hardcoding
the upstream directory name made every one of them fail with ``FileNotFoundError``
while still looking like a passing tree (R58).

Resolving the root in one place also keeps the next rename from touching seven
files again.
"""

from __future__ import annotations

from pathlib import Path

# tests/test_gateway/spa_source.py -> (test_gateway, tests, pyengine, <repo root>)
SPA_ROOT = Path(__file__).resolve().parents[3] / "webui"


def spa(*parts: str) -> Path:
    """Return a path inside the Vue console source tree."""

    return SPA_ROOT.joinpath(*parts)
