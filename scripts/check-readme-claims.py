#!/usr/bin/env python3
"""Fail if the two READMEs disagree on hard facts, or overstate the gate.

Why this exists
---------------
Two real defects, both found by hand in R27 and both invisible to every other check:

1. `README.zh-Hans.md` had drifted a generation behind `README.md`. It advertised
   the gate as "15/15 green (2026-09-22)" when it was 18, baselines of 2533/3212
   (310 files) when they were 2538/3216 (311), no `pyengine` layer at all, and
   i18n as 3834 keys when all six locales carry 3862. A Chinese reader was being
   shown an older, smaller project than the English reader -- on the same page pair.

2. `README.md` claimed `scripts/check-version-sync.py` was enforced "in CI".
   It was not: the `docs` job ran only `check-doc-cli-drift.py`.

and the same class again while writing this round's summary: the READMEs claimed
the full gate was "18/18" while `verify.sh --all` dispatched 17 checks -- the new
secret scan had been wired into `--fast` and `--core` but not into `--all`. That
is why check (b) below reads verify.sh rather than trusting a number in prose.

What is compared
----------------
Only language-independent facts. `webui unit tests` vs `webui 单测` is a
translation, not a discrepancy, so row *labels* are not compared -- only the
numbers, the set of gated scripts, the heading shape, and the gate total.

Exit codes
----------
0  both READMEs agree, and the claimed gate total matches verify.sh
1  a discrepancy (each is printed)
2  could not verify (a README missing, or a pattern matched nothing) -- never a pass
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
READMES = {
    "README.md": REPO_ROOT / "README.md",
    "README.zh-Hans.md": REPO_ROOT / "README.zh-Hans.md",
}
VERIFY = REPO_ROOT / "scripts" / "verify.sh"

BASELINE = re.compile(r"(\d+)\s+passed\s*/\s*(\d+)\s+failed")
IGNORED = re.compile(r"(\d+)\s+ignored")
SKIPPED = re.compile(r"(\d+)\s+skipped")
TEST_FILES = re.compile(r"(\d+)\s+(?:files|个文件)")
BADGE_TESTS = re.compile(r"core%20tests-(\d+)%20passing")
LOCALE_KEYS = re.compile(r"各\s*(\d+)\s*键|\((\d+) keys each\)")
ENGINE_VERSION = re.compile(r"`(\d+\.\d+\.\d+)`")
GATE_TOTAL = re.compile(r"(\d{1,2})/(\d{1,2})")
HEADING = re.compile(r"^(#{2,3})\s+(\S+)", re.M)
EMOJI = re.compile("[\U0001F300-\U0001FAFF\u2600-\u27BF]")
GATED_SCRIPT = re.compile(r"`(scripts/[\w.\-]+)`")
# A dispatched check inside the `--all` block, e.g. `  check_core_tests`.
DISPATCH = re.compile(r"^\s{2}(check_\w+)\s*$")


def facts(path: Path) -> dict[str, object]:
    text = path.read_text(encoding="utf-8")
    keys = {a or b for a, b in LOCALE_KEYS.findall(text)}
    return {
        "baselines": sorted((int(p), int(f)) for p, f in BASELINE.findall(text)),
        "ignored": sorted(int(x) for x in IGNORED.findall(text)),
        "skipped": sorted(int(x) for x in SKIPPED.findall(text)),
        "test_files": sorted(int(x) for x in TEST_FILES.findall(text)),
        "badge_tests": sorted(int(x) for x in BADGE_TESTS.findall(text)),
        "locale_keys": sorted(int(k) for k in keys),
        "engine_version": sorted(set(ENGINE_VERSION.findall(text))),
        "gate_total": sorted({f"{a}/{b}" for a, b in GATE_TOTAL.findall(text)}),
        "gated_scripts": sorted(set(GATED_SCRIPT.findall(text))),
        "heading_shape": [
            f"{h}{''.join(EMOJI.findall(t[:2]))}" for h, t in HEADING.findall(text)
        ],
    }


def dispatched_checks() -> list[str]:
    """`check_*` calls inside verify.sh's `--all` block, in order."""
    lines = VERIFY.read_text(encoding="utf-8").splitlines()
    start = next(
        (i for i, ln in enumerate(lines) if ln.startswith('if [ "$MODE_ALL" -eq 1 ]')),
        None,
    )
    if start is None:
        return []
    calls: list[str] = []
    for line in lines[start + 1:]:
        if line.startswith("fi"):
            break
        m = DISPATCH.match(line)
        if m:
            calls.append(m.group(1))
    return calls


def main() -> int:
    missing = [name for name, p in READMES.items() if not p.is_file()]
    if missing:
        print(f"[SKIP] not verified: missing {missing}", file=sys.stderr)
        return 2

    per = {name: facts(p) for name, p in READMES.items()}
    names = list(READMES)
    left, right = per[names[0]], per[names[1]]
    bad = 0

    # Honesty guard: a pattern that silently stops matching would make this check
    # vacuous (the failure mode this repository calls a "fake gate").
    empty = [k for k, v in left.items() if not v]
    if empty:
        print(f"[SKIP] not verified: no data extracted for {empty}", file=sys.stderr)
        return 2

    print(f"comparing {names[0]} vs {names[1]}")
    for key in left:
        if left[key] != right[key]:
            bad = 1
            print(f"  MISMATCH {key}:")
            print(f"    {names[0]:18} {ascii(str(left[key]))}")
            print(f"    {names[1]:18} {ascii(str(right[key]))}")

    calls = dispatched_checks()
    if not calls:
        print("[SKIP] not verified: could not read verify.sh's --all block", file=sys.stderr)
        return 2
    claimed = {f"{n}/{n}" for n in {len(calls)}}
    for name in names:
        got = set(per[name]["gate_total"])
        if got != claimed:
            bad = 1
            print(
                f"  MISMATCH gate total in {name}: claims {sorted(got)}, "
                f"but verify.sh --all dispatches {len(calls)} checks -> {sorted(claimed)}"
            )
            print(f"    dispatched: {', '.join(calls)}")

    if bad:
        print("\n[FAIL] the READMEs disagree with each other or with verify.sh", file=sys.stderr)
        return 1
    print(
        f"[PASS] both READMEs agree on every hard fact; "
        f"gate total {sorted(claimed)[0]} matches the {len(calls)} checks verify.sh --all dispatches"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
