"""Runtime configuration for the Marguerite File Intelligence Workbench.

All directories are resolved from environment variables so tests can point the
service at an isolated temporary workspace / data directory.  Reads happen at
call-time (not import-time) so fixtures that set ``MARGERITE_*`` env vars before
constructing the :class:`Workbench` take effect cleanly.
"""

from __future__ import annotations

import os
from pathlib import Path

# ---------------------------------------------------------------------------
# Default locations (overridable via env).  ``__file__`` lives in the package
# root, so a bare ``pip install`` run still has a sensible data dir.
# ---------------------------------------------------------------------------
_PACKAGE_ROOT = Path(__file__).resolve().parent
_DEFAULT_DATA = _PACKAGE_ROOT / "data"
_DEFAULT_WORKSPACE = Path(os.environ.get("MARGERITE_WORKSPACE", Path.cwd()))

# Hard-coded constants (anti R5: no magic strings/numbers without a name).
ENV_DATA_DIR = "MARGERITE_DATA_DIR"
ENV_WORKSPACE = "MARGERITE_WORKSPACE"


def data_dir() -> Path:
    """Return (creating if needed) the directory used for JSONL persistence."""
    p = Path(os.environ.get(ENV_DATA_DIR, _DEFAULT_DATA))
    p.mkdir(parents=True, exist_ok=True)
    return p


def workspace_root() -> Path:
    """Return (creating if needed) the sandbox root all file paths resolve into."""
    p = Path(os.environ.get(ENV_WORKSPACE, _DEFAULT_WORKSPACE))
    p.mkdir(parents=True, exist_ok=True)
    return p
