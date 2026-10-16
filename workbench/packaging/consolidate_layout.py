"""One-shot consolidation migration (idempotent-safe, move-only, logged).

Layout before                          Layout after
------------------------------------   ----------------------------------------
Marguerite/marguerite_workbench/   ->  Marguerite/workbench/backend/marguerite_workbench/
Marguerite/workbench-web/          ->  Marguerite/workbench/frontend/
Marguerite/scripts/package_workbench.py -> Marguerite/workbench/packaging/

Root debris (tmp dirs, stray logs/probes, stale build artefacts) is ARCHIVED —
never deleted — under Marguerite/_archive/pre-consolidation-20260824/, keeping
the workspace auditable and reversible.

Run:  python workbench/packaging/consolidate_layout.py [--apply]
      (default is a dry-run report; pass --apply to execute)
"""

from __future__ import annotations

import argparse
import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ARCHIVE = ROOT / "_archive" / "pre-consolidation-20260824"

MOVES = [
    (ROOT / "marguerite_workbench", ROOT / "workbench" / "backend" / "marguerite_workbench"),
    (ROOT / "workbench-web", ROOT / "workbench" / "frontend"),
    (ROOT / "scripts" / "package_workbench.py", ROOT / "workbench" / "packaging" / "package_workbench.py"),
]

ARCHIVE_DIRS = [
    "mgt_tmp2", "mgt_tmp3", "mgt_tmp4", "mgt_tmp5",
    ".install-test", ".install-test2", ".install-test2",  # dup entry tolerated
    "check-target", "_inspect", ".pytest_cache",
    "pytest-cache-files-0ga41xkp", "pytest-cache-files-g0bjnuhk",
]

ARCHIVE_FILE_PATTERNS = ["*.log", "*.jsonl"]
ARCHIVE_FILES = [
    "cleanup_out.txt", "t.obj", "nul", "query", "stop", "listing.txt",
    "marguerite-desktop.exe", "Marguerite_0.1.5_x64_en-US.wxs",
    "test-browser.js", "test-voxel-racer.js",
    "voxel-racer-logic.js", "voxel-racer-main.js", "voxel-racer.html",
]


def plan() -> tuple[list[tuple[Path, Path]], list[Path]]:
    moves: list[tuple[Path, Path]] = []
    archives: list[Path] = []
    for src, dst in MOVES:
        if src.exists():
            moves.append((src, dst))
    for name in dict.fromkeys(ARCHIVE_DIRS):  # de-dup, keep order
        p = ROOT / name
        if p.exists():
            archives.append(p)
    for pattern in ARCHIVE_FILE_PATTERNS:
        archives.extend(p for p in ROOT.glob(pattern) if p.is_file())
    for name in ARCHIVE_FILES:
        p = ROOT / name
        if p.exists() and p.is_file():
            archives.append(p)
    return moves, sorted(set(archives))


def apply(moves: list[tuple[Path, Path]], archives: list[Path]) -> None:
    for dst_parent in {d.parent for _, d in moves} | {ARCHIVE}:
        dst_parent.mkdir(parents=True, exist_ok=True)
    log: list[str] = []
    for src, dst in moves:
        if dst.exists():
            print(f"SKIP (target exists): {dst}")
            continue
        shutil.move(str(src), str(dst))
        log.append(f"MOVE {src.name} -> {dst}")
    for src in archives:
        dst = ARCHIVE / src.name
        if dst.exists():
            print(f"SKIP (archived already): {dst}")
            continue
        shutil.move(str(src), str(dst))
        log.append(f"ARCHIVE {src.name}")
    (ARCHIVE / "_manifest.txt").write_text("\n".join(log) + "\n", encoding="utf-8")
    print(f"applied {len(log)} operations; manifest at {ARCHIVE / '_manifest.txt'}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--apply", action="store_true", help="execute (default: dry run)")
    args = parser.parse_args()
    moves, archives = plan()
    print(f"plan: {len(moves)} structural move(s), {len(archives)} archive item(s)")
    for src, dst in moves:
        print(f"  MOVE {src} -> {dst}")
    for p in archives[:15]:
        print(f"  ARCHIVE {p.name}")
    if len(archives) > 15:
        print(f"  … and {len(archives) - 15} more")
    if args.apply:
        apply(moves, archives)
    else:
        print("dry run only — pass --apply to execute")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
