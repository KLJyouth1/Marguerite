"""Version-sync guard — the workbench analogue of check-doc-cli-drift.

Verifies that every version declaration agrees with ``workbench/VERSION``:

    backend/marguerite_workbench/api.py   FastAPI(version="X.Y.Z")
    frontend/package.json                 "version": "X.Y.Z"

Exit 0 = in sync; exit 1 = drift (prints the offending pairs).  Wired into
package_workbench via ``detect_version`` and safe to run standalone in CI.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

WORKBENCH_ROOT = Path(__file__).resolve().parents[1]
VERSION_FILE = WORKBENCH_ROOT / "VERSION"
_SOURCES = {
    "backend api.py": (
        WORKBENCH_ROOT / "backend" / "marguerite_workbench" / "api.py",
        re.compile(r'version="([0-9]+\.[0-9]+\.[0-9]+)"'),
    ),
    "frontend package.json": (
        WORKBENCH_ROOT / "frontend" / "package.json",
        None,  # parsed as JSON below
    ),
}


def main() -> int:
    declared = VERSION_FILE.read_text(encoding="utf-8").strip()
    drift: list[str] = []
    for label, (path, pattern) in _SOURCES.items():
        if not path.exists():
            drift.append(f"{label}: file missing ({path})")
            continue
        if pattern is None:
            actual = json.loads(path.read_text(encoding="utf-8")).get("version", "")
        else:
            match = pattern.search(path.read_text(encoding="utf-8"))
            actual = match.group(1) if match else ""
        status = "OK" if actual == declared else "DRIFT"
        print(f"[{status}] {label:<24} {actual or '<none>'} (expected {declared})")
        if actual != declared:
            drift.append(label)
    if drift:
        print(f"version drift detected in: {', '.join(drift)}")
        return 1
    print(f"all versions in sync at {declared}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
