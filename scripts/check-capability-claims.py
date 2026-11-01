#!/usr/bin/env python3
"""Fail if the capability registry sells a symbol that no production code calls.

Why this exists
---------------
`core/src/domain/capabilities.rs` is the single source of truth for what Marguerite
claims it can do: the TUI and the desktop GUI both render their capability tree from
it. The registry has a `maturity` field that separates `Real` from `Narrative`, so a
capability can be honestly labelled "pure story" -- but nothing ever checked the
*prose inside* the description.

The defect this gate catches is narrower and nastier than a `Narrative` row.
`mem.semantic` is marked `Real` and its description names `hybrid_retrieve`
("hybrid_retrieve 词法×向量加权检索 ... 含余弦/重排单测"). The function exists, is
`pub`, is re-exported from `memory/mod.rs`, and has three unit tests -- and has
**zero production call sites**. A reader (or an agent told to "use the hybrid memory
retrieval") gets a library that is on no chain, while the real chain ranks by a
different, deliberately chosen policy (`scoring::lexical_first` + relative
threshold; the RRF variant was measured and rejected).

That class -- "the code exists, so the copy is honest" -- is what a `pub fn` check
cannot see. So this gate counts call sites instead:

  * a symbol named in a capability description AND defined in this tree,
  * with no non-test, non-use, non-doc occurrence anywhere in `core/src`,
  * is a decorative claim -> FAIL, listing the cap id and the definition location.

Deliberately conservative (so it does not cry wolf and get loosened):
  * symbols NOT defined in this tree are skipped: `session_restorer`, `model_router`
    and friends are external project names in the `supersedes` column, not our code.
  * only `fn` / `struct` / `enum` / `trait` / `const` / `static` / `type` definitions
    count as "ours"; field names such as `use_copilot` are not definitions.
  * a symbol that is only ever *constructed as a type* still counts as used; the gate
    asks "does anything reference it outside its own definition and its own tests",
    not "is it called like a function".
  * `#[cfg(test)]` modules, `use` / `pub use` lines, comments and doc-comments are
    stripped before counting -- a re-export is not a caller, and a comment about a
    function is not a use of it.

Exit codes
----------
0  every in-tree symbol named by the registry has at least one production reference
1  at least one decorative claim (each printed with its evidence)
2  could not verify (registry missing, no capability parsed, ...) -- never a pass
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
REGISTRY = REPO_ROOT / "core" / "src" / "domain" / "capabilities.rs"
SRC_ROOT = REPO_ROOT / "core" / "src"

# ── registry parsing ─────────────────────────────────────────────────────────
CAP_CALL = re.compile(r"\bcap!\s*\(")
STRING_LIT = re.compile(r'"(?:[^"\\]|\\.)*"')
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]{2,}")
TOKEN = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
RUST_KEYWORDS = {
    # words that look like symbols but are vocabulary, not identifiers we own
    "and", "or", "not", "for", "with", "true", "false", "some", "none", "ok",
    "err", "pub", "fn", "let", "mod", "use", "struct", "enum", "impl", "self",
    "list", "add", "show", "search", "get", "set", "map", "vec", "box", "arc",
    "hash", "json", "yaml", "toml", "utf", "ascii", "http", "https", "unix",
    "windows", "linux", "linux2", "macos", "openai", "deepseek", "hermes",
    "claude", "cursor", "codex", "qoder", "trae", "bloom", "opus", "api",
    "rpc", "tui", "gui", "webui", "cli", "mcp", "sql", "fts", "bm25", "rrf",
    "cjk", "utf8", "l2", "kv", "pdf", "ocr", "rag", "sse", "ws", "http2",
}

# ── definition search ────────────────────────────────────────────────────────
DEF_PREFIX = r"(?:pub(?:\([^)]*\))?|default|unsafe|async|extern|auto)\s+"
DEF_RE = re.compile(
    r"^[ \t]*(?:" + DEF_PREFIX + r")*?(fn|struct|enum|trait|const|static|type|union|mod)\s+([A-Za-z_][A-Za-z0-9_]*)",
    re.M,
)

# ── noise stripped before counting references ────────────────────────────────
# A `use` statement runs to its `;`; Rust use trees contain no semicolons, so the
# non-greedy-free `[^;]*` below cannot overshoot into real code.
USE_STMT = re.compile(r"^[ \t]*(?:(?:pub|crate)[ \t]+)?use[ \t][^;]*;", re.M)
USE_LINE = re.compile(r"^[ \t]*(?:pub[ \t]+)?use[ \t]")
COMMENT_LINE = re.compile(r"^[ \t]*(?://|/\*|\*)")


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8", errors="replace")


def split_top_level(args: str) -> list[str]:
    """Split macro args on commas that are not nested in parens/brackets/strings."""
    out, buf, depth, i = [], [], 0, 0
    while i < len(args):
        c = args[i]
        if c == '"':
            j = i + 1
            while j < len(args):
                if args[j] == "\\":
                    j += 2
                    continue
                if args[j] == '"':
                    break
                j += 1
            buf.append(args[i : j + 1])
            i = j + 1
            continue
        if c in "([{":
            depth += 1
        elif c in ")]}":
            if depth == 0:
                break  # the closing paren of the cap! call itself
            depth -= 1
        if c == "," and depth == 0:
            out.append("".join(buf).strip())
            buf = []
            i += 1
            continue
        buf.append(c)
        i += 1
    if buf:
        out.append("".join(buf).strip())
    return out


def parse_capabilities(text: str) -> list[dict]:
    """Every `cap!(...)` in the registry, as {id, name, desc, maturity, line}."""
    caps = []
    for m in CAP_CALL.finditer(text):
        # skip the macro definition arms themselves (`($id:expr, ...)`), which
        # never start with a quoted id
        head = text[m.end() : m.end() + 2]
        if not head.lstrip().startswith('"'):
            continue
        args = split_top_level(text[m.end() :])
        if len(args) < 5:
            continue
        literals = STRING_LIT.findall(" ".join(args))
        if not literals:
            continue
        cap_id = literals[0][1:-1]
        cap_name = literals[1][1:-1] if len(literals) > 1 else ""
        # the description is the longest string literal (ids and names are short
        # and paren-free; descriptions carry the technical prose)
        desc = max(literals, key=len)[1:-1]
        # maturity is the 6th macro argument, a bare identifier -- read it by
        # position, never by scanning the prose for the word "Real"
        maturity = args[5].strip() if len(args) >= 6 else ""
        if maturity not in ("Real", "Partial", "Narrative"):
            maturity = "?"
        caps.append(
            {
                "id": cap_id,
                "name": cap_name,
                "desc": desc,
                "maturity": maturity,
                "line": text[: m.start()].count("\n") + 1,
            }
        )
    return caps


def strip_tests(text: str) -> str:
    """Remove `#[cfg(test)]` modules by brace-matching their `mod { ... }` body."""
    out = text
    while True:
        at = out.find("#[cfg(test)]")
        if at < 0:
            return out
        brace = out.find("{", at)
        semi = out.find(";", at)
        if 0 <= semi < brace:
            # a bare declaration (`#[cfg(test)] mod tests;`) -- drop only that line,
            # never the next unrelated block
            out = out[:at] + " " * (semi + 1 - at) + out[semi + 1 :]
            continue
        if brace < 0:
            return out[:at]
        depth, i = 0, brace
        while i < len(out):
            c = out[i]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    break
            elif c == '"':
                i += 1
                while i < len(out) and out[i] != '"':
                    if out[i] == "\\":
                        i += 1
                    i += 1
            elif out.startswith("//", i):
                nl = out.find("\n", i)
                i = nl - 1 if nl >= 0 else len(out)
            i += 1
        end = i + 1 if i < len(out) else len(out)
        out = out[:at] + " " * (end - at) + out[end:]
        if "#[cfg(test)]" not in out:
            return out


def production_body(text: str) -> str:
    """Strip test modules, `use` statements, and comment lines from a Rust file.

    A re-export is not a caller: `pub use self::compact::{... hybrid_retrieve ...}`
    in `memory/mod.rs` must never be counted as evidence that the symbol is on a
    chain. Use statements can span lines, so they are removed by their terminator
    rather than line by line; the removed span is blanked, not deleted, so later
    line numbers still line up.
    """
    body = strip_tests(text)

    def blank(m: re.Match) -> str:
        return re.sub(r"[^\n]", " ", m.group(0))

    body = USE_STMT.sub(blank, body)
    # blank (never delete) the single-line leftovers so reported line numbers still
    # match the file a reader will open
    return "\n".join(
        " " * len(ln) if (USE_LINE.match(ln) or COMMENT_LINE.match(ln)) else ln
        for ln in body.splitlines()
    )


def main() -> int:
    if not REGISTRY.is_file():
        print(f"[FAIL] registry not found: {REGISTRY}")
        return 2
    text = read(REGISTRY)
    caps = parse_capabilities(text)
    if len(caps) < 50:
        print(f"[FAIL] parsed only {len(caps)} cap!() rows -- parser broke, not a pass")
        return 2

    rust_files = sorted(p for p in SRC_ROOT.rglob("*.rs") if p.is_file())
    if not rust_files:
        print(f"[FAIL] no Rust sources under {SRC_ROOT}")
        return 2

    # symbol -> definition locations ("file:line kind")
    definitions: dict[str, list[str]] = {}
    # symbol -> production reference lines (file:line)
    references: dict[str, list[str]] = {}
    bodies: dict[Path, str] = {}

    for path in rust_files:
        raw = read(path)
        for dm in DEF_RE.finditer(raw):
            kind, name = dm.group(1), dm.group(2)
            loc = f"{path.relative_to(REPO_ROOT).as_posix()}:{raw[:dm.start()].count(chr(10)) + 1}"
            definitions.setdefault(name, []).append(f"{loc} {kind}")
        bodies[path] = production_body(raw)

    for path, body in bodies.items():
        rel = path.relative_to(REPO_ROOT).as_posix()
        for name, seen in refs_in(body, set(definitions)).items():
            bucket = references.setdefault(name, [])
            for line_no in seen:
                if len(bucket) < 8:
                    bucket.append(f"{rel}:{line_no}")

    offenders: list[tuple[str, str, list[str]]] = []
    audited: list[tuple[str, str, int]] = []
    for cap in caps:
        named = []
        for token in IDENT.findall(cap["desc"]):
            if token in RUST_KEYWORDS or token in named:
                continue
            # only audit symbols this tree actually defines; `session_restorer` and
            # friends in the supersedes column belong to other projects
            if token in definitions:
                named.append(token)
        for sym in named:
            n = len(references.get(sym, []))
            audited.append((cap["id"], sym, n))
            if n == 0:
                offenders.append((cap["id"], sym, definitions[sym]))

    print(f"registry: {len(caps)} capabilities; in-tree symbols named: {len(audited)}")
    if offenders:
        print()
        print(f"[FAIL] {len(offenders)} capability description(s) name a symbol that is defined")
        print("       in this tree and referenced NOWHERE outside its own definition,")
        print("       its own tests, re-exports, comments and string prose:")
        for cap_id, sym, defs in offenders:
            print(f"  - {cap_id}: `{sym}` -- defined at {'; '.join(defs[:3])}")
        print()
        print("       Fix either side: put it on a real chain, or stop selling it in the")
        print("       capability copy (the registry is what the TUI and GUI render).")
        return 1
    print("[PASS] every in-tree symbol named by the registry has a production reference")
    return 0


def WORD(sym: str) -> re.Pattern:
    """Whole-token matcher. Lookarounds instead of `\\b` so the pattern is exact."""
    return re.compile(r"(?<![0-9A-Za-z_])" + re.escape(sym) + r"(?![0-9A-Za-z_])")


def mask_strings(text: str) -> str:
    """Blank the *contents* of string literals, keeping length and line count."""

    def blank(m: re.Match) -> str:
        return '"' + " " * (len(m.group(0)) - 2) + '"'

    return STRING_LIT.sub(blank, text)


def refs_in(body: str, symbols: set[str]) -> dict[str, list[int]]:
    """symbol -> line numbers where it is *referenced* (not defined, not prose).

    Blanks string-literal contents first: the capability registry itself, log
    messages and doc text mention identifiers without calling them. Then blanks
    the name token of every definition header, so `fn hybrid_retrieve(` does not
    count as a reference to `hybrid_retrieve`. Tokens are collected in one pass
    over the masked text -- scanning per symbol over every file is quadratic and
    stops the gate from ever finishing.
    """
    masked = mask_strings(body)

    def mask_def(m: re.Match) -> str:
        head = m.group(0)[: m.start(2) - m.start(0)]
        return head + " " * len(m.group(2))

    masked = DEF_RE.sub(mask_def, masked)
    out: dict[str, list[int]] = {}
    for idx, line in enumerate(masked.splitlines(), start=1):
        for tm in TOKEN.finditer(line):
            tok = tm.group(0)
            if tok in symbols:
                bucket = out.setdefault(tok, [])
                if len(bucket) < 5:  # presence is the question, not volume
                    bucket.append(idx)
    return out


if __name__ == "__main__":
    sys.exit(main())

