"""End-to-end loop test: the env-driven turn breaker hard-aborts a runaway turn.

Uses a fake provider that *never* emits a terminal stop, so without a budget the
loop would keep iterating. With ``MARGUERITE_TURN_MAX_STEPS`` set the loop must
force-abort and surface a recognizable budget exhaustion.
"""

from __future__ import annotations

from collections.abc import AsyncIterator
from typing import Any

import pytest

from opensquilla.engine import (
    Agent,
    AgentConfig,
    ErrorEvent,
    ToolResult,
)
from opensquilla.engine.turn_budget import TurnBudgetExhausted
from opensquilla.provider import (
    DoneEvent as ProviderDone,
)
from opensquilla.provider import (
    TextDeltaEvent as ProviderText,
)
from opensquilla.provider import (
    ToolUseEndEvent as ProviderToolUseEnd,
    ToolUseStartEvent as ProviderToolUseStart,
)


class _RunawayToolProvider:
    provider_name = "fake"

    def __init__(self) -> None:
        self.calls = 0

    def chat(
        self,
        messages: list[Any],
        tools: list[Any] | None = None,
        config: Any | None = None,
    ) -> AsyncIterator[Any]:
        self.calls += 1
        return self._stream()

    async def _stream(self) -> AsyncIterator[Any]:
        # Never stop: always emit a tool call so the loop keeps iterating.
        yield ProviderToolUseStart(tool_use_id="step", tool_name="read_file")
        yield ProviderToolUseEnd(
            tool_use_id="step",
            tool_name="read_file",
            arguments={"path": "x.txt"},
        )
        yield ProviderDone(stop_reason="tool_calls", input_tokens=1, output_tokens=1)

    async def list_models(self) -> list[Any]:
        return []


async def _ok_tool(call: Any) -> ToolResult:
    return ToolResult(
        tool_use_id=call.tool_use_id,
        tool_name=call.tool_name,
        content="ok",
    )


@pytest.mark.asyncio
async def test_loop_aborts_when_step_budget_trips(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("MARGUERITE_TURN_MAX_STEPS", "1")
    provider = _RunawayToolProvider()
    agent = Agent(
        provider=provider,
        config=AgentConfig(flush_enabled=False, max_provider_retries=0),
        tool_handler=_ok_tool,
    )

    budget_seen = False
    raised: BaseException | None = None
    try:
        events = [event async for event in agent.run_turn("do work forever")]
    except TurnBudgetExhausted as exc:  # raised directly out of the loop
        raised = exc
        events = []
    else:
        for event in events:
            if isinstance(event, ErrorEvent) and (
                event.code == "turn_budget_exhausted" or "budget" in event.message.lower()
            ):
                budget_seen = True

    if raised is not None:
        assert raised.terminal_reason == "budget_exhausted"
    else:
        assert budget_seen, "expected a recognizable budget exhaustion error event"


@pytest.mark.asyncio
async def test_loop_runs_until_provider_stops_when_budget_disabled(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # No env set -> breaker disabled -> provider's own stop governs. The runaway
    # provider never stops, so bound it with max_turn_llm_calls to show the env
    # breaker is NOT what stops it.
    monkeypatch.delenv("MARGUERITE_TURN_MAX_STEPS", raising=False)
    monkeypatch.delenv("MARGUERITE_TURN_MAX_SECS", raising=False)
    monkeypatch.delenv("MARGUERITE_TURN_MAX_TOKENS", raising=False)

    class _FiniteProvider(_RunawayToolProvider):
        def __init__(self, limit: int) -> None:
            super().__init__()
            self.limit = limit

        async def _stream(self) -> AsyncIterator[Any]:
            if self.calls >= self.limit:
                yield ProviderText(text="done")
                yield ProviderDone(stop_reason="stop", input_tokens=1, output_tokens=1)
                return
            yield ProviderToolUseStart(tool_use_id="step", tool_name="read_file")
            yield ProviderToolUseEnd(
                tool_use_id="step", tool_name="read_file", arguments={"path": "x.txt"}
            )
            yield ProviderDone(stop_reason="tool_calls", input_tokens=1, output_tokens=1)

    agent = Agent(
        provider=_FiniteProvider(limit=2),
        config=AgentConfig(flush_enabled=False, max_provider_retries=0),
        tool_handler=_ok_tool,
    )
    events = [event async for event in agent.run_turn("bounded")]
    # No budget error should appear.
    assert not any(
        isinstance(e, ErrorEvent) and e.code == "turn_budget_exhausted" for e in events
    )
