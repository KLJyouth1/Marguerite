"""Dependency-free inspection of the copied V4 Phase 3 router bundle.

Split out of :mod:`opensquilla.squilla_router.v4_phase3` on purpose: the
gateway's boot-time asset validation is guarded against importing the ML
adapter module (it must not pull lightgbm/torch), yet it still needs to tell an
operator *why* ML routing is degraded. This module imports only the stdlib plus
:mod:`opensquilla.router_runtime_diagnostics`, so both surfaces can share one
verdict.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from opensquilla.router_runtime_diagnostics import (
    ROUTER_ASSETS_MISSING,
    ROUTER_WEIGHTS_NOT_FETCHED,
    router_runtime_hint,
)

#: Magic prefix of a Git LFS pointer file (42 bytes).
GIT_LFS_POINTER_PREFIX = b"version https://git-lfs.github.com/spec/v1"

#: Files the bundle layout must contain before anything else is worth checking.
REQUIRED_BUNDLE_FILES = ("runtime_src", "router.runtime.yaml")


def default_bundle_dir() -> Path:
    """Return the repository-bundled V4 Phase 3 runtime asset directory."""

    return Path(__file__).resolve().parent / "models" / "v4.2_phase3_inference"


@dataclass(frozen=True)
class ManifestScan:
    """Structural verdict on the bundle's ``artifact_manifest.json`` contents.

    Pure data: this never imports lightgbm/torch and never raises, so the
    doctor surface can report on a broken bundle instead of crashing on it.
    ``problems`` keeps the human-readable strings in manifest order (the same
    order the pre-existing error message used); ``pointer_assets`` is the
    subset that specifically proves the weights were never pulled.
    """

    manifest_present: bool
    checked: int
    problems: tuple[str, ...]
    pointer_assets: tuple[str, ...]


def scan_manifest(bundle_dir: Path) -> ManifestScan:
    """Verify every manifest entry against the bytes on disk.

    Distinguishes the three failure shapes that used to collapse into one
    opaque "incomplete bundle" error: file absent, Git LFS pointer standing in
    for the real artifact, and size drift against the manifest.
    """

    manifest_path = bundle_dir / "artifact_manifest.json"
    if not manifest_path.exists():
        return ManifestScan(False, 0, (), ())

    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        return ManifestScan(True, 0, (f"artifact_manifest.json: unreadable ({exc})",), ())

    problems: list[str] = []
    pointers: list[str] = []
    checked = 0
    for entry in manifest.get("files", []):
        raw_path = str(entry.get("path") or "")
        if not raw_path:
            continue
        rel_path = Path(raw_path)
        if rel_path.is_absolute() or ".." in rel_path.parts:
            problems.append(f"{raw_path}: invalid manifest path")
            continue

        path = bundle_dir / rel_path
        if not path.exists():
            problems.append(f"{raw_path}: missing")
            continue

        # Pointer detection MUST precede the size check. A Git LFS pointer is
        # always smaller than the artifact it stands in for, so a size-first
        # order reports it as "size 133 != manifest 23902398" — which is true
        # but useless: it hides the fact that one `git lfs pull` fixes it.
        # (That ordering bug is why the shipped bundle logged a generic
        # `router_runtime_unavailable` instead of a weights-missing diagnosis.)
        with path.open("rb") as handle:
            if handle.read(len(GIT_LFS_POINTER_PREFIX)) == GIT_LFS_POINTER_PREFIX:
                problems.append(f"{raw_path}: Git LFS pointer, not model data")
                pointers.append(raw_path)
                continue

        actual_size = path.stat().st_size
        expected_size = entry.get("size_bytes")
        if isinstance(expected_size, int) and actual_size != expected_size:
            problems.append(f"{raw_path}: size {actual_size} != manifest {expected_size}")
            continue

        checked += 1

    return ManifestScan(True, checked, tuple(problems), tuple(pointers))


def read_bundle_version(bundle_dir: Path) -> str:
    """Best-effort bundle version string (``"unknown"`` when undeterminable)."""

    for name in ("version.json", "inference_manifest.json"):
        path = bundle_dir / name
        if not path.exists():
            continue
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        for key in ("version", "model_version", "bundle_version"):
            value = data.get(key)
            if value:
                return str(value)
    return "unknown"


def inspect_bundle(bundle_dir: str | Path | None = None) -> dict[str, Any]:
    """Read-only verdict on the V4 bundle sitting on disk.

    The single source both the gateway doctor and the ML adapter report from,
    so an operator sees *why* ML routing is degraded without importing the ML
    stack (which is exactly what is broken).

    Keys: ``bundleDir`` / ``present`` / ``available`` / ``errorKind`` /
    ``hint`` / ``problemCount`` / ``problems`` / ``checkedAssets`` /
    ``pointerAssets`` / ``modelVersion``.
    """

    bundle = Path(bundle_dir) if bundle_dir else default_bundle_dir()
    present = bundle.is_dir()
    missing_required = [name for name in REQUIRED_BUNDLE_FILES if not (bundle / name).exists()]
    scan = scan_manifest(bundle)

    problems: list[str] = []
    if not present:
        problems.append(f"bundle directory not found: {bundle}")
    if missing_required:
        problems.append("missing V4 bundle files: " + ", ".join(missing_required))
    problems.extend(scan.problems)

    if not present or missing_required:
        error_kind: str | None = ROUTER_ASSETS_MISSING
    elif scan.pointer_assets:
        error_kind = ROUTER_WEIGHTS_NOT_FETCHED
    elif problems:
        error_kind = ROUTER_ASSETS_MISSING
    else:
        error_kind = None

    return {
        "bundleDir": str(bundle),
        "present": present,
        "available": error_kind is None,
        "errorKind": error_kind,
        "hint": router_runtime_hint(error_kind) if error_kind else "",
        "problemCount": len(problems),
        "problems": problems[:5],
        "checkedAssets": scan.checked,
        "pointerAssets": list(scan.pointer_assets[:5]),
        "modelVersion": read_bundle_version(bundle),
    }
