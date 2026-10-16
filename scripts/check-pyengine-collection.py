#!/usr/bin/env python3
"""Fail if a bare `pytest` over `pyengine/` cannot collect cleanly.

Why this exists
---------------
`pyengine/` is the Python sidecar. Its suite is large (~20k tests) and most of it
lives outside the gated subtree: `scripts/verify.sh --pyengine` deliberately runs
only `tests/unit`, because the rest reaches for the network, model APIs, a browser
or a real terminal.

That scoping left a blind spot: a bare `pytest` in `pyengine/` used to abort with
19 collection ERRORs from modules that import developer/release tooling which was
never migrated into this repository. Nothing failed, because nothing ran it.

`pyengine/tests/conftest.py` now skips exactly those modules -- naming the missing
prerequisite -- and the list is self-healing. This script is the guard on top: it
proves collection is still clean, and that the skip list has not quietly grown into
a place where broken tests go to hide.

It is intentionally *not* wired into `verify.sh`: a full collect-only pass imports
every test module, and the gate deliberately confines itself to `tests/unit`.

Exit codes
----------
0  collection clean
1  one or more collection errors (each is printed with its cause)
2  could not verify (no pytest, or no `pyengine/tests`) -- never reported as a pass
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
PYENGINE = REPO_ROOT / "pyengine"

_BLOCK_RE = re.compile(r"^_+ ERROR collecting (\S+) _+$")
_COLLECTED_RE = re.compile(r"(\d+) tests? collected")
_ERRORS_RE = re.compile(r"(\d+) errors?")
_SKIPPED_RE = re.compile(r"(\d+) test module\(s\) SKIPPED")


def _run(argv: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        argv,
        cwd=PYENGINE,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )


def _error_causes(text: str) -> list[tuple[str, str]]:
    """Map each collection error to the final exception line of its traceback."""
    lines = text.splitlines()
    starts = [
        (i, m.group(1))
        for i, line in enumerate(lines)
        if (m := _BLOCK_RE.match(line.strip()))
    ]
    causes: list[tuple[str, str]] = []
    for idx, (start, path) in enumerate(starts):
        end = starts[idx + 1][0] if idx + 1 < len(starts) else len(lines)
        cause = ""
        for line in lines[start:end]:
            stripped = line.strip()
            if stripped.startswith("E ") and ":" in stripped:
                cause = stripped[2:].strip()
        causes.append((path, cause or "(no exception line found)"))
    return causes


def main() -> int:
    if not PYENGINE.is_dir():
        print("[FAIL] pyengine/ not found -- cannot verify", file=sys.stderr)
        return 2

    probe = _run([sys.executable, "-c", "import pytest"])
    if probe.returncode != 0:
        print(
            "[SKIP] not verified: pytest is not installed for "
            f"{sys.executable}\n       install it, then re-run this script.",
            file=sys.stderr,
        )
        return 2

    result = _run([
        sys.executable, "-m", "pytest", "tests",
        "--collect-only", "-q", "--continue-on-collection-errors",
        "-p", "no:cacheprovider",
    ])
    text = (result.stdout or "") + (result.stderr or "")
    lines = text.splitlines()
    # `--collect-only -q` prints every node id, so counts must only ever be read
    # from the footer. Scanning the whole output would eventually match a test
    # whose name happens to contain "N errors" and report a false FAIL.
    footer = "\n".join(lines[-6:])

    collected = _COLLECTED_RE.search(footer)
    errors = _ERRORS_RE.search(footer)
    skipped = _SKIPPED_RE.search(text)
    causes = _error_causes(text)

    n_collected = collected.group(1) if collected else "?"
    n_errors = int(errors.group(1)) if errors else 0
    n_skipped = int(skipped.group(1)) if skipped else 0

    if causes or n_errors:
        print(f"[FAIL] {len(causes) or n_errors} collection error(s) in pyengine/tests")
        for path, cause in causes:
            print(f"       {path}\n         -> {cause}")
        print(
            "\n       Either restore the missing prerequisite, or -- if it is genuinely\n"
            "       un-migrated scaffolding -- add the module to _SCAFFOLDING_GROUPS in\n"
            "       pyengine/tests/conftest.py so the skip is explicit and visible.",
            file=sys.stderr,
        )
        return 1

    print(f"[PASS] pyengine/tests collects cleanly: {n_collected} tests, 0 errors")
    if n_skipped:
        print(
            f"       {n_skipped} module(s) not collected: missing prerequisite, or an "
            "inherited upstream contract (see AGENTS.md section 2)."
        )
        print("       Skipped is NOT passed -- these were not verified by this run.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
