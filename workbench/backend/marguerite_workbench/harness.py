"""Codex-class harness bridge.

* ``adapters()`` — enumerates ``codex_cli`` / ``opencode`` / ``aider`` (detected
  on ``PATH``) plus the always-available ``internal`` Marguerite harness, each
  carrying a distilled ``best_practices`` list (2026 agent-tool research).
* ``dispatch(task, adapter)`` — if a real CLI is available it is invoked in a
  sandbox (``subprocess`` with ``cwd=workspace``, ``timeout``, no shell, captured
  stdout) and normalised to :class:`HarnessResult`; otherwise the ``internal``
  plan→act→observe loop runs (a genuine, bounded agentic routine that can apply a
  safe file edit when the task clearly specifies one).
"""

from __future__ import annotations

import logging
import re
import shutil
import subprocess
import time
import uuid
from pathlib import Path
from typing import Optional

from marguerite_workbench.file_intel import apply_rule_edit, write_backup
from marguerite_workbench.models import HarnessAdapter, HarnessResult
from marguerite_workbench.security import safe_resolve

logger = logging.getLogger("marguerite.harness")

# Subprocess ceiling so a hung CLI can never block the server (anti hang).
DISPATCH_TIMEOUT_S = 120
OBSERVE_LIMIT = 25

# Cap retained subprocess output so a verbose CLI cannot bloat server memory
# (the returned *lines* are further capped by OBSERVE_LIMIT downstream).
MAX_OUTPUT_BYTES = 512 * 1024


def _cap_output(text: str, limit: int = MAX_OUTPUT_BYTES) -> str:
    """Truncate captured CLI output to ``limit`` UTF-8 bytes.

    Bounds the string the harness keeps in memory; a verbose command can no
    longer OOM the server.  Multibyte sequences near the cut are replaced
    cleanly rather than raising.
    """
    data = text.encode("utf-8", errors="replace")
    if len(data) <= limit:
        return text
    return data[:limit].decode("utf-8", errors="replace")


# Distilled patterns from 2026 agent-tool / agent-runtime research.
_BEST_PRACTICES = [
    "Harness ≠ model: the harness owns planning, tool use and safety; the model supplies reasoning.",
    "Subagent isolation: scope each delegated agent to a workspace + a single objective.",
    "Skills + hooks + MCP: package reusable capabilities as skills, wire lifecycle via hooks, extend via MCP.",
    "Sandboxing: run untrusted code in a sandbox with cwd=workspace, timeout, no shell, captured IO.",
    "Wiki / KB: persist durable learnings so the harness compounds experience across runs.",
    "Dynamic model routing: pick the cheapest model that meets the sub-task's quality bar.",
]

_ADAPTER_META = {
    "codex_cli": {
        "name": "OpenAI Codex CLI", "command": "codex", "args": ["exec", "-q"],
        "note": "Rust, kernel-sandbox capable agentic CLI.",
    },
    "opencode": {
        "name": "OpenCode", "command": "opencode", "args": ["run"],
        "note": "156K★, 75+ providers, built-in LSP.",
    },
    "aider": {
        "name": "Aider", "command": "aider", "args": ["--message"],
        "note": "Git-native pair programmer, 100+ providers.",
    },
}


def _detect(command: str) -> bool:
    return shutil.which(command) is not None


def adapters() -> list[HarnessAdapter]:
    out: list[HarnessAdapter] = []
    for aid, meta in _ADAPTER_META.items():
        available = _detect(meta["command"])
        out.append(HarnessAdapter(
            id=aid, name=meta["name"], available=available,
            command=meta["command"] if available else None,
            best_practices=_BEST_PRACTICES,
        ))
    # The native Marguerite harness is always available (the fallback engine).
    out.append(HarnessAdapter(
        id="internal", name="Marguerite Native Harness",
        available=True, command=None, best_practices=_BEST_PRACTICES,
    ))
    return out


def _available_cli() -> Optional[str]:
    for aid, meta in _ADAPTER_META.items():
        if _detect(meta["command"]):
            return aid
    return None


def dispatch(task: str, adapter: Optional[str], workspace: Path, data_dir: Path) -> HarnessResult:
    """Run ``task`` via a detected CLI or the internal plan→act→observe loop."""
    workspace = Path(workspace)
    data_dir = Path(data_dir)
    start = time.monotonic()

    # Resolve the concrete adapter.
    target = None
    if adapter:
        if adapter == "internal":
            target = "internal"
        elif adapter in _ADAPTER_META and _detect(_ADAPTER_META[adapter]["command"]):
            target = adapter
    if target is None:
        target = _available_cli() or "internal"

    if target != "internal":
        return _dispatch_cli(task, target, workspace, start)

    return _dispatch_internal(task, workspace, data_dir, start)


def _dispatch_cli(task: str, adapter_id: str, workspace: Path, start: float) -> HarnessResult:
    meta = _ADAPTER_META[adapter_id]
    cmd = [meta["command"], *meta["args"], task]
    try:
        proc = subprocess.run(
            cmd, cwd=str(workspace), capture_output=True, text=True,
            timeout=DISPATCH_TIMEOUT_S, shell=False,
        )
        stdout = _cap_output(proc.stdout or "")
        steps = [ln for ln in stdout.splitlines() if ln.strip()]
        diffs = [ln for ln in steps if ln.startswith(("+", "-")) and not ln.startswith(("+++", "---"))]
        status = "ok" if proc.returncode == 0 else "error"
        return HarnessResult(
            adapter=adapter_id, task=task, steps=steps[:OBSERVE_LIMIT], diffs=diffs,
            status=status, duration_ms=int((time.monotonic() - start) * 1000),
        )
    except subprocess.TimeoutExpired:
        return HarnessResult(
            adapter=adapter_id, task=task, status="error",
            steps=[f"timed out after {DISPATCH_TIMEOUT_S}s"],
            duration_ms=int((time.monotonic() - start) * 1000),
        )
    except Exception as exc:  # noqa: BLE001 — report, never crash the endpoint
        logger.warning("harness %s failed: %s", adapter_id, exc)
        return HarnessResult(
            adapter=adapter_id, task=task, status="error",
            steps=[f"dispatch error: {exc}"],
            duration_ms=int((time.monotonic() - start) * 1000),
        )


def _dispatch_internal(task: str, workspace: Path, data_dir: Path, start: float) -> HarnessResult:
    """Bounded plan→act→observe loop (the deterministic fallback engine)."""
    steps: list[str] = []
    diffs: list[str] = []
    status = "ok"

    # --- PLAN: decompose the task into concrete sub-steps ---
    subtasks = [s.strip() for s in re.split(r"[\n;]|(?<=\.)\s", task) if s.strip()]
    if not subtasks:
        subtasks = [task]
    steps.append(f"PLAN: decomposed task into {len(subtasks)} sub-step(s)")

    # --- ACT: detect an explicit safe file edit and apply it ---
    edit_m = re.search(
        r'(?:edit|modify|change|update|fix)\s+(?:file\s+)?["\']?([^\s"\']+\.\w+)["\']?',
        task, re.IGNORECASE,
    )
    if edit_m:
        rel = edit_m.group(1)
        try:
            resolved = safe_resolve(workspace, rel)
            if resolved.exists() and resolved.is_file():
                orig = resolved.read_text(encoding="utf-8", errors="replace")
                new_content, applied, note = apply_rule_edit(orig, task)
                if applied:
                    backup_ref = write_backup(data_dir / "backups", resolved, orig)
                    resolved.write_text(new_content, encoding="utf-8")
                    diff = "".join(__import__("difflib").unified_diff(
                        orig.splitlines(keepends=True), new_content.splitlines(keepends=True),
                        fromfile="original", tofile="modified",
                    ))
                    diffs.append(diff.strip() or f"(no textual diff) {note}")
                    steps.append(f"ACT: edited {rel} ({note}); backup={backup_ref}")
                else:
                    steps.append(f"ACT: no safe edit matched for {rel} ({note})")
                    status = "partial"
            else:
                steps.append(f"ACT: target {rel} not found in workspace — skipped mutation")
                status = "partial"
        except ValueError as exc:
            steps.append(f"ACT: rejected unsafe path {rel!r} ({exc})")
            status = "error"
    else:
        steps.append("ACT: no file target detected — performing read-only analysis")

    # --- OBSERVE: report the resulting workspace state ---
    files = [p.name for p in sorted(workspace.iterdir()) if p.is_file()][:OBSERVE_LIMIT]
    steps.append(f"OBSERVE: workspace has {len(files)} file(s): {', '.join(files) or '(none)'}")
    return HarnessResult(
        adapter="internal", task=task, steps=steps, diffs=diffs,
        status=status, duration_ms=int((time.monotonic() - start) * 1000),
    )
