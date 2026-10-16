#!/usr/bin/env python3
"""Fail if a real-looking API key is present in the set of files that would ship.

Why this exists
---------------
A real key was committed to this repository early on and still lives in its history
(`smoke_orchestrate.sh`, HEAD). It has since been removed from the working tree, but
nothing stopped it from happening and nothing would stop it happening again.

This is the guard. It scans exactly what `git add -A` would commit -- tracked files
plus untracked-but-not-ignored ones -- because a secret in an ignored scratch file is
not a publication risk while a secret in a committed file is.

Fixture policy (this is deliberate, not a heuristic accident)
-------------------------------------------------------------
Generated keys in tests are expected and fine; a test that redacts secrets needs
something that looks like one. So the rule is inverted: a key-shaped string is
**allowed only if it is recognisably a placeholder** -- it contains a marker such as
`fake`, `test`, `fixture`, `do-not-leak`, sequential padding (`1234`, `abcd`), or is a
CSS class name (`sk-` is also this project's class prefix).

Anything else fails the check. If a legitimate fixture trips it, label the fixture
(e.g. `sk-fake-...`) rather than loosening the rule -- an unlabelled 35-character
high-entropy string next to `API_KEY` is indistinguishable from the real thing.

Exit codes
----------
0  no real-looking key found
1  at least one found (each is printed, masked)
2  could not verify (not a git work tree, or git unavailable) -- never reported as a pass
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Provider-tagged first (more specific), then the generic shape. `sk-` is ALSO this
# repository's CSS class prefix, which is why the allow rules below matter.
PATTERNS = [
    re.compile(r"sk-ant-[A-Za-z0-9_\-]{20,}"),
    re.compile(r"sk-or-v1-[A-Za-z0-9]{20,}"),
    re.compile(r"sk-proj-[A-Za-z0-9_\-]{20,}"),
    re.compile(r"sk-[A-Za-z0-9][A-Za-z0-9]{19,}"),
    re.compile(r"sk-[0-9a-f]{32,}"),
]

# Substrings that mark a value as a deliberate placeholder. Deliberately generous:
# a false negative here costs a review comment, a false positive breaks the gate.
PLACEHOLDER = re.compile(
    r"(?i)(fake|dummy|test|example|sample|placeholder|fixture|redact|secret|"
    r"do-not-leak|notreal|not-real|invalid|nonce|tailboundary|unitsecret|sen71nel|"
    r"abcd|1234|0123456789|cdef|wxyz|qrst|i789|lorem|xxxx|zzz|yyy|"
    r"-key\b|-env\b|-only\b|-test\b|--|__)"
)

MAX_BYTES = 3_000_000


def shipping_files() -> list[str] | None:
    proc = subprocess.run(
        ["git", "ls-files", "-co", "--exclude-standard", "-z"],
        cwd=REPO_ROOT, capture_output=True,
    )
    if proc.returncode != 0:
        return None
    return [p for p in proc.stdout.decode("utf-8", "replace").split("\0") if p]


def mask(value: str) -> str:
    return value[:8] + "…" + value[-4:] if len(value) > 14 else value


def main() -> int:
    files = shipping_files()
    if files is None:
        print("[SKIP] not verified: not a git work tree (or git unavailable)", file=sys.stderr)
        return 2

    hits: list[tuple[str, int, str]] = []
    seen: set[tuple[str, int, str]] = set()
    scanned = 0
    for rel in files:
        path = REPO_ROOT / rel
        try:
            if not path.is_file() or path.stat().st_size > MAX_BYTES:
                continue
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        scanned += 1
        for pattern in PATTERNS:
            for m in pattern.finditer(text):
                value = m.group(0)
                if PLACEHOLDER.search(value):
                    continue
                line = text.count("\n", 0, m.start()) + 1
                # Overlapping patterns can match the same token; report it once.
                if (rel, line, value) in seen:
                    continue
                seen.add((rel, line, value))
                hits.append((rel, line, value))

    if hits:
        print(f"[FAIL] {len(hits)} real-looking API key(s) in files that would ship:")
        for rel, line, value in hits:
            print(f"       {rel}:{line}  {mask(value)}")
        print(
            "\n       If these are fixtures, label them (e.g. `sk-fake-...`) so they are\n"
            "       recognisable. If a real key leaked, ROTATE IT and rewrite history --\n"
            "       see AGENTS.md section 4 (a key already in history stays there).",
            file=sys.stderr,
        )
        return 1

    print(f"[PASS] no real-looking API key in {len(files)} shipping files ({scanned} text files scanned)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
