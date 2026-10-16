"""LLMLab tests — mathematical correctness of the first-principles engine.

Every assertion checks a *property* of the math (softmax mass, causal masking,
greedy==T→0, BPE roundtrip, weight-tying shapes), not snapshot values.
"""

from __future__ import annotations

import math
import random

import pytest

from marguerite_workbench.llmlab import (
    BPETokenizer,
    Config,
    TinyGPT,
    apply_penalties,
    make_sampler,
    renormalise,
    sample,
    truncate_top_k,
    truncate_top_p,
)


# ---------------------------------------------------------------------------
# tokenizer
# ---------------------------------------------------------------------------
def test_bpe_roundtrip_identity():
    text = "the quick brown fox jumps over the lazy dog — Marguerite 工作台"
    tok = BPETokenizer.train(text + " " + text.upper(), vocab_size=300)
    ids = tok.encode(text)
    assert tok.decode(ids) == text
    assert max(ids) < tok.vocab_size


def test_bpe_merges_compress_and_trace_is_ordered():
    corpus = "aaabdaaabac" * 6
    tok = BPETokenizer.train(corpus, vocab_size=260)
    traced = tok.encode_trace("aaabdaaabac")
    raw_len = len("aaabdaaabac".encode())
    assert traced["tokens_after" if False else "ids"]  # ids present
    assert len(traced["ids"]) <= raw_len, "merges must not expand the sequence"
    assert all(t["tokens_after"] <= t["tokens_before"] for t in traced["trace"])
    assert traced["compression"] >= 1.0


def test_bpe_vocab_size_floor():
    with pytest.raises(ValueError):
        BPETokenizer.train("abc", vocab_size=100)


# ---------------------------------------------------------------------------
# transformer
# ---------------------------------------------------------------------------
def test_config_validation():
    with pytest.raises(ValueError):
        Config(dim=33, n_heads=4)  # indivisible
    with pytest.raises(ValueError):
        Config(dim=0)
    assert Config(dim=32, n_heads=4).head_dim == 8


def test_forward_shapes_loss_finite_and_deterministic():
    cfg = Config(vocab_size=128, dim=16, n_heads=2, n_layers=2, block_size=32)
    ids = [i % 128 for i in range(12)]
    a = TinyGPT(cfg, seed=1).forward(ids, targets=ids)
    b = TinyGPT(cfg, seed=1).forward(ids, targets=ids)
    assert len(a.logits) == 12 and len(a.logits[0]) == 128
    assert a.loss is not None and math.isfinite(a.loss)
    assert a.logits == b.logits and a.loss == b.loss


def test_attention_rows_are_causal_probability_distributions():
    cfg = Config(vocab_size=64, dim=16, n_heads=2, n_layers=1, block_size=32)
    model = TinyGPT(cfg, seed=3)
    res = model.forward([1, 2, 3, 5], collect_attn=True)
    layer0 = res.telemetry["attention"][0]
    for head in layer0:
        for i, row in enumerate(head):
            assert len(row) == i + 1, "causal: query i attends only to keys <= i"
            assert abs(sum(row) - 1.0) < 1e-6
            assert all(p >= 0.0 for p in row)


def test_sequence_limit_enforced():
    model = TinyGPT(Config(vocab_size=64, block_size=8), seed=0)
    with pytest.raises(ValueError):
        model.forward([1] * 9)
    with pytest.raises(ValueError):
        model.forward([999])


# ---------------------------------------------------------------------------
# sampler
# ---------------------------------------------------------------------------
LOGITS = [0.1, 2.0, -1.0, 0.4, 2.0]


def test_greedy_matches_temperature_zero():
    assert sample(LOGITS, temperature=0.0) == 1
    seeded = make_sampler(seed=5, temperature=0.0)
    assert seeded(LOGITS) == 1


def test_top_k_one_equals_greedy():
    rng = random.Random(11)
    for _ in range(20):
        assert sample(LOGITS, temperature=1.0, top_k=1, rng=rng) == 1


def test_top_p_keeps_minimal_mass_prefix():
    probs = [0.4, 0.3, 0.15, 0.1, 0.05]
    kept = truncate_top_p(probs, 0.7)
    assert sum(kept) >= 0.7 - 1e-9
    # smallest such set: 0.4+0.3 == 0.7 exactly
    assert [v for v in kept if v > 0] == [0.4, 0.3]


def test_top_k_zeroes_truncated_tail():
    kept = truncate_top_k([0.5, 0.3, 0.2], 2)
    assert kept[2] == 0.0 and abs(sum(kept) - 0.8) < 1e-9
    assert sum(renormalise(kept)) == pytest.approx(1.0)


def test_penalties_suppress_repeated_tokens():
    counts = {1: 3}
    out = apply_penalties([1.0, 1.0, 1.0], counts, frequency_penalty=1.0, presence_penalty=0.5)
    assert out[1] == 1.0 - 3 * 1.0 - 0.5
    assert out[0] == 1.0


def test_sampling_respects_support():
    rng = random.Random(9)
    drawn = {sample(LOGITS, temperature=1.5, top_k=2, rng=rng) for _ in range(200)}
    assert drawn <= {1, 4}, "top-k=2 must restrict draws to the two strongest logits"


def test_generate_length_and_window():
    cfg = Config(vocab_size=64, dim=16, n_heads=2, n_layers=1, block_size=8)
    model = TinyGPT(cfg, seed=2)
    out = model.generate([1, 2, 3], 6, make_sampler(seed=0, temperature=1.0))
    assert len(out) == 6 and all(0 <= t < 64 for t in out)
