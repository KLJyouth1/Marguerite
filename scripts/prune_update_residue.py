#!/usr/bin/env python3
"""Prune the unbounded update residue left behind by local Marguerite deployments.

Three families are covered, each with its own keep window. Nothing is ever
deleted unless its name matches one of the strict patterns below:

    exe       <root>\\marguerite-gui.exe.bak-<YYYYMMDD-HHMMSS>          keep newest N (default 3)
    dist      <root>\\webui\\dist.bak-<YYYYMMDD-HHMMSS>                  keep newest N (default 2)
              <root>\\webui\\dist.prev-*  /  dist.stale-*               same pool as dist.bak-*
    versions  <root>\\.versions\\<encoded>.<YYYYMMDDTHHMMSSZ>[.i].bak    keep newest N per file
                                                                       (default 20, the same
                                                                       value as KEEP_VERSIONS in
                                                                       core/src/tools/versions.rs;
                                                                       entries are grouped by the
                                                                       encoded file prefix)

Safety model
------------
* dry-run by default: only ``--apply`` deletes, and every victim is printed with
  its relative path and byte size before it is touched;
* the install root is validated fail-closed: an empty value, a drive root, the
  repository root (or any root that contains / is contained by the repository),
  the user profile, LOCALAPPDATA / APPDATA, Windows, Program Files, ProgramData
  and temp directories are refused with exit code 2;
* only plain files / plain directories are candidates; symlinks, junctions,
  entries whose mtime cannot be read and unexpected file types are reported as
  ``unverified`` and never deleted (exit 1, so "skipped" never looks like "passed");
* the newest N of every family are never candidates (N >= 1 is enforced), and
  ``_guard()`` re-checks parent directory, name shape and entry type immediately
  before each delete;
* ``--self-test`` builds a fake tree in a temporary directory and proves that the
  keep window, the dry-run default, ``--only`` isolation and the root guards
  actually fire. It never touches the real install directory.

Exit codes (same convention as the other scripts/ checks)
    0  plan produced / apply completed with nothing failed
    1  at least one entry was refused, locked or unverified
    2  bad arguments or an unusable / unsafe root (nothing was touched)

Wiring (deliberately NOT done inside scripts/package_full.cmd or package_app.cmd)
--------------------------------------------------------------------------------
Those two scripts are GBK + CRLF and drive a release build; deleting user data
from inside them would let a failed prune fail (or silently over-succeed) a
release. Keep ``--apply`` a human decision and run the prune as an explicit
post-deploy step after the new build has been verified:

    python -X utf8 scripts/prune_update_residue.py            # dry run, read the plan
    python -X utf8 scripts/prune_update_residue.py --apply    # only if the plan looks right

If a packaging hook is ever added, wire the dry run only and treat a non-empty
plan as a warning, never as a pass. ``--self-test`` is safe to wire anywhere.
"""

from __future__ import annotations

import argparse
import io
import json
import os
import re
import shutil
import stat
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CATEGORIES = ("exe", "dist", "versions")
DEFAULT_KEEP = {"exe": 3, "dist": 2, "versions": 20}

EXE_RE = re.compile(r"\Amarguerite-gui\.exe\.bak-\d{8}-\d{6}\Z")
DIST_RE = re.compile(r"\Adist\.(?:bak-\d{8}-\d{6}|prev-[A-Za-z0-9._-]+|stale-[A-Za-z0-9._-]+)\Z")
VERSIONS_RE = re.compile(r"\A(?P<prefix>.+)\.(?P<stamp>\d{8}T\d{6}Z)(?:\.(?P<seq>\d+))?\.bak\Z")

_FILE_ATTRIBUTE_REPARSE_POINT = 0x400

# Roots that are never acceptable as a deletion target. Resolved on each check
# so the comparison is always against canonical paths.
_DANGEROUS_ENV = (
    ("USERPROFILE", "the user profile"),
    ("LOCALAPPDATA", "LOCALAPPDATA itself"),
    ("APPDATA", "APPDATA itself"),
    ("PROGRAMFILES", "Program Files"),
    ("PROGRAMFILES(X86)", "Program Files (x86)"),
    ("PROGRAMDATA", "ProgramData"),
    ("WINDIR", "the Windows directory"),
    ("SystemRoot", "the Windows directory"),
    ("TEMP", "a temp directory"),
    ("TMP", "a temp directory"),
)


@dataclass
class Candidate:
    path: Path
    label: str
    size: int
    mtime: float
    is_dir: bool
    group: str | None = None


# --------------------------------------------------------------------------- #
# Small filesystem helpers
# --------------------------------------------------------------------------- #


def _stat_no_follow(path: Path):
    """lstat(); kept as a module function so --self-test can prove the
    "mtime unreadable => never delete" rule by patching it."""
    return os.stat(path, follow_symlinks=False)


def _is_reparse_point(st) -> bool:
    return bool(getattr(st, "st_file_attributes", 0) & _FILE_ATTRIBUTE_REPARSE_POINT)


def _is_plain_dir(path: Path) -> bool:
    try:
        st = _stat_no_follow(path)
    except OSError:
        return False
    return stat.S_ISDIR(st.st_mode) and not stat.S_ISLNK(st.st_mode) and not _is_reparse_point(st)


def _label(root: Path, path: Path) -> str:
    try:
        return path.relative_to(root).as_posix()
    except ValueError:
        return path.as_posix()


def _tree_size(path: Path) -> int:
    total = 0
    for dirpath, _dirnames, filenames in os.walk(path, followlinks=False):
        for name in filenames:
            try:
                total += os.stat(os.path.join(dirpath, name), follow_symlinks=False).st_size
            except OSError:
                continue
    return total


def _human(num: int) -> str:
    value = float(num)
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if value < 1024 or unit == "TiB":
            return f"{int(value)} {unit}" if unit == "B" else f"{value:.2f} {unit}"
        value /= 1024
    return f"{value:.2f} TiB"


def _unlink(path: Path) -> None:
    try:
        os.unlink(path)
    except PermissionError:
        os.chmod(path, stat.S_IWRITE)  # read-only attribute from an installer
        os.unlink(path)


def _rmtree_onexc(func, path, exc) -> None:
    try:
        os.chmod(path, stat.S_IWRITE)
    except OSError:
        raise exc
    func(path)


def _delete(cand: Candidate) -> None:
    if cand.is_dir:
        shutil.rmtree(cand.path, onexc=_rmtree_onexc)
    else:
        _unlink(cand.path)


# --------------------------------------------------------------------------- #
# Root validation (fail-closed)
# --------------------------------------------------------------------------- #


def _dangerous_roots() -> list[tuple[Path, str]]:
    found: list[tuple[Path, str]] = []
    for var, label in _DANGEROUS_ENV:
        raw = os.environ.get(var)
        if not raw:
            continue
        try:
            found.append((Path(raw).resolve(), label))
        except OSError:
            continue
    try:
        found.append((Path.home().resolve(), "the home directory"))
    except OSError:
        pass
    return found


def resolve_root(raw: str | None) -> tuple[Path | None, str | None]:
    """Return (resolved_root, None) or (None, reason). Never raises."""
    if raw is None:
        base = os.environ.get("LOCALAPPDATA", "").strip()
        if not base:
            return None, "LOCALAPPDATA is not set; pass --root explicitly"
        raw = os.path.join(base, "Marguerite")

    text = os.path.expandvars(os.path.expanduser(str(raw).strip()))
    if not text.strip():
        return None, "empty --root"
    if not os.path.isabs(text):
        return None, f"--root must be an absolute path: {raw!r}"

    raw_path = Path(text)
    if raw_path.is_symlink():
        return None, f"--root is a symlink/junction: {raw_path}"
    try:
        resolved = raw_path.resolve(strict=True)
    except FileNotFoundError:
        return None, f"--root does not exist: {raw_path}"
    except OSError as exc:
        return None, f"--root cannot be resolved: {raw_path} ({exc.__class__.__name__})"

    if resolved.parent == resolved:
        return None, f"refusing a drive/volume root: {resolved}"
    if len(resolved.parts) < 2:
        return None, f"refusing an overly shallow root: {resolved}"
    if not resolved.is_dir():
        return None, f"--root is not a directory: {resolved}"
    if not _is_plain_dir(resolved):
        return None, f"--root is a symlink/junction: {resolved}"

    for danger, label in _dangerous_roots():
        if resolved == danger:
            return None, f"refusing {label}: {resolved}"

    repo = REPO_ROOT
    if resolved == repo:
        return None, f"refusing the repository root: {resolved}"
    if resolved.is_relative_to(repo):
        return None, f"refusing a root inside the repository: {resolved}"
    if repo.is_relative_to(resolved):
        return None, f"refusing a root that contains the repository: {resolved}"

    return resolved, None


# --------------------------------------------------------------------------- #
# Scanning
# --------------------------------------------------------------------------- #


def _candidate(
    root: Path,
    path: Path,
    *,
    expect_dir: bool,
    unverified: list[dict],
    group: str | None = None,
) -> Candidate | None:
    label = _label(root, path)
    try:
        st = _stat_no_follow(path)
    except OSError as exc:
        unverified.append({"path": label, "reason": f"mtime unreadable ({exc.__class__.__name__})"})
        return None
    if stat.S_ISLNK(st.st_mode) or _is_reparse_point(st):
        unverified.append({"path": label, "reason": "symlink/reparse point"})
        return None
    if expect_dir:
        if not stat.S_ISDIR(st.st_mode):
            unverified.append({"path": label, "reason": "not a plain directory"})
            return None
        size = _tree_size(path)
    else:
        if not stat.S_ISREG(st.st_mode):
            unverified.append({"path": label, "reason": "not a regular file"})
            return None
        size = st.st_size
    return Candidate(path=path, label=label, size=size, mtime=st.st_mtime, is_dir=expect_dir, group=group)


def scan_exe(root: Path, unverified: list[dict]) -> list[Candidate]:
    found: list[Candidate] = []
    try:
        entries = list(os.scandir(root))
    except OSError as exc:
        unverified.append({"path": ".", "reason": f"cannot list root ({exc.__class__.__name__})"})
        return found
    for entry in entries:
        if not EXE_RE.match(entry.name):
            continue
        cand = _candidate(root, root / entry.name, expect_dir=False, unverified=unverified)
        if cand is not None:
            found.append(cand)
    return found


def _scan_named_dir(
    root: Path,
    subdir: str,
    pattern: re.Pattern[str],
    *,
    expect_dir: bool,
    grouped: bool,
    unverified: list[dict],
) -> list[Candidate]:
    base = root / subdir
    if not base.exists():
        return []
    if not _is_plain_dir(base):
        unverified.append({"path": subdir, "reason": "not a plain directory"})
        return []
    found: list[Candidate] = []
    try:
        entries = list(os.scandir(base))
    except OSError as exc:
        unverified.append({"path": subdir, "reason": f"cannot list ({exc.__class__.__name__})"})
        return []
    for entry in entries:
        match = pattern.match(entry.name)
        if not match:
            continue
        cand = _candidate(
            root,
            base / entry.name,
            expect_dir=expect_dir,
            unverified=unverified,
            group=match.group("prefix") if grouped else None,
        )
        if cand is not None:
            found.append(cand)
    return found


def scan_dist(root: Path, unverified: list[dict]) -> list[Candidate]:
    return _scan_named_dir(root, "webui", DIST_RE, expect_dir=True, grouped=False, unverified=unverified)


def scan_versions(root: Path, unverified: list[dict]) -> list[Candidate]:
    return _scan_named_dir(root, ".versions", VERSIONS_RE, expect_dir=False, grouped=True, unverified=unverified)


_SCANNERS = {"exe": scan_exe, "dist": scan_dist, "versions": scan_versions}


# --------------------------------------------------------------------------- #
# Selection and guarded deletion
# --------------------------------------------------------------------------- #


def _split_newest(items: list[Candidate], keep: int) -> tuple[list[Candidate], list[Candidate]]:
    """Return (kept, victims); victims are oldest-first and never include the
    newest `keep` entries."""
    ordered = sorted(items, key=lambda c: (c.mtime, c.label))
    if len(ordered) <= keep:
        return ordered, []
    return ordered[-keep:], ordered[:-keep]


def _split_groups(items: list[Candidate], keep: int) -> tuple[list[Candidate], list[Candidate]]:
    groups: dict[str, list[Candidate]] = {}
    for cand in items:
        groups.setdefault(cand.group or "", []).append(cand)
    kept: list[Candidate] = []
    victims: list[Candidate] = []
    for key in sorted(groups):
        group_kept, group_victims = _split_newest(groups[key], keep)
        kept.extend(group_kept)
        victims.extend(group_victims)
    victims.sort(key=lambda c: (c.mtime, c.label))
    return kept, victims


def _expected_parent(root: Path, category: str) -> Path:
    if category == "exe":
        return root
    return root / ("webui" if category == "dist" else ".versions")


def _guard(cand: Candidate, root: Path, category: str, pattern: re.Pattern[str]) -> str | None:
    """Last-second re-check. Returns a reason to skip, or None to delete."""
    try:
        parent = cand.path.parent.resolve(strict=True)
        expected = _expected_parent(root, category).resolve(strict=True)
    except OSError as exc:
        return f"parent unavailable ({exc.__class__.__name__})"
    if parent != expected:
        return "unexpected parent directory"
    if cand.path.is_symlink():
        return "symlink"
    if not pattern.match(cand.path.name):
        return "name no longer matches"
    try:
        st = _stat_no_follow(cand.path)
    except OSError as exc:
        return f"stat failed ({exc.__class__.__name__})"
    if stat.S_ISLNK(st.st_mode) or _is_reparse_point(st):
        return "symlink/reparse point"
    if cand.is_dir:
        if not stat.S_ISDIR(st.st_mode):
            return "no longer a directory"
    elif not stat.S_ISREG(st.st_mode):
        return "no longer a regular file"
    return None


def _entry(cand: Candidate) -> dict:
    return {"path": cand.label, "bytes": cand.size, "mtime": round(cand.mtime, 3)}


def _prune_category(category: str, root: Path, keep: int, apply: bool, stream) -> dict:
    unverified: list[dict] = []
    items = _SCANNERS[category](root, unverified)
    if category == "versions":
        kept, victims = _split_groups(items, keep)
    else:
        kept, victims = _split_newest(items, keep)

    planned_bytes = sum(c.size for c in victims)
    scope = "/file" if category == "versions" else ""
    stream.write(
        f"[{category}] scanned={len(items)} keep={keep}{scope} delete={len(victims)} "
        f"{'freed' if apply else 'would-free'}={_human(planned_bytes)}\n"
    )
    verb = "DELETE" if apply else "WOULD-DELETE"
    for cand in victims:
        stream.write(f"  {verb:<13} {cand.size:>12} B  {_human(cand.size):>10}  {cand.label}\n")

    deleted: list[Candidate] = []
    failed: list[dict] = []
    if apply:
        pattern = _PATTERNS[category]
        for cand in victims:
            reason = _guard(cand, root, category, pattern)
            if reason:
                failed.append({"path": cand.label, "error": reason})
                stream.write(f"  SKIP          {cand.label} ({reason})\n")
                continue
            try:
                _delete(cand)
            except OSError as exc:
                failed.append({"path": cand.label, "error": f"{exc.__class__.__name__}: {exc}"})
                stream.write(f"  FAILED        {cand.label} ({exc.__class__.__name__})\n")
                continue
            deleted.append(cand)

    return {
        "name": category,
        "keep": keep,
        "scanned": len(items),
        "kept": [_entry(c) for c in kept],
        "planned": [_entry(c) for c in victims],
        "deleted": [_entry(c) for c in deleted],
        "failed": failed,
        "unverified": unverified,
        "bytes_kept": sum(c.size for c in kept),
        "bytes_planned": planned_bytes,
        "bytes_freed": sum(c.size for c in deleted),
    }


_PATTERNS = {"exe": EXE_RE, "dist": DIST_RE, "versions": VERSIONS_RE}


def run(root: Path, keeps: dict[str, int], only: list[str], apply: bool, stream) -> tuple[dict, int]:
    mode = "apply" if apply else "dry-run"
    stream.write(f"prune_update_residue  root={root}  mode={mode.upper()}")
    stream.write("  (nothing is deleted; add --apply)\n" if not apply else "  (deleting)\n")

    categories = [_prune_category(name, root, keeps[name], apply, stream) for name in only]
    totals = {
        "scanned": sum(c["scanned"] for c in categories),
        "planned": sum(len(c["planned"]) for c in categories),
        "deleted": sum(len(c["deleted"]) for c in categories),
        "failed": sum(len(c["failed"]) for c in categories),
        "unverified": sum(len(c["unverified"]) for c in categories),
        "bytes_planned": sum(c["bytes_planned"] for c in categories),
        "bytes_freed": sum(c["bytes_freed"] for c in categories),
    }
    code = 1 if totals["failed"] or totals["unverified"] else 0

    if apply:
        stream.write(
            f"summary: deleted {totals['deleted']} path(s), freed {_human(totals['bytes_freed'])}; "
            f"{totals['failed']} failed, {totals['unverified']} unverified\n"
        )
    else:
        stream.write(
            f"summary: dry-run, nothing deleted; {totals['planned']} path(s), "
            f"{_human(totals['bytes_planned'])} would be freed; "
            f"{totals['unverified']} unverified\n"
        )

    report = {
        "tool": "prune_update_residue",
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "root": str(root),
        "mode": mode,
        "categories": categories,
        "totals": totals,
        "exit_code": code,
    }
    return report, code


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #


def _parse_only(raw: str) -> tuple[list[str] | None, str | None]:
    parts = [p.strip().lower() for p in raw.split(",") if p.strip()]
    unknown = [p for p in parts if p not in CATEGORIES]
    if unknown:
        return None, f"unknown --only value(s): {', '.join(unknown)}"
    if not parts:
        return None, "--only needs at least one of exe,dist,versions"
    return [c for c in CATEGORIES if c in parts], None


def _emit_json(report: dict) -> None:
    payload = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    buffer = getattr(sys.stdout, "buffer", None)
    if buffer is not None:
        buffer.write(payload.encode("utf-8", "backslashreplace"))
        buffer.flush()
    else:
        sys.stdout.write(payload)


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="prune_update_residue.py",
        description="Prune old exe / webui dist backups and .versions snapshots "
        "(dry-run by default; only --apply deletes).",
        epilog="Exit codes: 0 = done, 1 = refused/locked/unverified entries, "
        "2 = bad arguments or unsafe root (nothing touched).",
    )
    parser.add_argument(
        "--root",
        default=None,
        help=r"install root; defaults to %%LOCALAPPDATA%%\Marguerite",
    )
    parser.add_argument(
        "--only",
        default="exe,dist,versions",
        help="comma-separated subset of exe,dist,versions (default: all)",
    )
    parser.add_argument("--exe-keep", type=int, default=DEFAULT_KEEP["exe"], help="keep newest N exe backups (default 3)")
    parser.add_argument("--dist-keep", type=int, default=DEFAULT_KEEP["dist"], help="keep newest N dist backups (default 2)")
    parser.add_argument(
        "--versions-keep",
        type=int,
        default=DEFAULT_KEEP["versions"],
        help="keep newest N snapshots per file (default 20, same as KEEP_VERSIONS)",
    )
    parser.add_argument("--apply", action="store_true", help="actually delete; without it this is a dry run")
    parser.add_argument("--json", action="store_true", help="emit a machine-readable report on stdout")
    parser.add_argument("--self-test", action="store_true", help="prove the rules fire in a temporary tree")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _build_parser().parse_args(argv)

    if args.self_test:
        return self_test()

    root, err = resolve_root(args.root)
    if err:
        print(f"prune_update_residue: {err}", file=sys.stderr)
        print("prune_update_residue: refusing to run (fail-closed)", file=sys.stderr)
        return 2

    only, err = _parse_only(args.only)
    if err:
        print(f"prune_update_residue: {err}", file=sys.stderr)
        return 2

    keeps = {"exe": args.exe_keep, "dist": args.dist_keep, "versions": args.versions_keep}
    bad = [f"--{name}-keep={value}" for name, value in keeps.items() if value < 1]
    if bad:
        print(f"prune_update_residue: keep values must be >= 1: {', '.join(bad)}", file=sys.stderr)
        return 2

    if args.json:
        sink = io.StringIO()
        report, code = run(root, keeps, only, apply=args.apply, stream=sink)
        _emit_json(report)
    else:
        _report, code = run(root, keeps, only, apply=args.apply, stream=sys.stdout)
    return code


# --------------------------------------------------------------------------- #
# Self-test
# --------------------------------------------------------------------------- #


def self_test() -> int:
    """Build a fake install tree under a temp dir and prove every rule fires.

    A prune that has never deleted anything is indistinguishable from one that
    cannot delete anything, so each assertion below checks both directions:
    what must be selected and what must survive.
    """
    results: list[tuple[str, bool, object]] = []

    def check(label: str, ok: bool, detail: object = "") -> None:
        results.append((label, ok, detail))
        if ok:
            print(f"[SELFTEST OK]   {label}")
        else:
            print(f"[SELFTEST FAIL] {label}: {detail}", file=sys.stderr)

    with tempfile.TemporaryDirectory(prefix="prune-residue-selftest-") as tmp:
        root = Path(tmp) / "Marguerite"
        (root / "webui").mkdir(parents=True)
        (root / ".versions").mkdir()
        base = 1_700_000_000.0

        exe_names = [f"marguerite-gui.exe.bak-2026102{i}-120000" for i in range(1, 7)]
        for offset, name in enumerate(exe_names):
            path = root / name
            path.write_bytes(b"x" * (100 + offset))
            os.utime(path, (base + offset, base + offset))
        (root / "marguerite-gui.exe").write_bytes(b"current-binary")
        (root / "marguerite-gui.exe.bak").write_bytes(b"unmatched")
        (root / "other.exe.bak-20261001-120000").write_bytes(b"wrong-prefix")

        dist_names = [f"dist.bak-2026102{i}-120000" for i in range(1, 6)]
        for offset, name in enumerate(dist_names):
            path = root / "webui" / name
            path.mkdir()
            (path / "index.html").write_text("<html></html>", encoding="utf-8")
            os.utime(path, (base + offset, base + offset))
        live = root / "webui" / "dist"
        live.mkdir()
        (live / "index.html").write_text("live", encoding="utf-8")
        (root / "webui" / "dist.old-20260101").mkdir()

        def make_versions(prefix: str, count: int, start: float) -> list[str]:
            names = []
            for offset in range(count):
                stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime(start + offset))
                name = f"{prefix}.{stamp}.bak"
                path = root / ".versions" / name
                path.write_bytes(b"v" * 10)
                os.utime(path, (start + offset, start + offset))
                names.append(name)
            return names

        prefix_a = "_swot%2Faudit.py"
        prefix_b = "notes.md"
        names_a = make_versions(prefix_a, 25, base)
        names_b = make_versions(prefix_b, 22, base + 5000)
        (root / ".versions" / "README.txt").write_text("user data", encoding="utf-8")

        def count(category: str) -> int:
            if category == "exe":
                return len([p for p in root.iterdir() if p.is_file() and EXE_RE.match(p.name)])
            if category == "dist":
                return len([p for p in (root / "webui").iterdir() if p.is_dir() and DIST_RE.match(p.name)])
            return len([p for p in (root / ".versions").iterdir() if p.is_file() and VERSIONS_RE.match(p.name)])

        keeps = dict(DEFAULT_KEEP)
        only_all = list(CATEGORIES)

        # 1. Planning selects the oldest entries only.
        report, code = run(root, keeps, only_all, apply=False, stream=io.StringIO())
        by_name = {cat["name"]: cat for cat in report["categories"]}
        exe_planned = [item["path"] for item in by_name["exe"]["planned"]]
        check("dry-run exit 0", code == 0, code)
        check("exe: 6 scanned, 3 planned", by_name["exe"]["scanned"] == 6 and len(exe_planned) == 3, exe_planned)
        check("exe: oldest 3 selected", exe_planned == exe_names[:3], exe_planned)
        check(
            "exe: newest 3 kept by name",
            [item["path"] for item in by_name["exe"]["kept"]] == exe_names[3:],
            by_name["exe"]["kept"],
        )
        dist_planned = [item["path"] for item in by_name["dist"]["planned"]]
        check("dist: 5 scanned, 3 planned", by_name["dist"]["scanned"] == 5 and len(dist_planned) == 3, dist_planned)
        check("dist: oldest 3 selected", dist_planned == [f"webui/{n}" for n in dist_names[:3]], dist_planned)

        # 2. .versions is grouped per source file: 25 => 5 victims, 22 => 2 victims,
        #    and a group that is already within the window contributes none.
        versions = by_name["versions"]
        victims_a = [i["path"] for i in versions["planned"] if Path(i["path"]).name.startswith(prefix_a + ".")]
        victims_b = [i["path"] for i in versions["planned"] if Path(i["path"]).name.startswith(prefix_b + ".")]
        check("versions: 47 scanned", versions["scanned"] == 47, versions["scanned"])
        check("versions: prefix A drops 5", len(victims_a) == 5, victims_a)
        check("versions: prefix B drops 2", len(victims_b) == 2, victims_b)
        check(
            "versions: oldest of A selected",
            sorted(victims_a) == sorted(f".versions/{name}" for name in names_a[:5]),
            victims_a,
        )
        check(
            "versions: oldest of B selected",
            sorted(victims_b) == sorted(f".versions/{name}" for name in names_b[:2]),
            victims_b,
        )
        check(
            "versions: 20 kept per file",
            len([i for i in versions["kept"] if Path(i["path"]).name.startswith(prefix_a + ".")]) == 20
            and len([i for i in versions["kept"] if Path(i["path"]).name.startswith(prefix_b + ".")]) == 20,
            (len(versions["kept"]),),
        )

        # 3. Dry-run changed nothing.
        check(
            "dry-run deletes nothing",
            count("exe") == 6 and count("dist") == 5 and count("versions") == 47,
            (count("exe"), count("dist"), count("versions")),
        )
        check("live webui/dist untouched", (live / "index.html").read_text(encoding="utf-8") == "live")
        check("dist.old-* untouched (unmatched name)", (root / "webui" / "dist.old-20260101").is_dir())

        # 4. A matching name with the wrong type is reported, never deleted, and
        #    turns the exit code to 1 (an unverified entry is not a pass).
        wrong_root = Path(tmp) / "wrong-type-root"
        wrong_root.mkdir()
        wrong_type = wrong_root / "marguerite-gui.exe.bak-20261001-120000"
        wrong_type.mkdir()
        (wrong_root / "marguerite-gui.exe.bak-20261002-120000").write_bytes(b"keep-me")
        wrong_report, wrong_code = run(wrong_root, keeps, ["exe"], apply=True, stream=io.StringIO())
        check(
            "wrong file type reported unverified",
            wrong_code == 1
            and any("not a regular file" in item["reason"] for item in wrong_report["categories"][0]["unverified"]),
            (wrong_code, wrong_report["categories"][0]["unverified"]),
        )
        check("unverified entry is not deleted", wrong_type.is_dir())

        # 5. The JSON report is machine readable and consistent with the totals.
        try:
            parsed = json.loads(json.dumps(report, ensure_ascii=False))
            ok_json = parsed["totals"]["planned"] == 13 and parsed["exit_code"] == code
        except (TypeError, ValueError, KeyError) as exc:  # pragma: no cover - defensive
            ok_json = False
            detail_json = f"{exc.__class__.__name__}"
        else:
            detail_json = ""
        check("json report parses and totals agree", ok_json, detail_json)

        # 6. --only isolates categories.
        _report, code_only = run(root, keeps, ["exe"], apply=True, stream=io.StringIO())
        check(
            "--only exe prunes exe only",
            code_only == 0 and count("exe") == 3 and count("dist") == 5 and count("versions") == 47,
            (code_only, count("exe"), count("dist"), count("versions")),
        )
        check("newest exe backups survive --apply", all((root / n).is_file() for n in exe_names[3:]))

        # 7. Full apply leaves exactly the keep windows.
        report, code = run(root, keeps, only_all, apply=True, stream=io.StringIO())
        check(
            "apply keeps exe=3 dist=2 versions=20/file",
            code == 0 and count("exe") == 3 and count("dist") == 2 and count("versions") == 40,
            (code, count("exe"), count("dist"), count("versions")),
        )
        check("apply preserved live dist", (live / "index.html").read_text(encoding="utf-8") == "live")
        check("apply preserved decoy files", (root / "marguerite-gui.exe").is_file() and (root / ".versions" / "README.txt").is_file())
        check("apply preserved unmatched dist.old-*", (root / "webui" / "dist.old-20260101").is_dir())
        check("apply freed bytes are reported", report["totals"]["bytes_freed"] > 0, report["totals"])

        # 8. Unreadable mtime => unverified, exit 1, nothing deleted for that entry.
        extra = root / "marguerite-gui.exe.bak-20261029-120000"
        extra.write_bytes(b"z")
        os.utime(extra, (base + 9000, base + 9000))
        real_stat = _stat_no_follow

        def fake_stat(path, _real=real_stat, _target=extra):
            if Path(path) == _target:
                raise PermissionError("simulated unreadable mtime")
            return _real(path)

        globals()["_stat_no_follow"] = fake_stat
        try:
            report, code = run(root, keeps, ["exe"], apply=True, stream=io.StringIO())
        finally:
            globals()["_stat_no_follow"] = real_stat
        check(
            "unreadable mtime => unverified + exit 1 + kept",
            code == 1 and extra.is_file() and report["categories"][0]["unverified"],
            (code, report["categories"][0]["unverified"]),
        )

        # 9. Root guards are fail-closed.
        drive = Path(os.environ.get("SystemDrive", "C:") + "\\")
        bad_roots = {
            "empty": "",
            "drive root": str(drive),
            "repo root": str(REPO_ROOT),
            "home": str(Path.home()),
            "missing": str(Path(tmp) / "does-not-exist"),
        }
        for label, value in bad_roots.items():
            _resolved, reason = resolve_root(value)
            check(f"refuses {label}", reason is not None, (label, value, reason))

        good_root, reason = resolve_root(str(root))
        check("accepts the temp install root", good_root == root.resolve() and reason is None, reason)

    failed = [label for label, ok, _detail in results if not ok]
    print(f"[SELFTEST] {len(results) - len(failed)}/{len(results)} checks passed")
    return 1 if failed else 0


if __name__ == "__main__":
    # Paths may carry characters a cp936 console cannot encode; a reporter that
    # dies mid-report would turn a real finding into a silent pass.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(errors="backslashreplace")
    sys.exit(main())
