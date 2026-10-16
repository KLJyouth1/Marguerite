#!/usr/bin/env python3
"""Fail if a relative link in *this project's own* docs does not resolve.

Why this exists
---------------
`README.zh-Hans.md` said `[SECURITY.md](../SECURITY.md)` and `[AGENTS.md](../AGENTS.md)`.
That file lives at the repository root, so `../` pointed *outside* the repository:
two dead links on the landing page, in the first file a visitor reads. Measured
scope of the same mistake: of 1446 relative links across 2096 shipped markdown
files, only 3 were broken in this project's own docs -- all three fixed.

Scope is a positive allowlist, and that is deliberate
-----------------------------------------------------
The other 195 broken links are all inside vendored third-party skill packs
(`skills/codebuddy/**`, `skills/trae/**`, `pyengine/.../bundled/**`), and some are
not links at all -- the regex sees `{videoUrl}` and `result|data|output|error` in
prose and table pipes. A "no broken links anywhere" gate would therefore demand
fixing 195 files this project does not own, or a pile of exemptions.

So this checks an allowlist of first-party doc roots instead. A vendored pack
added tomorrow cannot make the gate red; a new first-party doc under one of these
roots is checked automatically.

Exit codes
----------
0  every relative link in scope resolves
1  at least one does not (each is printed with file:line)
2  could not verify (no shipped files in scope) -- never reported as a pass
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote

REPO_ROOT = Path(__file__).resolve().parent.parent

# First-party doc roots. Root-level *.md files are picked up dynamically.
SCOPE_DIRS = ("docs/", ".github/", "scripts/", "design/", "workbench/")

LINK = re.compile(r"!?\[[^\]]*\]\(\s*<?([^)\s>]+)>?[^)]*\)")
SCHEME = re.compile(r"^[a-zA-Z][a-zA-Z0-9+.\-]*:")
# GitHub does not render link syntax inside code, and docs legitimately *quote* a
# broken link when explaining it (AGENTS.md does exactly that). Without this, the
# explanation itself is reported as the defect.
INLINE_CODE = re.compile(r"`[^`]*`")
FENCE = re.compile(r"^\s*(```|~~~)")


def shipping_markdown() -> list[str] | None:
    proc = subprocess.run(
        ["git", "ls-files", "-co", "--exclude-standard", "-z"],
        cwd=REPO_ROOT, capture_output=True,
    )
    if proc.returncode != 0:
        return None
    names = [p for p in proc.stdout.decode("utf-8", "replace").split("\0") if p]
    return [p for p in names if p.lower().endswith((".md", ".markdown"))]


def in_scope(rel: str) -> bool:
    if rel.lower().endswith((".md", ".markdown")) and "/" not in rel:
        return True  # root-level docs: README, CONTRIBUTING, SECURITY, ...
    return rel.startswith(SCOPE_DIRS)


def main() -> int:
    files = shipping_markdown()
    if files is None:
        print("[SKIP] not verified: not a git work tree (or git unavailable)", file=sys.stderr)
        return 2
    scoped = [f for f in files if in_scope(f)]
    if not scoped:
        print("[SKIP] not verified: no first-party markdown found", file=sys.stderr)
        return 2

    broken: list[tuple[str, int, str]] = []
    checked = 0
    for rel in scoped:
        path = REPO_ROOT / rel
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        in_fence = False
        for lineno, line in enumerate(text.splitlines(), 1):
            if FENCE.match(line):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            for raw in LINK.findall(INLINE_CODE.sub("", line)):
                target = raw.strip()
                if SCHEME.match(target) or target.startswith(("#", "/")):
                    continue
                target_path = target.partition("#")[0]
                if not target_path:
                    continue  # pure in-page anchor
                checked += 1
                if not (path.parent / unquote(target_path)).exists():
                    broken.append((rel, lineno, target))

    if checked == 0:
        # Every link matching attempt failing would silently "pass" forever.
        print("[SKIP] not verified: no relative links found in scope", file=sys.stderr)
        return 2

    if broken:
        print(f"[FAIL] {len(broken)} relative link(s) in first-party docs do not resolve:")
        for rel, lineno, target in broken:
            print(f"       {rel}:{lineno}  ->  {target}")
        print(
            "\n       Links are relative to the file that contains them. A file at the\n"
            "       repository root reaches a sibling with `SECURITY.md`, not `../SECURITY.md`.",
            file=sys.stderr,
        )
        return 1

    print(f"[PASS] {checked} relative links across {len(scoped)} first-party docs all resolve")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
