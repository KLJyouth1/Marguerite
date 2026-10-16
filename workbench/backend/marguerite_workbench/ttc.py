"""Test-Time Compute — self-consistency, best-of-N and adaptive budgets.

Design (see docs/UPGRADE_PLAN_V2.md §支柱B):

* **Self-Consistency**: draw N candidate answers, normalise each into a vote
  key, aggregate by majority.  Agreement = top-vote share.
* **Best-of-N**: generate N candidates and rank them with a pluggable scorer
  (length-aware default; callers may inject domain verifiers).
* **AdaptiveBudget**: map a difficulty heuristic to sampling/loop parameters,
  honouring the DeepSeek thinking-mode constraint table (thinking-style tasks
  get low temperature; creative ones get higher temperature + nucleus).

Every path routes through :func:`marguerite_workbench.llm.llm_or_fallback`
discipline: with ``UNIV_LLM_*`` configured the real model is sampled N times;
without keys the deterministic fallback generates candidates from the local
knowledge base and the result is honestly tagged ``engine="fallback"`` — the
offline invariant is preserved without faking model output.
"""

from __future__ import annotations

import logging
import re
from dataclasses import dataclass, field
from typing import Callable

from marguerite_workbench import llm as llm_mod

logger = logging.getLogger("marguerite.ttc")

Difficulty = str  # "low" | "medium" | "high"

# Sampling presets aligned with the plan's policy table (§2.5 of the harness spec).
SAMPLING_PRESETS: dict[str, dict[str, float]] = {
    "code": {"temperature": 0.0, "top_p": 1.0},          # greedy for determinism
    "reasoning": {"temperature": 0.3, "top_p": 0.95},
    "creative": {"temperature": 0.8, "top_p": 0.95},
}

# Difficulty → compute budget (rounds / candidates / effort hint).
BUDGET_TABLE: dict[Difficulty, dict[str, int | str]] = {
    "low": {"n_candidates": 1, "max_rounds": 4, "effort": "low"},
    "medium": {"n_candidates": 3, "max_rounds": 8, "effort": "medium"},
    "high": {"n_candidates": 5, "max_rounds": 12, "effort": "high"},
}

_HIGH_SIGNALS = re.compile(
    r"\b(prove|derive|optimi[sz]e|architecture|migrat|debug|concurrency|"
    r"race|deadlock|security|threat|refactor|distributed|一致性|证明|推导|并发|安全)\b",
    re.IGNORECASE,
)
_MED_SIGNALS = re.compile(r"\b(explain|compare|design|summar\w*|review|分析|对比|设计|总结)\b", re.IGNORECASE)


@dataclass
class TtcResult:
    strategy: str
    engine: str                 # "llm" | "fallback"
    answer: str
    candidates: list[str] = field(default_factory=list)
    votes: dict[str, int] = field(default_factory=dict)
    scores: list[float] = field(default_factory=list)
    agreement: float = 0.0      # top-vote share for sc; best score for bon
    meta: dict = field(default_factory=dict)
    thinking: list[str] = field(default_factory=list)  # reasoning_content traces (llm engine only)

    def to_dict(self) -> dict:
        return {
            "strategy": self.strategy, "engine": self.engine, "answer": self.answer,
            "candidates": self.candidates, "votes": self.votes, "scores": self.scores,
            "agreement": round(self.agreement, 4), "meta": self.meta,
            "thinking": [t[:400] for t in self.thinking],
        }


# ---------------------------------------------------------------------------
# difficulty → budget
# ---------------------------------------------------------------------------
def estimate_difficulty(task: str) -> Difficulty:
    """Cheap lexical heuristic; deliberately transparent (no hidden magic)."""
    if _HIGH_SIGNALS.search(task) or len(task) > 400:
        return "high"
    if _MED_SIGNALS.search(task) or len(task) > 120:
        return "medium"
    return "low"


def budget_for(task: str) -> dict:
    difficulty = estimate_difficulty(task)
    budget = dict(BUDGET_TABLE[difficulty])
    budget["difficulty"] = difficulty
    return budget


def preset_for(kind: str) -> dict[str, float]:
    try:
        return dict(SAMPLING_PRESETS[kind])
    except KeyError as exc:
        raise ValueError(f"unknown preset kind {kind!r}; choose from {sorted(SAMPLING_PRESETS)}") from exc


# ---------------------------------------------------------------------------
# candidate generation (LLM or honest fallback)
# ---------------------------------------------------------------------------
def _normalise(answer: str) -> str:
    """Vote key: collapse whitespace/punctuation/case so semantically identical
    answers merge (string-level approximation of semantic voting)."""
    return re.sub(r"[^\w]+", " ", answer.strip().lower()).strip()


def _fallback_candidates(prompt: str, n: int, kb=None) -> list[str]:
    """Deterministic candidates from the local KB when no LLM is configured.

    Each candidate is the best-matching KB chunk prefixed by its source, which
    makes the majority vote measure *retrieval consensus* — real signal, never
    fabricated.
    """
    hits = []
    if kb is not None:
        try:
            hits = kb.search(prompt, n) or []
        except Exception as exc:  # noqa: BLE001 — degrade loudly, then continue
            logger.warning("kb fallback search failed: %s", exc)
    if not hits:
        return [f"no evidence available for: {prompt[:80]}"] * n
    cands = []
    for i in range(n):
        hit = hits[i % len(hits)]
        cands.append(f"[{hit['source']}] {hit['text'][:200]}")
    return cands


def _generate_n(prompt: str, n: int, kb, temperature_by_index: Callable[[int], float]) -> tuple[list[str], str, list[str]]:
    """Generate N candidates; returns (candidates, engine_tag, thinking_traces)."""
    client = llm_mod.get_client()
    if client.enabled:
        out: list[str] = []
        traces: list[str] = []
        for i in range(n):
            try:
                content, thinking = client.complete_with_trace(
                    prompt, temperature=temperature_by_index(i)
                )
                out.append(content)
                # DeepSeek thinking-mode contract: traces are display-only and
                # never re-enter the conversation history.
                if thinking:
                    traces.append(thinking)
            except Exception as exc:  # noqa: BLE001 — one bad call must not kill the batch
                logger.warning("candidate %d/%d failed: %s", i + 1, n, exc)
        if out:
            return out, "llm", traces
    return _fallback_candidates(prompt, n, kb), "fallback", []


# ---------------------------------------------------------------------------
# strategies
# ---------------------------------------------------------------------------
def self_consistency(prompt: str, n: int = 3, kb=None) -> TtcResult:
    """Majority vote over N independent completions."""
    n = max(1, min(n, 8))
    cands, engine, thinking = _generate_n(prompt, n, kb, lambda i: 0.7)
    votes: dict[str, int] = {}
    key_of: dict[str, str] = {}
    for c in cands:
        key = _normalise(c)[:120]
        votes[key] = votes.get(key, 0) + 1
        key_of.setdefault(key, c)
    top_key = max(votes, key=lambda k: votes[k])
    total = sum(votes.values())
    return TtcResult(
        strategy="self_consistency", engine=engine, answer=key_of[top_key],
        candidates=cands, votes=votes, agreement=votes[top_key] / total,
        meta={"n": n}, thinking=thinking,
    )


def _default_scorer(candidate: str) -> float:
    """Transparent heuristic: specificity signals minus hedging filler."""
    length_score = min(len(candidate), 400) / 400
    specifics = len(re.findall(r"\d|[A-Za-z_]{4,}", candidate)) / max(1, len(candidate.split()))
    hedge = len(re.findall(r"\b(maybe|probably|not sure|i think|可能|也许)\b", candidate, re.I))
    return max(0.0, 0.5 * length_score + 0.5 * min(1.0, specifics) - 0.1 * hedge)


def best_of_n(prompt: str, n: int = 3, scorer: Callable[[str], float] | None = None, kb=None) -> TtcResult:
    """Generate N candidates and keep the scorer's arg-max."""
    n = max(1, min(n, 8))
    scorer = scorer or _default_scorer
    cands, engine, thinking = _generate_n(prompt, n, kb, lambda i: 0.9 if i else 0.2)
    scored = [float(scorer(c)) for c in cands]
    best_i = max(range(len(cands)), key=lambda i: scored[i])
    return TtcResult(
        strategy="best_of_n", engine=engine, answer=cands[best_i],
        candidates=cands, scores=scored, agreement=max(scored),
        meta={"n": n, "scorer": getattr(scorer, "__name__", "custom")}, thinking=thinking,
    )


def run_strategy(strategy: str, prompt: str, n: int = 3, kb=None) -> TtcResult:
    """Table-driven dispatch (anti R1): adding a strategy is a registry entry."""
    registry: dict[str, Callable[..., TtcResult]] = {
        "self_consistency": self_consistency,
        "best_of_n": best_of_n,
    }
    try:
        fn = registry[strategy]
    except KeyError as exc:
        raise ValueError(f"unknown strategy {strategy!r}; choose from {sorted(registry)}") from exc
    return fn(prompt, n=n, kb=kb)
