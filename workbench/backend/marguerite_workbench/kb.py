"""Dependency-light knowledge base: chunk + BM25 + local vector hybrid search.

Mirrors Marguerite's ``core/src/kb/store.rs`` BM25 implementation (same tokeniser,
same k1/b parameters) and augments it with the shared ``LocalEmbedding`` so the
``/kb/search`` endpoint returns a *hybrid* (BM25 + cosine) ranking.  All state
is persisted to JSONL under ``data/`` and rebuilt in-memory on load.
"""

from __future__ import annotations

import json
import logging
import math
from collections import defaultdict
from pathlib import Path
from typing import Optional

from marguerite_workbench.memory import VEC_DIM, cosine, embed

logger = logging.getLogger("marguerite.kb")

# BM25 parameters (identical to the Rust reference implementation).
_K1 = 1.5
_B = 0.75
_CHUNK_SIZE = 400
_CHUNK_OVERLAP = 80

# Hybrid score weights (named constants, anti R5).
_W_BM25 = 0.6
_W_VECTOR = 0.4


def is_cjk(c: str) -> bool:
    cp = ord(c)
    return (
        (0x4E00 <= cp <= 0x9FFF)
        or (0x3400 <= cp <= 0x4DBF)
        or (0x3040 <= cp <= 0x30FF)  # Japanese kana
        or (0xAC00 <= cp <= 0xD7AF)  # Hangul
    )


def tokenize(text: str) -> list[str]:
    """Tokeniser shared by BM25 and chunking (mirrors Rust ``kb::tokenize``)."""
    out: list[str] = []
    word = ""
    for c in text.lower():
        if is_cjk(c):
            if word:
                out.append(word)
                word = ""
            out.append(c)
        elif c.isalnum():
            word += c
        elif word:
            out.append(word)
            word = ""
    if word:
        out.append(word)
    # CJK bigrams improve Chinese recall.
    cjk_chars = [t for t in out if len(t) == 1 and is_cjk(t[0])]
    if len(cjk_chars) >= 2:
        for a, b in zip(cjk_chars, cjk_chars[1:]):
            out.append(a + b)
    return [t for t in out if t]


def chunk_text(text: str, size: int = _CHUNK_SIZE, overlap: int = _CHUNK_OVERLAP) -> list[str]:
    """Split text into retrieval chunks, keeping paragraph boundaries."""
    approx_words = max(8, size // 6)
    overlap_words = max(2, overlap // 6)
    paragraphs = [p for p in text.split("\n\n") if p.strip()]
    out: list[str] = []
    for para in paragraphs:
        if len(para) <= size:
            out.append(para)
            continue
        words = para.split()
        start = 0
        while start < len(words):
            end = min(start + approx_words, len(words))
            out.append(" ".join(words[start:end]))
            if end == len(words):
                break
            start = max(0, end - overlap_words)
    if not out:
        out.append(text)
    return out


def bm25_score(docs: list[str], query: str) -> list[tuple[int, float]]:
    """Pure BM25 over a list of documents; returns ``(doc_index, score)`` desc."""
    q_tokens = tokenize(query)
    if not q_tokens or not docs:
        return []
    n = len(docs)
    df: defaultdict[str, float] = defaultdict(float)
    tfs: list[dict[str, float]] = []
    lens: list[float] = []
    for text in docs:
        toks = tokenize(text)
        tf: dict[str, float] = defaultdict(float)
        for t in toks:
            tf[t] += 1.0
            df[t] += 1.0
        lens.append(len(toks))
        tfs.append(tf)
    avgdl = sum(lens) / max(n, 1)
    scored: list[tuple[int, float]] = []
    for i, tf in enumerate(tfs):
        score = 0.0
        for qt in q_tokens:
            if qt in df:
                idf = math.log((n - df[qt] + 0.5) / (df[qt] + 0.5) + 1.0)
                f = tf.get(qt, 0.0)
                if f > 0.0:
                    score += idf * (f * (_K1 + 1.0)) / (
                        f + _K1 * (1.0 - _B + _B * (lens[i] / avgdl))
                    )
        if score > 0.0:
            scored.append((i, score))
    scored.sort(key=lambda x: -x[1])
    return scored


def _minmax(values: list[float]) -> list[float]:
    if not values:
        return []
    lo, hi = min(values), max(values)
    if hi == lo:
        return [1.0 for _ in values]
    return [(v - lo) / (hi - lo) for v in values]


class KnowledgeBase:
    """In-memory BM25 + vector KB, persisted to JSONL."""

    def __init__(self, path: str | Path) -> None:
        self.path = Path(path)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.chunks: list[dict] = []
        self._next = 0
        self._load()

    def _load(self) -> None:
        if not self.path.exists():
            return
        with self.path.open("r", encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    self.chunks.append(json.loads(line))
                    self._next = max(self._next, self._chunk_seq() + 1)
                except json.JSONDecodeError as exc:  # R3: skip, log
                    logger.warning("skipping corrupt kb line: %s", exc)

    def _chunk_seq(self) -> int:
        seq = 0
        for c in self.chunks:
            cid = c.get("id", "")
            if cid.startswith("chunk_"):
                try:
                    seq = max(seq, int(cid[len("chunk_"):]))
                except ValueError:
                    pass
        return seq

    def _save(self) -> None:
        with self.path.open("w", encoding="utf-8") as fh:
            for c in self.chunks:
                fh.write(json.dumps(c, ensure_ascii=False) + "\n")

    # -- ingestion ----------------------------------------------------------
    def ingest(self, text: str, doc_id: str, title: Optional[str] = None, source: Optional[str] = None) -> int:
        title = title or doc_id
        source = source or doc_id
        added = 0
        for piece in chunk_text(text):
            cid = f"chunk_{self._next}"
            self._next += 1
            self.chunks.append(
                {
                    "id": cid,
                    "doc_id": doc_id,
                    "title": title,
                    "text": piece,
                    "embedding": embed(piece),
                    "source": source,
                }
            )
            added += 1
        self._save()
        return added

    # -- retrieval ----------------------------------------------------------
    def search(self, q: str, k: int = 5) -> list[dict]:
        if not self.chunks:
            return []
        docs = [c["text"] for c in self.chunks]
        bm25 = dict(bm25_score(docs, q))
        qv = embed(q)
        vec_scores = [cosine(qv, c.get("embedding") or embed(c["text"])) for c in self.chunks]

        norm_bm25 = _minmax([bm25.get(i, 0.0) for i in range(len(docs))])
        norm_vec = _minmax(vec_scores)

        combined = []
        for i, chunk in enumerate(self.chunks):
            score = _W_BM25 * norm_bm25[i] + _W_VECTOR * norm_vec[i]
            if score <= 0.0:
                continue
            combined.append((score, chunk))
        combined.sort(key=lambda x: -x[0])
        return [
            {"text": c["text"], "score": round(score, 4), "source": c.get("source", c["doc_id"])}
            for score, c in combined[:k]
        ]

    def document_count(self) -> int:
        return len({c["doc_id"] for c in self.chunks})

    def chunk_count(self) -> int:
        return len(self.chunks)
