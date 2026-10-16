"""Repeated-identical-call notice at the dispatch layer.

Two paths with **opposite** defaults, because the two cases have opposite
semantics:

* **Success** — ``OPENSQUILLA_REPEATED_CALL_NOTICE`` (off by default). When
  armed with a threshold N, a read-only tool call whose canonicalized arguments
  and raw result are byte-identical to the previous identical call gets a notice
  injected into the finalized content from the Nth repeat onward, plus a
  ``dispatch.repeated_call_notice`` runtime event. Tools outside the read-only
  allowlist and changed results never trigger it. Repeats of a *successful* call
  can be legitimate (data refresh, state change), hence opt-in.
* **Failure** — ``OPENSQUILLA_REPEATED_FAILURE_NOTICE`` (**on by default**,
  threshold 3; set to ``0`` to disable). A repeated *identical failure* is a
  death loop: same tool, same arguments, same error — the Nth repeat carries no
  new information. No tool allowlist applies, because byte-identical failure
  output is positive proof the call had *no* side effect (the inverse of the
  rationale that excludes execution/write tools on the success path).

Counters are scoped per session_key on both paths.
"""

from __future__ import annotations

import json

import pytest

from opensquilla.engine.types import ToolCall
from opensquilla.result_budget import ToolResultBudgetPolicy
from opensquilla.tools.dispatch import build_tool_handler
from opensquilla.tools.registry import ToolRegistry
from opensquilla.tools.types import ToolContext, ToolSpec, current_tool_context

_NOTICE_PREFIX = "[repeated_call_notice]"


def _build_registry(read_file_results: list[str] | None = None) -> ToolRegistry:
    registry = ToolRegistry()
    results = read_file_results if read_file_results is not None else ["stable file body"]
    call_index = {"value": 0}

    async def read_file(path: str) -> str:
        index = min(call_index["value"], len(results) - 1)
        call_index["value"] += 1
        return results[index]

    async def failing_read(path: str) -> str:
        raise ValueError("boom")

    async def echo_probe(value: str) -> str:
        return "identical probe output"

    registry.register(
        ToolSpec(
            name="read_file",
            description="read file",
            parameters={"path": {"type": "string"}},
            required=["path"],
        ),
        read_file,
    )
    registry.register(
        ToolSpec(
            name="git_status",
            description="failing status",
            parameters={"path": {"type": "string"}},
            required=["path"],
        ),
        failing_read,
    )
    registry.register(
        ToolSpec(
            name="echo_probe",
            description="echo probe",
            parameters={"value": {"type": "string"}},
            required=["value"],
        ),
        echo_probe,
    )
    return registry


def _read_file_call(tool_use_id: str) -> ToolCall:
    return ToolCall(
        tool_use_id=tool_use_id,
        tool_name="read_file",
        arguments={"path": "src/a.py"},
    )


def _notice_events(events: list[dict[str, object]]) -> list[dict[str, object]]:
    return [
        event for event in events if event.get("name") == "dispatch.repeated_call_notice"
    ]


@pytest.mark.asyncio
async def test_default_off_results_are_byte_identical_and_no_event(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv("OPENSQUILLA_REPEATED_CALL_NOTICE", raising=False)
    events: list[dict[str, object]] = []
    handler = build_tool_handler(
        _build_registry(),
        ToolContext(on_runtime_event=events.append, session_key="agent:main:off"),
    )

    contents = []
    for index in range(5):
        result = await handler(_read_file_call(f"tc-off-{index}"))
        assert result.is_error is False
        contents.append(result.content)

    assert contents == ["stable file body"] * 5
    assert _notice_events(events) == []


@pytest.mark.asyncio
async def test_invalid_gate_values_stay_off(monkeypatch: pytest.MonkeyPatch) -> None:
    for raw in ("0", "abc", "-2", "  "):
        monkeypatch.setenv("OPENSQUILLA_REPEATED_CALL_NOTICE", raw)
        events: list[dict[str, object]] = []
        handler = build_tool_handler(
            _build_registry(),
            ToolContext(on_runtime_event=events.append, session_key="agent:main:invalid"),
        )

        for index in range(3):
            result = await handler(_read_file_call(f"tc-invalid-{index}"))
            assert result.content == "stable file body"

        assert _notice_events(events) == []


@pytest.mark.asyncio
async def test_threshold_three_notices_from_third_identical_call(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("OPENSQUILLA_REPEATED_CALL_NOTICE", "3")
    events: list[dict[str, object]] = []
    handler = build_tool_handler(
        _build_registry(),
        ToolContext(on_runtime_event=events.append, session_key="agent:main:notice"),
    )

    first = await handler(_read_file_call("tc-n-1"))
    second = await handler(_read_file_call("tc-n-2"))
    third = await handler(_read_file_call("tc-n-3"))
    fourth = await handler(_read_file_call("tc-n-4"))

    assert first.content == "stable file body"
    assert second.content == "stable file body"
    assert third.content == (
        f"{_NOTICE_PREFIX} This exact read_file call has already been run 3 "
        "times this session and returned an identical result each time.\n"
        "stable file body"
    )
    assert fourth.content.startswith(
        f"{_NOTICE_PREFIX} This exact read_file call has already been run 4 times"
    )

    notice_events = _notice_events(events)
    assert [event["repeat_count"] for event in notice_events] == [3, 4]
    event = notice_events[0]
    assert event["feature"] == "repeated_call_notice"
    assert event["tool"] == "read_file"
    assert event["tool_name"] == "read_file"
    assert event["tool_use_id"] == "tc-n-3"
    assert event["threshold"] == 3
    assert event["injected_to_model"] is True
    assert event["session_key"] == "agent:main:notice"
    assert event["agent_id"] == "main"
    assert isinstance(event["arguments_sha256"], str)
    assert len(event["arguments_sha256"]) == 64
    assert isinstance(event["result_sha256"], str)
    assert len(event["result_sha256"]) == 64


@pytest.mark.asyncio
async def test_changed_result_resets_the_counter(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("OPENSQUILLA_REPEATED_CALL_NOTICE", "2")
    events: list[dict[str, object]] = []
    handler = build_tool_handler(
        _build_registry(read_file_results=["body-a", "body-b", "body-c", "body-c"]),
        ToolContext(on_runtime_event=events.append, session_key="agent:main:reset"),
    )

    for index in range(3):
        result = await handler(_read_file_call(f"tc-reset-{index}"))
        assert result.content == f"body-{'abc'[index]}"
    assert _notice_events(events) == []

    # body-c repeats: count restarts at the first stable result, so the second
    # identical result reaches the threshold.
    repeated = await handler(_read_file_call("tc-reset-3"))
    assert repeated.content.startswith(
        f"{_NOTICE_PREFIX} This exact read_file call has already been run 2 times"
    )
    assert [event["repeat_count"] for event in _notice_events(events)] == [2]


@pytest.mark.asyncio
async def test_non_allowlisted_tool_successes_are_never_noticed(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """成功路径的契约**未变**：只读白名单之外的工具，成功也不提示。

    （成功路径的重复可能是合法的——数据刷新、状态变化——所以只在显式
    配置时提示，且限定结果为其参数的纯函数的那批只读工具。）
    """
    monkeypatch.setenv("OPENSQUILLA_REPEATED_CALL_NOTICE", "1")
    events: list[dict[str, object]] = []
    handler = build_tool_handler(
        _build_registry(),
        ToolContext(on_runtime_event=events.append, session_key="agent:main:excluded"),
    )

    for index in range(3):
        probe_result = await handler(
            ToolCall(
                tool_use_id=f"tc-probe-{index}",
                tool_name="echo_probe",
                arguments={"value": "same"},
            )
        )
        assert probe_result.is_error is False
        assert probe_result.content == "identical probe output"

    assert _notice_events(events) == []


@pytest.mark.asyncio
async def test_repeated_identical_failures_are_noticed_for_any_tool(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """**核心回归**：相同失败重复到达默认阈值（3）即提示，且**不设工具白名单**。

    这是「死循环」的正解。真机实测：agent 以 2–4 秒间隔重复调用
    `shell_exec()` / `code_exec()` 并失败，持续数分钟、累计 63 次 FAILED ——
    而旧实现**恰好跳过失败**（`final_result.is_error` 早退）**且**默认阈值 0
    （功能整体关闭），两道门同时把它挡在门外。

    失败路径不设白名单的理由：成功路径的白名单是「执行/写工具字节相同的
    输出不能证明没有副作用」；对失败而言，字节相同的失败恰恰**证明没有副作用**。
    """
    monkeypatch.delenv("OPENSQUILLA_REPEATED_FAILURE_NOTICE", raising=False)
    events: list[dict[str, object]] = []
    handler = build_tool_handler(
        _build_registry(),
        ToolContext(on_runtime_event=events.append, session_key="agent:main:failing"),
    )

    # git_status 不在成功白名单里，但它**失败** —— 必须照样被计数。
    for index in range(2):
        result = await handler(
            ToolCall(
                tool_use_id=f"tc-fail-{index}",
                tool_name="git_status",
                arguments={"path": "."},
            )
        )
        assert result.is_error is True
        assert _NOTICE_PREFIX not in result.content, "未到阈值不应提示"

    third = await handler(
        ToolCall(
            tool_use_id="tc-fail-2",
            tool_name="git_status",
            arguments={"path": "."},
        )
    )
    assert third.is_error is True
    assert third.content.startswith(
        f"{_NOTICE_PREFIX} This exact git_status call has already failed 3 times"
    )
    # 提示必须明确要求**换法子**，而不只是「你已经试过了」。
    assert "change approach" in third.content
    assert [event["repeat_count"] for event in _notice_events(events)] == [3]


@pytest.mark.asyncio
async def test_repeated_failure_notice_can_be_disabled(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """显式设 0 可关闭失败提示（与成功路径「默认关、显式开」相反的默认值）。"""
    monkeypatch.setenv("OPENSQUILLA_REPEATED_FAILURE_NOTICE", "0")
    events: list[dict[str, object]] = []
    handler = build_tool_handler(
        _build_registry(),
        ToolContext(on_runtime_event=events.append, session_key="agent:main:off-fail"),
    )

    for index in range(5):
        result = await handler(
            ToolCall(
                tool_use_id=f"tc-off-fail-{index}",
                tool_name="git_status",
                arguments={"path": "."},
            )
        )
        assert _NOTICE_PREFIX not in result.content

    assert _notice_events(events) == []


@pytest.mark.asyncio
async def test_changed_failure_signature_resets_the_counter(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """**不同**的失败不得累计 —— 否则「换了法子但仍失败」会被误判成死循环。

    失败签名只依赖 (异常/错误内容)，不掺时间戳/耗时/tool_use_id。
    """
    monkeypatch.delenv("OPENSQUILLA_REPEATED_FAILURE_NOTICE", raising=False)
    events: list[dict[str, object]] = []
    registry = _build_registry()
    call_index = {"value": 0}

    async def alternating_failure(path: str) -> str:
        call_index["value"] += 1
        raise ValueError(f"boom-{call_index['value']}")

    registry.register(
        ToolSpec(
            name="flaky_probe",
            description="fails differently each time",
            parameters={"path": {"type": "string"}},
            required=["path"],
        ),
        alternating_failure,
    )
    handler = build_tool_handler(
        registry,
        ToolContext(on_runtime_event=events.append, session_key="agent:main:changing"),
    )

    for index in range(5):
        result = await handler(
            ToolCall(
                tool_use_id=f"tc-change-{index}",
                tool_name="flaky_probe",
                arguments={"path": "."},
            )
        )
        assert _NOTICE_PREFIX not in result.content, "签名变化必须重置计数"

    assert _notice_events(events) == []


@pytest.mark.asyncio
async def test_json_dict_content_gets_notice_as_key_and_stays_parseable(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("OPENSQUILLA_REPEATED_CALL_NOTICE", "2")
    events: list[dict[str, object]] = []
    handler = build_tool_handler(
        _build_registry(read_file_results=["y" * 2000]),
        ToolContext(
            on_runtime_event=events.append,
            session_key="agent:main:json",
            tool_result_budget_policy=ToolResultBudgetPolicy(
                max_single_tool_result_chars=200,
            ),
        ),
    )

    first = await handler(_read_file_call("tc-json-1"))
    first_payload = json.loads(first.content)
    assert first_payload["result_truncated"] is True
    assert "repeated_call_notice" not in first_payload

    second = await handler(_read_file_call("tc-json-2"))
    second_payload = json.loads(second.content)
    assert second_payload["result_truncated"] is True
    assert second_payload["tool"] == "read_file"
    assert second_payload["repeated_call_notice"] == (
        f"{_NOTICE_PREFIX} This exact read_file call has already been run 2 "
        "times this session and returned an identical result each time."
    )
    assert [event["repeat_count"] for event in _notice_events(events)] == [2]


@pytest.mark.asyncio
async def test_session_keys_track_independent_counters(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("OPENSQUILLA_REPEATED_CALL_NOTICE", "2")
    events: list[dict[str, object]] = []
    handler = build_tool_handler(_build_registry())
    parent_ctx = ToolContext(
        on_runtime_event=events.append,
        session_key="agent:main:parent",
    )
    subagent_ctx = ToolContext(
        on_runtime_event=events.append,
        session_key="subagent:agent:main:parent",
    )

    async def dispatch(ctx: ToolContext, tool_use_id: str):
        token = current_tool_context.set(ctx)
        try:
            return await handler(_read_file_call(tool_use_id))
        finally:
            current_tool_context.reset(token)

    parent_first = await dispatch(parent_ctx, "tc-scope-p1")
    subagent_first = await dispatch(subagent_ctx, "tc-scope-s1")
    assert parent_first.content == "stable file body"
    assert subagent_first.content == "stable file body"
    assert _notice_events(events) == []

    parent_second = await dispatch(parent_ctx, "tc-scope-p2")
    assert parent_second.content.startswith(_NOTICE_PREFIX)

    subagent_second = await dispatch(subagent_ctx, "tc-scope-s2")
    assert subagent_second.content.startswith(_NOTICE_PREFIX)

    notice_events = _notice_events(events)
    assert [event["session_key"] for event in notice_events] == [
        "agent:main:parent",
        "subagent:agent:main:parent",
    ]
    assert [event["repeat_count"] for event in notice_events] == [2, 2]


@pytest.mark.asyncio
async def test_json_file_body_keeps_exact_bytes_with_text_prefix(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # A file whose content is itself a JSON object (package.json etc.) is NOT
    # the structured result_truncated wrapper: the notice must land as a text
    # prefix and the original bytes must survive verbatim, or a later
    # edit_file old_text taken from the displayed content will not match.
    monkeypatch.setenv("OPENSQUILLA_REPEATED_CALL_NOTICE", "2")
    file_body = '{\n  "name": "pkg",\n  "version": "1.0.0"\n}'
    events: list[dict[str, object]] = []
    handler = build_tool_handler(
        _build_registry(read_file_results=[file_body]),
        ToolContext(
            on_runtime_event=events.append,
            session_key="agent:main:jsonbody",
        ),
    )

    first = await handler(_read_file_call("tc-body-1"))
    assert first.content == file_body

    second = await handler(_read_file_call("tc-body-2"))
    assert second.content.startswith(_NOTICE_PREFIX)
    assert second.content.endswith("\n" + file_body)
    assert "repeated_call_notice" not in json.loads(file_body)
    assert [event["repeat_count"] for event in _notice_events(events)] == [2]


@pytest.mark.asyncio
async def test_no_session_identity_is_never_counted(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # A caller without session_key gets no counter at all: id()-derived scope
    # fallbacks alias across unrelated callers (id reuse after GC; every None
    # ctx shares one id), so identity-less traffic must stay notice-free
    # rather than share a merged counter.
    monkeypatch.setenv("OPENSQUILLA_REPEATED_CALL_NOTICE", "2")
    events: list[dict[str, object]] = []
    handler = build_tool_handler(
        _build_registry(),
        ToolContext(on_runtime_event=events.append, session_key=None),
    )

    for index in range(4):
        result = await handler(_read_file_call(f"tc-anon-{index}"))
        assert result.content == "stable file body"

    assert _notice_events(events) == []
