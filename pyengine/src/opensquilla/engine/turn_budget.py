"""Per-turn hard circuit-breaker budgets (P2-9).

This is the *unified, env-driven* turn budget on top of the pre-existing
per-turn limits already enforced inside the agent loop
(``max_turn_llm_calls`` / ``max_turn_input_tokens`` / ``max_turn_output_tokens``
/ ``timeout``). It deliberately lives in a dependency-free, fully pure module so
the breach decision is unit-testable without a live provider or gateway.

Three knobs, all **disabled by default** (``0``):

* ``MARGUERITE_TURN_MAX_STEPS``  — max LLM calls / iterations for one turn.
* ``MARGUERITE_TURN_MAX_SECS``  — max wall-clock seconds for one turn.
* ``MARGUERITE_TURN_MAX_TOKENS`` — max cumulative input+output tokens.

When a knob trips, the loop raises :class:`TurnBudgetExhausted`, which the task
runtime turns into the recognizable terminal status
``AgentTaskStatus.BUDGET_EXHAUSTED`` (``terminal_reason="budget_exhausted"``)
with the breach detail persisted on the task row. This is a *hard* abort — not a
silent truncation and not a "compact and continue".
"""

from __future__ import annotations

import os
from dataclasses import asdict, dataclass
from typing import Mapping

BudgetKind = str  # "steps" | "time" | "tokens"


@dataclass(frozen=True)
class TurnBudgetLimits:
    """Three independent per-turn ceilings. ``0`` means disabled."""

    max_steps: int = 0
    max_secs: float = 0.0
    max_tokens: int = 0

    def any_enabled(self) -> bool:
        return (
            (self.max_steps or 0) > 0
            or (self.max_secs or 0.0) > 0
            or (self.max_tokens or 0) > 0
        )


@dataclass(frozen=True)
class BudgetBreach:
    """A single, recognizable budget breach.

    ``used``/``limit``/``remaining`` are carried as floats so the same shape
    works for step counts, seconds and token counts.
    """

    kind: BudgetKind
    limit: float
    used: float

    @property
    def remaining(self) -> float:
        return max(0.0, self.limit - self.used)

    def to_dict(self) -> dict[str, object]:
        return {
            "kind": self.kind,
            "limit": self.limit,
            "used": self.used,
            "remaining": self.remaining,
        }


def _positive_int(value: str | None) -> int:
    try:
        parsed = int(float(str(value).strip()))
    except (TypeError, ValueError):
        return 0
    return parsed if parsed > 0 else 0


def _positive_float(value: str | None) -> float:
    try:
        parsed = float(str(value).strip())
    except (TypeError, ValueError):
        return 0.0
    return parsed if parsed > 0.0 else 0.0


def limits_from_env(environ: Mapping[str, str] | None = None) -> TurnBudgetLimits:
    """Resolve turn budgets from the process environment.

    Unset / invalid / non-positive values all mean "disabled" so an
    accidental junk value can never silently enable a breaker.
    """

    env = os.environ if environ is None else environ
    return TurnBudgetLimits(
        max_steps=_positive_int(env.get("MARGUERITE_TURN_MAX_STEPS")),
        max_secs=_positive_float(env.get("MARGUERITE_TURN_MAX_SECS")),
        max_tokens=_positive_int(env.get("MARGUERITE_TURN_MAX_TOKENS")),
    )


def check_turn_budget(
    *,
    used_steps: int,
    elapsed_secs: float,
    used_tokens: int,
    limits: TurnBudgetLimits,
) -> BudgetBreach | None:
    """Return the first breached knob, or ``None`` when the turn is within budget.

    Boundary semantics: a knob trips exactly when ``used >= limit`` (the first
    call/second/token that would *reach* the ceiling aborts the turn). Checks
    run in a stable precedence — steps, then time, then tokens — so a test
    that trips two knobs at once sees a deterministic result.
    """

    if not limits.any_enabled():
        return None

    used_steps = max(0, int(used_steps))
    used_tokens = max(0, int(used_tokens))
    elapsed_secs = max(0.0, float(elapsed_secs))

    if limits.max_steps > 0 and used_steps >= limits.max_steps:
        return BudgetBreach(
            kind="steps",
            limit=float(limits.max_steps),
            used=float(used_steps),
        )
    if limits.max_secs > 0.0 and elapsed_secs >= limits.max_secs:
        return BudgetBreach(
            kind="time",
            limit=float(limits.max_secs),
            used=elapsed_secs,
        )
    if limits.max_tokens > 0 and used_tokens >= limits.max_tokens:
        return BudgetBreach(
            kind="tokens",
            limit=float(limits.max_tokens),
            used=float(used_tokens),
        )
    return None


class TurnBudgetExhausted(RuntimeError):
    """Raised inside the agent loop when a turn budget hard-breaches.

    Carries the structured breach so the task runtime can persist it and the
    RPC/WS layer can surface ``{status:"budget_exhausted", budget:{...}}``
    without re-deriving it.
    """

    terminal_reason = "budget_exhausted"

    def __init__(self, breach: BudgetBreach) -> None:
        self.breach = breach
        super().__init__(
            "turn budget exhausted: "
            f"{breach.kind} used={breach.used:g} limit={breach.limit:g}"
        )

    @property
    def code(self) -> str:
        return "turn_budget_exhausted"

    def payload(self, *, task_id: str | None, partial_text: str = "") -> dict[str, object]:
        """Build the recognizable response body documented for Rust/frontend.

        Shape::

            {
              "status": "budget_exhausted",
              "budget": {"kind": ..., "limit": ..., "used": ..., "remaining": ...},
              "partial_text": "...",
              "task_id": "..."
            }
        """

        return {
            "status": "budget_exhausted",
            "budget": self.breach.to_dict(),
            "partial_text": partial_text,
            "task_id": task_id,
        }


__all__ = [
    "BudgetBreach",
    "BudgetKind",
    "TurnBudgetExhausted",
    "TurnBudgetLimits",
    "check_turn_budget",
    "limits_from_env",
]


# Keep asdict referenced for symmetry with other engine data modules; it is not
# required at runtime but keeps the frozen dataclasses greppable for callers
# that already use asdict elsewhere.
_ = asdict
