"""Sampling strategies over raw logits — the final link of the LLM math chain.

Implements the exact policy table from the upgrade plan:

| policy            | mechanism                                        |
|-------------------|--------------------------------------------------|
| greedy            | argmax(logits)                                   |
| temperature       | softmax(logits / T); T→0 converges to greedy     |
| top-k             | keep only the k highest-probability tokens       |
| top-p (nucleus)   | smallest set with cumulative prob >= p           |
| freq/pres penalty | logits -= f*count + p*(count>0), applied pre-T   |

All functions are pure (logits in, token id out given an RNG seed) so tests can
assert exact distributions, and the Model Lab can replay identical traces.
"""

from __future__ import annotations

import math
import random


def apply_penalties(
    logits: list[float],
    counts: dict[int, int] | None,
    frequency_penalty: float = 0.0,
    presence_penalty: float = 0.0,
) -> list[float]:
    """OpenAI-style penalties computed from the running token tally."""
    if not counts or (frequency_penalty == 0.0 and presence_penalty == 0.0):
        return list(logits)
    return [
        v - frequency_penalty * counts.get(i, 0) - (presence_penalty if i in counts else 0.0)
        for i, v in enumerate(logits)
    ]


def _softmax(row: list[float]) -> list[float]:
    m = max(row)
    exps = [math.exp(v - m) for v in row]
    s = sum(exps)
    return [e / s for e in exps]


def truncate_top_k(probs: list[float], k: int) -> list[float]:
    """Zero out everything outside the k largest entries (ties keep the lower index)."""
    if k <= 0:
        raise ValueError("top_k must be >= 1")
    order = sorted(range(len(probs)), key=lambda i: probs[i], reverse=True)[:k]
    keep = set(order)
    return [p if i in keep else 0.0 for i, p in enumerate(probs)]


def truncate_top_p(probs: list[float], p: float) -> list[float]:
    """Smallest prefix of probability-sorted tokens whose mass >= p is kept."""
    if not 0.0 < p <= 1.0:
        raise ValueError("top_p must be in (0, 1]")
    order = sorted(range(len(probs)), key=lambda i: probs[i], reverse=True)
    out = [0.0] * len(probs)
    cum = 0.0
    for idx in order:
        out[idx] = probs[idx]
        cum += probs[idx]
        if cum >= p:
            break
    return out


def renormalise(probs: list[float]) -> list[float]:
    total = sum(probs)
    if total <= 0.0:
        raise ValueError("probability mass collapsed to zero after truncation")
    return [v / total for v in probs]


def sample(
    logits: list[float],
    *,
    temperature: float = 1.0,
    top_k: int | None = None,
    top_p: float | None = None,
    frequency_penalty: float = 0.0,
    presence_penalty: float = 0.0,
    counts: dict[int, int] | None = None,
    rng: random.Random | None = None,
) -> int:
    """Draw one token id. temperature=0 (or a numerically tiny value) → greedy."""
    rng = rng or random.Random()
    penalised = apply_penalties(logits, counts, frequency_penalty, presence_penalty)
    if temperature <= 1e-6:
        return max(range(len(penalised)), key=lambda i: penalised[i])
    scaled = [v / temperature for v in penalised]
    probs = _softmax(scaled)
    if top_k is not None and top_k < len(probs):
        probs = truncate_top_k(probs, top_k)
    if top_p is not None:
        probs = truncate_top_p(probs, top_p)
    probs = renormalise(probs)
    roll = rng.random()
    cum = 0.0
    for i, p in enumerate(probs):
        cum += p
        if roll < cum:
            return i
    # Numerical residue: return the last nonzero-probability token.
    live = [i for i, p in enumerate(probs) if p > 0.0]
    return live[-1]


def make_sampler(seed: int = 0, **kwargs):
    """Curry :func:`sample` into the callable TinyGPT.generate expects."""
    rng = random.Random(seed)

    def draw(logits: list[float], counts: dict[int, int] | None = None) -> int:
        return sample(logits, counts=counts, rng=rng, **kwargs)

    return draw
