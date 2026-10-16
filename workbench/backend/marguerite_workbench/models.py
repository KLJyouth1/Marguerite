"""Pydantic v2 data models — defined exactly as the shared API contract.

These are the single source of truth shared between ``api.py`` and the test
suite.  ``id`` / ``created_at`` on :class:`MemoryItem` carry defaults so the
very same model can be used both as the store *request* body (where the
contract omits them) and the *response* body.
"""

from __future__ import annotations

from typing import Optional

from pydantic import BaseModel, Field


class FileSection(BaseModel):
    id: str
    title: Optional[str] = None
    start_line: int
    end_line: int
    kind: str  # heading|function|class|block|raw
    collapsed: bool = False
    preview: str  # first ~12 lines
    full: Optional[str] = None  # null when collapsed


class FileCard(BaseModel):
    file_id: str
    path: str
    language: Optional[str] = None
    size_bytes: int
    sha256: str
    sections: list[FileSection] = Field(default_factory=list)
    meta: dict = Field(default_factory=dict)  # {lines, encoding, has_binary, summary}


class MemoryItem(BaseModel):
    id: str = ""
    layer: str = ""  # L1..L7
    kind: str = ""
    content: str = ""
    embedding: Optional[list[float]] = None
    created_at: str = ""
    refs: list[str] = Field(default_factory=list)


class ExpertSkill(BaseModel):
    name: str
    description: str
    procedure: list[str] = Field(default_factory=list)


class ExpertRole(BaseModel):
    role: str  # product_reviewer|security_officer|qa_lead|designer|investigator|*_lead|*_member
    title: str
    team: Optional[str] = None  # team id for the eight expert teams; null for the core distilled roles
    distilled_from: list[str] = Field(default_factory=list)
    problem_solving_template: str = ""  # chain-of-thought steps (newline separated)
    skills: list[ExpertSkill] = Field(default_factory=list)
    experience_notes: str = ""


class ExpertAnalysis(BaseModel):
    role: str
    verdict: str  # pass|conditional|fail
    findings: list[str] = Field(default_factory=list)
    recommendations: list[str] = Field(default_factory=list)
    confidence: float = 0.0  # 0..1


class HarnessAdapter(BaseModel):
    id: str  # codex_cli|opencode|aider|internal
    name: str
    available: bool = False
    command: Optional[str] = None
    best_practices: list[str] = Field(default_factory=list)


class HarnessResult(BaseModel):
    adapter: str
    task: str
    steps: list[str] = Field(default_factory=list)
    diffs: list[str] = Field(default_factory=list)
    status: str  # ok|partial|error
    duration_ms: int = 0


class ResearchResult(BaseModel):
    query: str
    results: list[dict] = Field(default_factory=list)  # {title,url,snippet}
    synthesized: str = ""


class LayerStatus(BaseModel):
    layer: str
    name: str
    count: int
    capacity: int


class KbSearchHit(BaseModel):
    text: str
    score: float
    source: str


class KbIngestResult(BaseModel):
    indexed_chunks: int


class DisplayResult(BaseModel):
    html: str
    language: str


class ModifyResult(BaseModel):
    new_content: str
    diff: str
    backup_ref: Optional[str] = None
    applied: bool


class AskResult(BaseModel):
    answer: str
    citations: list[dict] = Field(default_factory=list)


class InteractResult(BaseModel):
    reply: str
    session_id: str


class MemoryQueryResult(BaseModel):
    items: list[MemoryItem] = Field(default_factory=list)
