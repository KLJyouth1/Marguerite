from __future__ import annotations

import sys
from pathlib import Path

_ROOT = Path(__file__).parent.parent


def test_start_sh_exists() -> None:
    assert (_ROOT / "start.sh").exists(), "start.sh must exist at repo root"


def test_start_ps1_exists() -> None:
    assert (_ROOT / "start.ps1").exists(), "start.ps1 must exist at repo root"


def test_start_sh_is_executable() -> None:
    if sys.platform.startswith("win"):
        return  # git mode bits not enforced on Windows
    mode = (_ROOT / "start.sh").stat().st_mode
    assert mode & 0o111 != 0, "start.sh must have executable bit set"
