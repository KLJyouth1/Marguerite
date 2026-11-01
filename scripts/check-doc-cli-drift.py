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

Second round (2026-11): the guard itself had a coverage hole. It globbed
`docs/*.md` only, so `docs/agents/` (the split-out AGENTS.md volumes) was never
scanned -- and those volumes are exactly where a second "authoritative" route
table lives. Result: `docs/architecture.md` claimed **19** subcommands while
`docs/agents/03-invariants.md` claimed **20** (it knew about
`export-diagnostics`), and no gate could see the contradiction. Hence Tier 0.

What it checks
--------------
Tier 0 (FAIL) -- the route surface must be internally consistent and every doc
  that *claims* a size must claim the real one:
    * `argv.rs::SUBCOMMANDS` and the `route_table.register(...)` calls in
      program.rs must be the same set (adding to one place only is the classic
      half-landed change);
    * any doc line of the form `权威子命令表（N 个` or `当前 **N 个路由**` must
      agree with the registered count;
    * the `### 权威子命令表` markdown table must list exactly the registered
      names -- no missing row, no extra row.

Tier 1 (FAIL) -- every documented `marguerite <sub>` must be a registered route.

Tier 2 (FAIL) -- for subcommands that dispatch on nested verbs (a handler that
  reads `sub_args`, e.g. `mcp`), every documented `marguerite <sub> <verb>` must
  appear as a string literal inside that handler's body. This is exactly what
  catches `mcp connect` while allowing `mcp list`.

Scan scope
----------
  * `docs/*.md`            -> Tier 0 + Tier 1 + Tier 2
  * `docs/agents/*.md`     -> Tier 0 + Tier 1   (the split-out AGENTS.md volumes
                              restate the route contract -- that is precisely
                              where the 19-vs-20 contradiction lived -- but
                              their fences mix CLI examples with placeholder
                              syntax like `marguerite <sub>`, so enforcing
                              Tier 2 there yields systematic false positives,
                              and a gate that cries wolf gets loosened instead
                              of fixed)
  * excluded: `internal/`, `planning/`, `reports/`, `research/`, `uiux/`,
    `archive/`, `decisions/`, `legacy/`, `prd/` -- historical, retired, or not
    contract-bearing; plus `plans/`, `product/`, `qa/`, `design/` -- dated
    point-in-time working documents (roadmaps, the PRD, a test plan, the
    liquid-glass and provenance specs). Verified 2026-11-01: those four hold 13
    markdown files and zero `marguerite <verb>` invocations between them, so
    Tier 1 had nothing to check. They are excluded because they are *not* the
    CLI contract's home, not because they are red.
  * any *other* `docs/<sub>/` directory is an ERROR, never a silent skip: a new
    subdirectory must be classified in SCANNED_SUBDIRS or EXCLUDED_SUBDIRS.
    The 19-vs-20 drift survived because the old glob simply never reached the
    directory holding the second table -- "unclassified directory" makes that
    failure mode impossible to reintroduce quietly.

Note on hyphenated subcommands: `export-diagnostics` is a route. Every pattern
below therefore accepts `[a-z0-9_-]`, and quoted-string parsing is used for
`SUBCOMMANDS` so `"export-diagnostics"` is captured whole rather than as
`export`.

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

#: Sub-trees of `docs/` that restate the CLI contract and must be scanned.
SCANNED_SUBDIRS = {"agents"}

#: Sub-trees of `docs/` that are historical or not contract-bearing.
#: `legacy/` and `prd/` are in the list because AGENTS.md §1 references
#: `docs/legacy/gui-react-2026-10-21/` and `docs/prd/2026-10-21-gui-react-retire.md`
#: -- retired-material vaults, same category as `archive/`. Listing a directory
#: that turns out not to exist is a no-op, so erring toward "excluded" here costs
#: nothing, while leaving one out fails the gate on day one.
EXCLUDED_SUBDIRS = {
    "internal",
    "planning",
    "reports",
    "research",
    "uiux",
    "archive",
    "decisions",
    "legacy",
    "prd",
    # Point-in-time working documents (dated filenames say so): `plans/` holds
    # remediation roadmaps, `product/` the PRD, `qa/` a test plan, `design/` the
    # liquid-glass + provenance specs. Verified 2026-11-01: none of them contains
    # a single `marguerite <verb>` invocation, so Tier 1 would have had nothing to
    # check. They are excluded because they are *not* the CLI contract's home,
    # not because they are red.
    "plans",
    "product",
    "qa",
    "design",
}

#: `NAME` chars: lowercase, digits, `_`, and `-` (for `export-diagnostics`).
_NAME = r"[a-z][a-z0-9_-]*"

#: Doc lines that *assert* a route count. Each group(1) is the claimed number.
DECLARED_COUNT_PATTERNS = (
    re.compile(r"权威子命令表\s*（\s*(\d+)\s*个"),
    re.compile(r"当前\s*\*\*\s*(\d+)\s*个路由"),
)

#: Heading of the markdown table that mirrors the route table.
AUTH_TABLE_HEADING = re.compile(r"^###\s*权威子命令表.*$", re.M)


def read(p: Path) -> str:
    return p.read_text(encoding="utf-8", errors="ignore")


def parse_subcommands(argv: str) -> set[str]:
    """Names inside `pub const SUBCOMMANDS: &[&str] = &[ ... ]`."""
    m = re.search(r"SUBCOMMANDS\s*:\s*&\[&str\]\s*=\s*&\[(.*?)\];", argv, re.S)
    if not m:
        return set()
    return set(re.findall(r'"(' + _NAME + r')"', m.group(1)))


def registered_routes(prog: str) -> set[str]:
    """Names passed to `route_table.register("<name>", ...)` in program.rs.

    Scoped to `route_table.register` on purpose: a bare `\\bregister\\(` would
    also sweep unrelated registries (`ToolRegistry`, plugin containers) and
    silently widen the expected route set.
    """
    return set(
        re.findall(r'route_table\.register\(\s*"(' + _NAME + r')"', prog)
    )


def canonical_routes(prog: str, argv: str) -> set[str]:
    """Union of both authoritative sources (kept for Tier1 tolerance)."""
    return parse_subcommands(argv) | registered_routes(prog)


def handler_bodies(prog: str) -> dict[str, str]:
    """Map each top-level `fn name(` to its body (up to the next top-level fn).

    The `async`/`pub` prefixes are part of the pattern on purpose: with a bare
    ``\\nfn\\s+`` every `async fn` handler in `program.rs` was invisible, so the
    dispatcher set came out empty and Tier 2 could never fire. A check that is
    structurally unable to fail is worse than no check -- it reads as green.
    """
    fn_head = r"\n(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+"
    bodies: dict[str, str] = {}
    for m in re.finditer(fn_head + r"([A-Za-z_]\w*)\s*\(", prog):
        name = m.group(1)
        start = m.end()
        rest = prog[start:]
        nxt = re.search(fn_head + r"[A-Za-z_]\w*\s*\(", rest)
        bodies[name] = rest[: nxt.start()] if nxt else rest
    return bodies


def verb_dispatchers(prog: str, bodies: dict[str, str]) -> set[str]:
    """Subcommands whose handler branches on `sub_args` (nested-verb dispatch)."""
    out: set[str] = set()
    for sub, handler in re.findall(
        r'route_table\.register\(\s*"(' + _NAME + r')"\s*,\s*([A-Za-z_]\w*)\s*,', prog
    ):
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


def doc_dirs() -> list[str]:
    """Every `docs/<sub>/` directory that actually holds markdown."""
    return sorted(
        {
            p.relative_to(DOCS_DIR).parts[0]
            for p in DOCS_DIR.rglob("*.md")
            if p.relative_to(DOCS_DIR).parent != Path(".")
        }
    )


def unclassified_dirs() -> list[str]:
    """Subdirectories that are neither scanned nor deliberately excluded."""
    known = SCANNED_SUBDIRS | EXCLUDED_SUBDIRS
    return [d for d in doc_dirs() if d not in known]


def doc_targets() -> list[tuple[Path, bool]]:
    """`(path, strict_nested)` for every doc in scope, deterministically ordered."""
    targets: list[tuple[Path, bool]] = []
    for path in sorted(DOCS_DIR.rglob("*.md")):
        rel = path.relative_to(DOCS_DIR)
        if rel.parent == Path("."):
            targets.append((path, True))  # shipped manual: Tier 1 + Tier 2
            continue
        root = rel.parts[0]
        if root not in SCANNED_SUBDIRS:
            continue  # excluded, or reported by unclassified_dirs()
        targets.append((path, False))  # maintainer volume: Tier 1 only
    return targets


def auth_table_routes(md_text: str) -> set[str] | None:
    """Subcommand names listed by the `### 权威子命令表` markdown table, if any."""
    m = AUTH_TABLE_HEADING.search(md_text)
    if not m:
        return None
    tail = md_text[m.end() :]
    nxt = re.search(r"^###\s", tail, re.M)
    block = tail[: nxt.start()] if nxt else tail
    listed = set(re.findall(r"^\|\s*`(" + _NAME + r")`", block, re.M))
    return listed or None


def main() -> int:
    argv = read(CLI_DIR / "argv.rs")
    prog = read(CLI_DIR / "program.rs")

    declared = parse_subcommands(argv)
    routes = registered_routes(prog)
    bodies = handler_bodies(prog)
    dispatchers = verb_dispatchers(prog, bodies)

    drift: list[str] = []

    # ---- Tier 0a: the two code-side sources must agree -------------------
    for name in sorted(declared - routes):
        drift.append(
            f"[Tier0] '{name}' is in argv.rs::SUBCOMMANDS but has no "
            f"route_table.register(...) call in program.rs."
        )
    for name in sorted(routes - declared):
        drift.append(
            f"[Tier0] '{name}' is registered in program.rs but missing from "
            f"argv.rs::SUBCOMMANDS."
        )

    if not routes:
        drift.append(
            "[Tier0] no registered routes parsed out of program.rs -- the parser "
            "is broken, so every downstream check would be vacuous."
        )

    for name in unclassified_dirs():
        drift.append(
            f"[Tier0] docs/{name}/ is not classified: add it to SCANNED_SUBDIRS "
            f"if it restates the CLI contract, or to EXCLUDED_SUBDIRS if it is "
            f"history. An unclassified directory must never be silently skipped."
        )

    doc_top: dict[str, set[str]] = {}
    doc_nested: dict[tuple[str, str], set[str]] = {}

    for path, strict_nested in doc_targets():
        text = read(path)
        rel = path.relative_to(ROOT).as_posix()

        # ---- Tier 0b: a doc that claims a size must claim the real size ---
        claims = [
            raw for pattern in DECLARED_COUNT_PATTERNS for raw in pattern.findall(text)
        ]
        for raw in claims:
            if int(raw) != len(routes):
                drift.append(
                    f"[Tier0] {rel} declares {raw} subcommands, but "
                    f"{len(routes)} routes are registered: {sorted(routes)}"
                )

        # ---- Tier 0c: a table that claims to be complete must be complete --
        # Only enforced when the same file also asserts a count: that assertion
        # is what turns a table into a completeness claim. A category listing
        # (e.g. "the browser-related subcommands") must not be judged by it.
        listed = auth_table_routes(text) if claims else None
        if listed is not None:
            for name in sorted(listed - routes):
                drift.append(
                    f"[Tier0] {rel} route table lists '{name}', which is not a "
                    f"registered route."
                )
            for name in sorted(routes - listed):
                drift.append(
                    f"[Tier0] {rel} route table is missing '{name}' "
                    f"({len(listed)} listed vs {len(routes)} registered)."
                )

        # ---- Tier 1 / Tier 2 ---------------------------------------------
        for snip in code_snippets(text):
            for m in re.finditer(r"\bmarguerite\s+(" + _NAME + r")", snip):
                doc_top.setdefault(m.group(1).rstrip("-_"), set()).add(rel)
            if not strict_nested:
                continue
            for m in re.finditer(
                r"\bmarguerite\s+(" + _NAME + r")\s+(" + _NAME + r")", snip
            ):
                doc_nested.setdefault(
                    (m.group(1).rstrip("-_"), m.group(2).rstrip("-_")), set()
                ).add(rel)

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
        handler = re.search(
            r'route_table\.register\(\s*"' + re.escape(sub) + r'"\s*,\s*([A-Za-z_]\w*)\s*,',
            prog,
        )
        allowed = (
            set(re.findall("(" + _NAME + ")", bodies.get(handler.group(1), "")))
            if handler
            else set()
        )
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
        print(f"\nregistered routes ({len(routes)}) = {sorted(routes)}")
        print(f"SUBCOMMANDS     ({len(declared)}) = {sorted(declared)}")
        print(f"verb-dispatchers = {sorted(dispatchers)}")
        return 1

    scanned = doc_targets()
    print(
        f"OK: {len(routes)} routes consistent across argv.rs, program.rs and the "
        f"authoritative tables.\n"
        f"{len(doc_top)} top-level + {len(doc_nested)} nested documented CLI "
        f"tokens all resolve to real CLI code ({len(scanned)} docs scanned).\n"
        f"registered routes = {sorted(routes)}\n"
        f"verb-dispatchers = {sorted(dispatchers)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
