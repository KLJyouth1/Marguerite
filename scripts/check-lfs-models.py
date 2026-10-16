#!/usr/bin/env python3
"""Packaging guard for the SquillaRouter V4 Phase-3 artifact bundle.

Historical defect (2026-10-12, found in a user's log panel): the bundle shipped in the
desktop installer contained **Git LFS pointer files** instead of the real models. Every
artifact was ~130 bytes where the manifest requires megabytes (``bge_onnx/model.onnx``
was 133 bytes vs. 23,902,398), so the V4 router silently degraded to the safe fallback and
local ML routing was disabled — the installer looked fine while a feature quietly vanished.

This guard makes packaging **fail closed** by verifying each artifact against
``artifact_manifest.json`` (existence, size, and sha256). A pointer-only check is not
enough: it would miss truncated or wrong-version binaries.

Usage:
    python scripts/check-lfs-models.py [model_root ...] [--manifest PATH] [--size-only]

Exit codes: 0 = all artifacts verified, 1 = at least one artifact missing/mismatched.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

LFS_MAGIC = b"git-lfs.github.com/spec/v1"
MANIFEST_NAME = "artifact_manifest.json"

# Pointer files are tiny; anything larger cannot be a pointer.
MAX_POINTER_BYTES = 1024

# Default bundle root (the one that must be real binaries).
DEFAULT_ROOT = "pyengine/src/opensquilla/squilla_router/models/v4.2_phase3_inference"


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def _is_pointer(path: Path) -> bool:
    try:
        if path.stat().st_size > MAX_POINTER_BYTES:
            return False
        return LFS_MAGIC in path.read_bytes()[:MAX_POINTER_BYTES]
    except OSError:
        return False


def find_manifest(root: Path) -> Path | None:
    """Locate the bundle manifest: inside ``root`` or in its nearest parent."""
    for cand in [root, *root.parents]:
        m = cand / MANIFEST_NAME
        if m.is_file():
            return m
    return None


def verify_against_manifest(
    manifest: Path, root: Path, size_only: bool = False
) -> tuple[list[str], list[str]]:
    """Verify every manifest entry. Returns (ok_details, problems)."""
    try:
        spec = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        return [], [f"manifest unreadable/non-JSON: {manifest}: {exc}"]

    entries = spec.get("files") or []
    if not isinstance(entries, list):
        return [], [f"manifest has no 'files' list: {manifest}"]

    ok: list[str] = []
    problems: list[str] = []
    for entry in entries:
        rel = entry.get("path", "")
        want_size = entry.get("size_bytes")
        want_sha = entry.get("sha256")
        p = root / rel
        if not p.is_file():
            problems.append(f"missing: {p}")
            continue
        actual_size = p.stat().st_size
        if want_size is not None and actual_size != want_size:
            if _is_pointer(p):
                problems.append(
                    f"lfs-pointer: {p} ({actual_size} B — a Git LFS pointer, not the artifact; "
                    f"expected {want_size:,} B). Run `git lfs pull`."
                )
            else:
                problems.append(
                    f"size-mismatch: {p} ({actual_size:,} B != manifest {want_size:,} B)"
                )
            continue
        if want_sha and not size_only:
            actual_sha = _sha256(p)
            if actual_sha != want_sha:
                problems.append(
                    f"sha256-mismatch: {p}\n      actual {actual_sha}\n      expect {want_sha}"
                )
                continue
        ok.append(f"{rel} ({actual_size:,} B)")
    return ok, problems


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("roots", nargs="*", help="bundle root(s) to verify")
    ap.add_argument("--manifest", help="explicit manifest path (default: auto-detect)")
    ap.add_argument(
        "--size-only",
        action="store_true",
        help="skip sha256 (fast path; catches truncation but not bit-rot)",
    )
    args = ap.parse_args(argv)

    roots = [Path(r) for r in (args.roots or [DEFAULT_ROOT])]
    repo = Path(__file__).resolve().parent.parent
    normalized: list[Path] = []
    for r in roots:
        if r.is_dir():
            normalized.append(r)
        elif (repo / r).is_dir():
            normalized.append(repo / r)
        else:
            normalized.append(r)  # keep for the error message

    any_problem = False
    for root in normalized:
        print(f"[router-guard] bundle root: {root}")
        manifest = (
            Path(args.manifest) if args.manifest else find_manifest(root)
        )
        if manifest is None or not manifest.is_file():
            print(f"[router-guard]   no {MANIFEST_NAME} found — falling back to pointer scan")
            still_bad = any(_is_pointer(p) for p in root.rglob("*") if p.is_file())
            if still_bad:
                print("[router-guard]   ERROR: LFS pointer files present", file=sys.stderr)
                any_problem = True
            else:
                print("[router-guard]   OK (no pointers found)")
            continue

        ok, problems = verify_against_manifest(manifest, root, size_only=args.size_only)
        print(f"[router-guard]   manifest: {manifest.name} ({len(ok)} verified)")
        for p in problems:
            print(f"[router-guard]   ERROR: {p}", file=sys.stderr)
        if problems:
            any_problem = True
        else:
            print("[router-guard]   OK — artifacts are real and match the manifest")

    if any_problem:
        print(
            "\n[router-guard] PACKAGING ABORTED: artifacts are missing/stub/corrupt.\n"
            "[router-guard] The router would silently fall back to default-tier routing.\n"
            "[router-guard] Restore the real bundle (see PROVENANCE.md for provenance) "
            "and re-run packaging.",
            file=sys.stderr,
        )
        return 1
    print("\n[router-guard] OK — bundle is complete and verified.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
