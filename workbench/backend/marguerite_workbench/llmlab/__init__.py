"""LLMLab — 第一性原理实验室（tokenizer / transformer math / sampling）。

Public surface re-exports the three pillars so callers can ``from
marguerite_workbench.llmlab import BPETokenizer, TinyGPT, sample``.
"""

from marguerite_workbench.llmlab.sampler import (
    apply_penalties,
    make_sampler,
    renormalise,
    sample,
    truncate_top_k,
    truncate_top_p,
)
from marguerite_workbench.llmlab.tokenizer import BPETokenizer
from marguerite_workbench.llmlab.transformer import Config, ForwardResult, TinyGPT

__all__ = [
    "BPETokenizer",
    "Config",
    "ForwardResult",
    "TinyGPT",
    "apply_penalties",
    "make_sampler",
    "renormalise",
    "sample",
    "truncate_top_k",
    "truncate_top_p",
]
