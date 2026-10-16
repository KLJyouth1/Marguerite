"""TinyGPT — a dependency-free, mathematically faithful transformer forward pass.

Implements the exact chain documented in the Marguerite upgrade plan and
mirrored from karpathy/build-nanogpt's GPT-2 reproduction:

    tokens (T,)
      -> x = wte[tokens] + wpe[:T]                      # (T, C)
      -> × N blocks:  x = x + MHA(LN(x))   (causal)
                      x = x + MLP(LN(x))   (4C, GELU)
      -> LN(x)
      -> logits = x @ wte.T                             # weight tying, (T, V)
      -> loss = cross_entropy(logits[:-1], targets[1:]) # next-token objective

Everything runs on plain Python floats so every intermediate tensor is
inspectable; ``ForwardResult.telemetry`` carries the per-head causal attention
matrices (post-softmax probabilities) for the Model Lab heatmap.  Dimensions
are deliberately tiny (dim<=64, T<=128): this is a *teaching + verification*
engine, not a training engine — see the plan's NOT IMPLEMENTED section.
"""

from __future__ import annotations

import math
import random
from dataclasses import dataclass, field

LN_EPS = 1e-5
MAX_DIM = 64
MAX_BLOCK = 128
MAX_LAYERS = 4


@dataclass(frozen=True)
class Config:
    vocab_size: int = 256
    dim: int = 32          # C = n_embd
    n_heads: int = 4       # d_k = dim / n_heads must divide evenly
    n_layers: int = 2
    block_size: int = 64   # T context length supported by wpe

    def __post_init__(self) -> None:
        for name, value, cap in (
            ("vocab_size", self.vocab_size, 4096),
            ("dim", self.dim, MAX_DIM),
            ("block_size", self.block_size, MAX_BLOCK),
            ("n_layers", self.n_layers, MAX_LAYERS),
        ):
            if not 1 <= value <= cap:
                raise ValueError(f"{name}={value} outside [1, {cap}]")
        if self.dim % self.n_heads != 0:
            raise ValueError(f"dim={self.dim} must be divisible by n_heads={self.n_heads}")

    @property
    def head_dim(self) -> int:
        return self.dim // self.n_heads


@dataclass
class ForwardResult:
    logits: list[list[float]]                 # (T, V)
    loss: float | None = None                 # next-token CE when targets given
    telemetry: dict = field(default_factory=dict)


def _softmax(row: list[float]) -> list[float]:
    m = max(row)
    exps = [math.exp(v - m) for v in row]
    s = sum(exps)
    return [e / s for e in exps]


def layer_norm(x: list[list[float]], gain: list[float], bias: list[float]) -> list[list[float]]:
    out: list[list[float]] = []
    for row in x:
        mean = sum(row) / len(row)
        var = sum((v - mean) ** 2 for v in row) / len(row)
        inv = 1.0 / math.sqrt(var + LN_EPS)
        out.append([(v - mean) * inv * g + b for v, g, b in zip(row, gain, bias)])
    return out


def gelu(value: float) -> float:
    """GELU with the tanh approximation used by GPT-2."""
    return 0.5 * value * (1.0 + math.tanh(math.sqrt(2.0 / math.pi) * (value + 0.044715 * value ** 3)))


def _matmul(a: list[list[float]], b: list[list[float]]) -> list[list[float]]:
    bt = list(zip(*b))
    return [[sum(x * y for x, y in zip(row, col)) for col in bt] for row in a]


class Linear:
    """y = x @ W (+b); W is (in, out)."""

    __slots__ = ("w", "b")

    def __init__(self, w: list[list[float]], b: list[float]) -> None:
        self.w, self.b = w, b

    def __call__(self, x: list[list[float]]) -> list[list[float]]:
        out = _matmul(x, self.w)
        return [[v + b for v, b in zip(row, self.b)] for row in out]


def _rand_matrix(rng: random.Random, rows: int, cols: int, scale: float) -> list[list[float]]:
    return [[rng.gauss(0.0, scale) for _ in range(cols)] for _ in range(rows)]


def _rand_vector(rng: random.Random, size: int) -> list[float]:
    return [rng.gauss(0.0, 0.02) for _ in range(size)]


class AttentionHead:
    """One causal scaled-dot-product head over pre-projected q/k/v rows."""

    def __init__(self) -> None:
        self.last_attn: list[list[float]] = []

    def __call__(self, q: list[list[float]], k: list[list[float]], v: list[list[float]]) -> list[list[float]]:
        t = len(q)
        dk_inv = 1.0 / math.sqrt(len(q[0]))
        scores = [
            [sum(qi * ki for qi, ki in zip(q[i], k[j])) * dk_inv for j in range(i + 1)]
            for i in range(t)
        ]  # upper triangle simply never materialised — causal by construction
        probs = [_softmax(row) for row in scores]
        self.last_attn = probs
        out = []
        for i in range(t):
            row_out = [0.0] * len(v[0])
            for j, p in enumerate(probs[i]):
                if p == 0.0:
                    continue
                vj = v[j]
                for d, vv in enumerate(vj):
                    row_out[d] += p * vv
            out.append(row_out)
        return out


class Block:
    def __init__(self, cfg: Config, rng: random.Random) -> None:
        d = cfg.dim
        self.cfg = cfg
        scale_qk = 0.02 / math.sqrt(cfg.head_dim)
        # Fused projections: one (C, 3C) matmul split into heads afterwards.
        self.wqkv = Linear(_rand_matrix(rng, d, 3 * d, scale_qk), _rand_vector(rng, 3 * d))
        self.wo = Linear(_rand_matrix(rng, d, d, scale_qk), _rand_vector(rng, d))
        self.ln1_g, self.ln1_b = [1.0] * d, [0.0] * d
        hidden = 4 * d
        self.mlp_fc = Linear(_rand_matrix(rng, d, hidden, 0.02), _rand_vector(rng, hidden))
        self.mlp_proj = Linear(_rand_matrix(rng, hidden, d, 0.02 / math.sqrt(hidden)), _rand_vector(rng, d))
        self.ln2_g, self.ln2_b = [1.0] * d, [0.0] * d
        self.heads = [AttentionHead() for _ in range(cfg.n_heads)]

    def attention(self, ln_x: list[list[float]]) -> tuple[list[list[float]], list[list[list[float]]]]:
        qkv = self.wqkv(ln_x)
        t, d = len(ln_x), self.cfg.dim
        hd = self.cfg.head_dim
        attn_out = [[0.0] * d for _ in range(t)]
        all_attn: list[list[list[float]]] = []
        for h, head in enumerate(self.heads):
            lo = h * hd
            hi = lo + hd
            q = [[row[lo + k] for k in range(hd)] for row in qkv[:]]
            k_rows = [[row[d + lo + k] for k in range(hd)] for row in qkv]
            v_rows = [[row[2 * d + lo + k] for k in range(hd)] for row in qkv]
            head_out = head(q, k_rows, v_rows)
            all_attn.append(head.last_attn)
            for i in range(t):
                for k in range(hd):
                    attn_out[i][lo + k] += head_out[i][k]
        return self.wo(attn_out), all_attn

    def __call__(self, x: list[list[float]]) -> tuple[list[list[float]], list[list[list[float]]]]:
        ln1 = layer_norm(x, self.ln1_g, self.ln1_b)
        attn, attn_map = self.attention(ln1)
        x1 = [[xi + ai for xi, ai in zip(rx, ra)] for rx, ra in zip(x, attn)]
        ln2 = layer_norm(x1, self.ln2_g, self.ln2_b)
        hidden = self.mlp_fc(ln2)
        act = [[gelu(v) for v in row] for row in hidden]
        mlp = self.mlp_proj(act)
        x2 = [[xi + mi for xi, mi in zip(r1, rm)] for r1, rm in zip(x1, mlp)]
        return x2, attn_map


class TinyGPT:
    """Randomly initialised (seeded) GPT-2 style model — real math, tiny scale."""

    def __init__(self, config: Config | None = None, seed: int = 42) -> None:
        self.cfg = config or Config()
        rng = random.Random(seed)
        c = self.cfg
        scale = 0.02 / math.sqrt(c.dim)
        self.wte = _rand_matrix(rng, c.vocab_size, c.dim, scale)
        self.wpe = _rand_matrix(rng, c.block_size, c.dim, scale)
        self.blocks = [Block(c, rng) for _ in range(c.n_layers)]
        self.lnf_g, self.lnf_b = [1.0] * c.dim, [0.0] * c.dim

    # ------------------------------------------------------------------ fwd
    def forward(self, token_ids: list[int], targets: list[int] | None = None,
                collect_attn: bool = False) -> ForwardResult:
        c = self.cfg
        t = len(token_ids)
        if t < 1:
            raise ValueError("need at least one token")
        if t > c.block_size:
            raise ValueError(f"sequence length {t} exceeds block_size={c.block_size}")
        for tok in token_ids:
            if not 0 <= tok < c.vocab_size:
                raise ValueError(f"token id {tok} outside vocabulary [0, {c.vocab_size})")
        if targets is not None and len(targets) != t:
            raise ValueError("targets must align with token_ids")

        x = [[self.wte[tok][d] + self.wpe[pos][d] for d in range(c.dim)]
             for pos, tok in enumerate(token_ids)]
        attn_maps: list[list[list[list[float]]]] = []
        for block in self.blocks:
            x, amap = block(x)
            if collect_attn:
                attn_maps.append(amap)

        xf = layer_norm(x, self.lnf_g, self.lnf_b)
        # Weight tying: project onto embedding transpose.
        logits = [[sum(row[d] * self.wte[v][d] for d in range(c.dim)) for v in range(c.vocab_size)]
                  for row in xf]

        loss = None
        if targets is not None:
            losses = []
            for i in range(t - 1):
                probs = _softmax(logits[i])
                losses.append(-math.log(max(probs[targets[i + 1]], 1e-12)))
            loss = sum(losses) / len(losses) if losses else None

        telemetry: dict = {}
        if collect_attn:
            telemetry["attention"] = attn_maps  # [layer][head][query][key<=query]
            telemetry["entropy"] = [
                [
                    -sum(p * math.log(p) for p in row if p > 0.0)
                    for row in head
                ]
                for amap in attn_maps for head in amap
            ]
        return ForwardResult(logits=logits, loss=loss, telemetry=telemetry)

    # -------------------------------------------------------------- generate
    def generate(self, prompt_ids: list[int], max_new_tokens: int, sample_fn, ) -> list[int]:
        ids = list(prompt_ids)
        chosen: list[int] = []
        for _ in range(max_new_tokens):
            window = ids[-self.cfg.block_size:]
            logits = self.forward(window).logits[-1]
            nxt = sample_fn(logits, counts=_tally(ids))
            ids.append(nxt)
            chosen.append(nxt)
        return chosen


def _tally(ids: list[int]) -> dict[int, int]:
    counts: dict[int, int] = {}
    for i in ids:
        counts[i] = counts.get(i, 0) + 1
    return counts


def cross_entropy_manual(probs_row: list[float], target: int) -> float:
    """Exposed for tests: manual single-position cross-entropy."""
    return -math.log(max(probs_row[target], 1e-12))
