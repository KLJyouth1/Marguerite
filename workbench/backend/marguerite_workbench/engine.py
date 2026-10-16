"""Workbench engine wiring — the C-layer context shared by every front door.

Owns the workspace sandbox root plus all engine instances (memory bridge,
knowledge base, file intelligence) for one process.  Both the FastAPI app
(``api.py``) and the CLI (``cli.py``) construct exactly one :class:`Workbench`,
so every surface produces identical real engine output with no duplicated
wiring logic.

Kept deliberately free of any web-framework imports: a ``python -m
marguerite_workbench …`` invocation should not pay for (or log noise from)
FastAPI/Starlette unless it actually serves HTTP.
"""

from __future__ import annotations

from pathlib import Path

from marguerite_workbench.file_intel import FileIntel
from marguerite_workbench.kb import KnowledgeBase
from marguerite_workbench.memory import MemoryEngine
from marguerite_workbench import research


class Workbench:
    """Owns the shared workspace + all engine instances for one process."""

    def __init__(self, workspace: Path, data_dir: Path) -> None:
        self.workspace = Path(workspace)
        self.data_dir = Path(data_dir)
        self.workspace.mkdir(parents=True, exist_ok=True)
        self.data_dir.mkdir(parents=True, exist_ok=True)
        self.memory = MemoryEngine(self.data_dir / "memory.jsonl")
        self.kb = KnowledgeBase(self.data_dir / "kb.jsonl")
        self.files = FileIntel(self.workspace, self.data_dir, self.kb, self.memory)
        research.set_kb(self.kb)
