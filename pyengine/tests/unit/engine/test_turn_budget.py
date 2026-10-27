"""Pure decision-table tests for the per-turn hard breaker (P2-9)."""

from __future__ import annotations

import pytest

from opensquilla.engine.turn_budget import (
    BudgetBreach,
    TurnBudgetExhausted,
    TurnBudgetLimits,
    check_turn_budget,
    limits_from_env,
)


def test_all_disabled_never_breaches() -> None:
    limits = TurnBudgetLimits()
    assert limits.any_enabled() is False
    assert (
        check_turn_budget(
            used_steps=9999,
            elapsed_secs=9999.0,
            used_tokens=999999,
            limits=limits,
        )
        is None
    )


@pytest.mark.parametrize(
    ("used_steps", "limit", "breached"),
    [
        (0, 1, False),   # below ceiling: not yet
        (1, 1, True),    # exactly at ceiling: boundary trips
        (2, 1, True),    # over ceiling
    ],
)
def test_steps_boundary_is_inclusive(used_steps: int, limit: int, breached: bool) -> None:
    limits = TurnBudgetLimits(max_steps=limit)
    breach = check_turn_budget(
        used_steps=used_steps,
        elapsed_secs=0.0,
        used_tokens=0,
        limits=limits,
    )
    if breached:
        assert breach is not None
        assert breach.kind == "steps"
    else:
        assert breach is None


@pytest.mark.parametrize(
    ("elapsed", "limit", "breached"),
    [
        (0.9, 1.0, False),
        (1.0, 1.0, True),  # boundary
        (1.1, 1.0, True),
    ],
)
def test_time_boundary_is_inclusive(elapsed: float, limit: float, breached: bool) -> None:
    limits = TurnBudgetLimits(max_secs=limit)
    breach = check_turn_budget(
        used_steps=0,
        elapsed_secs=elapsed,
        used_tokens=0,
        limits=limits,
    )
    assert (breach is not None) is breached
    if breached:
        assert breach.kind == "time"


@pytest.mark.parametrize(
    ("used_tokens", "limit", "breached"),
    [
        (99, 100, False),
        (100, 100, True),  # boundary
        (101, 100, True),
    ],
)
def test_tokens_boundary_is_inclusive(used_tokens: int, limit: int, breached: bool) -> None:
    limits = TurnBudgetLimits(max_tokens=limit)
    breach = check_turn_budget(
        used_steps=0,
        elapsed_secs=0.0,
        used_tokens=used_tokens,
        limits=limits,
    )
    assert (breach is not None) is breached
    if breached:
        assert breach.kind == "tokens"


def test_precedence_steps_before_time_before_tokens() -> None:
    limits = TurnBudgetLimits(max_steps=1, max_secs=1.0, max_tokens=1)
    breach = check_turn_budget(
        used_steps=1,
        elapsed_secs=5.0,
        used_tokens=999,
        limits=limits,
    )
    assert breach is not None
    # Steps wins even though time and tokens also tripped.
    assert breach.kind == "steps"


def test_negative_usage_is_clamped() -> None:
    limits = TurnBudgetLimits(max_steps=1, max_secs=1.0, max_tokens=1)
    breach = check_turn_budget(
        used_steps=-5,
        elapsed_secs=-2.0,
        used_tokens=-10,
        limits=limits,
    )
    assert breach is None


@pytest.mark.parametrize(
    ("env", "expected"),
    [
        ({}, TurnBudgetLimits()),
        ({"MARGUERITE_TURN_MAX_STEPS": "3"}, TurnBudgetLimits(max_steps=3)),
        ({"MARGUERITE_TURN_MAX_SECS": "4.5"}, TurnBudgetLimits(max_secs=4.5)),
        ({"MARGUERITE_TURN_MAX_TOKENS": "1024"}, TurnBudgetLimits(max_tokens=1024)),
        # Junk / non-positive must mean disabled, never an accidental breaker.
        ({"MARGUERITE_TURN_MAX_STEPS": "abc"}, TurnBudgetLimits()),
        ({"MARGUERITE_TURN_MAX_STEPS": "-2"}, TurnBudgetLimits()),
        ({"MARGUERITE_TURN_MAX_SECS": "0"}, TurnBudgetLimits()),
    ],
)
def test_limits_from_env(env: dict[str, str], expected: TurnBudgetLimits) -> None:
    assert limits_from_env(env) == expected


def test_breach_payload_shape() -> None:
    breach = BudgetBreach(kind="steps", limit=2.0, used=2.0)
    assert breach.remaining == 0.0
    exc = TurnBudgetExhausted(breach)
    assert exc.terminal_reason == "budget_exhausted"
    assert exc.code == "turn_budget_exhausted"
    payload = exc.payload(task_id="task-123", partial_text="partial answer")
    assert payload == {
        "status": "budget_exhausted",
        "budget": {"kind": "steps", "limit": 2.0, "used": 2.0, "remaining": 0.0},
        "partial_text": "partial answer",
        "task_id": "task-123",
    }
