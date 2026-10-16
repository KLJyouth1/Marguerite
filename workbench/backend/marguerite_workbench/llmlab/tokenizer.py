"""Byte-level BPE tokenizer — a faithful, dependency-free minbpe re-implementation.

Semantics follow karpathy/minbpe: the base vocabulary is the 256 byte values;
training repeatedly finds the most frequent adjacent token pair and merges it
into a fresh id (256, 257, …), recording every merge so ``encode`` can replay
exactly the learned compression on unseen text (pairs not in the merge table
are left untouched).

The :meth:`BPETokenizer.train` / :meth:`encode` / :meth:`decode` triad is
deterministic and fully offline; ``encode_trace`` additionally reports which
merge fired at each step for the workbench's Model Lab visualisation.
"""

from __future__ import annotations

from dataclasses import dataclass, field


Pair = tuple[int, int]


def _counts(ids: list[int]) -> dict[Pair, int]:
    counts: dict[Pair, int] = {}
    for pair in zip(ids, ids[1:]):
        counts[pair] = counts.get(pair, 0) + 1
    return counts


def _merge(ids: list[int], pair: Pair, index: int) -> list[int]:
    out: list[int] = []
    i = 0
    while i < len(ids):
        if i < len(ids) - 1 and ids[i] == pair[0] and ids[i + 1] == pair[1]:
            out.append(index)
            i += 2
        else:
            out.append(ids[i])
            i += 1
    return out


@dataclass
class BPETokenizer:
    """Trained merge table + reversible byte vocabulary."""

    merges: dict[Pair, int] = field(default_factory=dict)
    vocab: dict[int, bytes] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if not self.vocab:
            self.vocab = {i: bytes([i]) for i in range(256)}
            for (a, b), idx in self.merges.items():
                left, right = self.vocab[a], self.vocab[b]
                self.vocab[idx] = left + right

    # ------------------------------------------------------------------ train
    @classmethod
    def train(cls, text: str, vocab_size: int = 276) -> "BPETokenizer":
        """Learn ``vocab_size - 256`` merges from ``text``.

        Tie-breaking mirrors minbpe: scan the count dict (insertion order =
        first-seen order) and take the first pair with the maximal count.
        """
        if vocab_size < 256:
            raise ValueError("vocab_size must be >= 256 (byte base vocabulary)")
        ids = list(text.encode("utf-8"))
        target_merges = vocab_size - 256
        merges: dict[Pair, int] = {}
        for step in range(target_merges):
            counts = _counts(ids)
            if not counts:
                break  # nothing left to merge (corpus exhausted)
            # dict iteration is insertion order (= first-seen), and max() keeps
            # the earliest of ties — exactly minbpe's deterministic tie-break.
            best_pair = max(counts, key=lambda p: counts[p])
            index = 256 + step
            ids = _merge(ids, best_pair, index)
            merges[best_pair] = index
        return cls(merges=merges)

    # ------------------------------------------------------------------ apply
    def encode(self, text: str) -> list[int]:
        ids = list(text.encode("utf-8"))
        while len(ids) >= 2:
            counts = _counts(ids)
            # Replay learned merges in rank order (lowest merged id first).
            pair = min(counts, key=lambda p: self.merges.get(p, float("inf")))
            if pair not in self.merges:
                break
            ids = _merge(ids, pair, self.merges[pair])
        return ids

    def decode(self, ids: list[int]) -> str:
        raw = b"".join(self.vocab[i] for i in ids)
        return raw.decode("utf-8", errors="replace")

    def encode_trace(self, text: str) -> dict:
        """Encode plus the ordered list of merge operations that fired."""
        ids = list(text.encode("utf-8"))
        trace: list[dict] = []
        while len(ids) >= 2:
            counts = _counts(ids)
            pair = min(counts, key=lambda p: self.merges.get(p, float("inf")))
            if pair not in self.merges:
                break
            before = len(ids)
            ids = _merge(ids, pair, self.merges[pair])
            trace.append({"pair": [pair[0], pair[1]], "new_id": self.merges[pair],
                          "tokens_before": before, "tokens_after": len(ids)})
        return {"ids": ids, "trace": trace, "compression": round(len(text.encode('utf-8')) / max(1, len(ids)), 4)}

    @property
    def vocab_size(self) -> int:
        return 256 + len(self.merges)
