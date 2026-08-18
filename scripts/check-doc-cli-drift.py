#!/usr/bin/env python3
"""
Doc/Code CLI drift checker for Marguerite.

Why this exists
---------------
During the CLI/TUI optimization pass (2026-08-07) we found docs describing CLI
surface that did not match the code:

  * docs/mcp.md advertised `marguerite mcp connect`, but `mcp_handler` only
    implements `list`  ->  the `connect` verb never existed.
  * docs/development.md listed `gui/` as a workspace member, but `gui/` is an
    orphan (no Cargo.toml) and the real frontends are `tui/` + `desktop/`.

This script is the automated guard so it cannot drift again silently.

What it checks
--------------
Docs live in docs/*.md. CLI usage examples appear inside fenced code blocks
(``` ... ```) or inline `code` spans; we only scan those (prose is ignored).

  Tier 1 (FAIL): every top-level subcommand `marguerite <sub>` must be a
    registered route in core/src/cli/argv.rs::SUBCOMMANDS or a
    `route_table.register("<sub>", ...)` call in program.rs.

  Tier 2 (FAIL): for subcommands that dispatch on nested verbs (a handler that
    reads `sub_args`, e.g. `mcp`), every documented `marguerite <sub> <verb>`
    must appear as a string literal inside that handler's body. This is exactly
    what catches `mcp connect` while allowing `mcp list`.

Flags (tokens starting with `-`, e.g. `--health`, `--json`) and arguments that
are not verb-dispatched (e.g. session ids, file paths) are intentionally NOT
checked, to avoid false positives.

Usage
-----
  python3 scripts/check-doc-cli-drift.py
  exit 0 = no drift, exit 1 = drift detected (printed list).

Wire into CI as a step after checkout:
  - name: Doc/Code CLI drift check
    run: python3 scripts/check-doc-cli-drift.py
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]  # marguerite/ workspace root
CLI_DIR = ROOT / "core" / "src" / "cli"
DOCS_DIR = ROOT / "docs"


def read(p: Path) -> str:
    return p.read_text(encoding="utf-8", errors="ignore")


def canonical_routes(prog: str, argv: str) -> set[str]:
    routes: set[str] = set()
    m = re.search(r'SUBCOMMANDS\s*:\s*&\[&str\]\s*=\s*&\[([^\]]*)\]', argv, re.S)
    if m:
        routes.update(re.findall(r"([a-z][a-z0-9_]*)", m.group(1)))
    routes.update(re.findall(r'\bregister\(\s*"([a-z][a-z0-9_]*)"', prog))
    return routes


def handler_bodies(prog: str) -> dict[str, str]:
    """Map each top-level `fn name(` to its body (up to the next top-level fn)."""
    bodies: dict[str, str] = {}
    for m in re.finditer(r"\nfn\s+([A-Za-z_]\w*)\s*\(", prog):
        name = m.group(1)
        start = m.end()
        rest = prog[start:]
        nxt = re.search(r"\nfn\s+[A-Za-z_]\w*\s*\(", rest)
        bodies[name] = rest[: nxt.start()] if nxt else rest
    return bodies


def verb_dispatchers(prog: str, bodies: dict[str, str]) -> set[str]:
    """Subcommands whose handler branches on `sub_args` (nested-verb dispatch)."""
    out: set[str] = set()
    for sub, handler in re.findall(r'\bregister\(\s*"([a-z][a-z0-9_]*)"\s*,\s*([A-Za-z_]\w*)\s*,', prog):
        if "sub_args" in bodies.get(handler, ""):
            out.add(sub)
    return out


def code_snippets(md_text: str) -> list[str]:
    snips: list[str] = []
    for m in re.finditer(r"```[^\n]*\n(.*?)```", md_text, re.S):
        snips.append(m.group(1))
    for m in re.finditer(r"`([^`\n]+)`", md_text):
        snips.append(m.group(1))
    return snips


def main() -> int:
    argv = read(CLI_DIR / "argv.rs")
    prog = read(CLI_DIR / "program.rs")
    cli_src = "\n".join(read(f) for f in CLI_DIR.glob("*.rs"))

    routes = canonical_routes(prog, argv)
    bodies = handler_bodies(prog)
    dispatchers = verb_dispatchers(prog, bodies)

    doc_top: dict[str, set[str]] = {}
    doc_nested: dict[tuple[str, str], set[str]] = {}

    for md in sorted(DOCS_DIR.glob("*.md")):
        for snip in code_snippets(read(md)):
            for m in re.finditer(r"\bmarguerite\s+([a-z][a-z0-9_]+)\b", snip):
                doc_top.setdefault(m.group(1), set()).add(md.name)
            for m in re.finditer(r"\bmarguerite\s+([a-z][a-z0-9_]+)\s+([a-z][a-z0-9_]+)\b", snip):
                doc_nested.setdefault((m.group(1), m.group(2)), set()).add(md.name)

    drift: list[str] = []

    # Tier 1: top-level subcommand must be a registered route.
    for sub, files in sorted(doc_top.items()):
        if sub not in routes:
            drift.append(
                f"[Tier1] top-level subcommand '{sub}' documented in {sorted(files)} "
                f"has NO registered route in the CLI."
            )

    # Tier 2: nested verb under a verb-dispatcher must be handled by that handler.
    for (sub, verb), files in sorted(doc_nested.items()):
        if sub not in dispatchers:
            continue  # nested token is an argument, not a dispatched verb
        if verb in routes:
            continue
        handler = re.search(rf'\bregister\(\s*"{sub}"\s*,\s*([A-Za-z_]\w*)\s*,', prog)
        allowed = set(re.findall(r"([a-z][a-z0-9_]*)", bodies.get(handler.group(1), ""))) if handler else set()
        if verb not in allowed:
            drift.append(
                f"[Tier2] nested verb '{sub} {verb}' documented in {sorted(files)} "
                f"is not handled by {handler.group(1) if handler else sub + '_handler'} "
                f"(no matching string arm found)."
            )

    if drift:
        print("DOC/CODE CLI DRIFT DETECTED:")
        for d in drift:
            print("  - " + d)
        print(f"\ncanonical routes = {sorted(routes)}")
        print(f"verb-dispatchers = {sorted(dispatchers)}")
        return 1

    print(
        f"OK: {len(doc_top)} top-level + {len(doc_nested)} nested documented CLI "
        f"tokens all resolve to real CLI code.\n"
        f"canonical routes = {sorted(routes)}\n"
        f"verb-dispatchers = {sorted(dispatchers)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
