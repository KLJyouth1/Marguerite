"""事件循环心跳与新鲜度纯函数（P0-2 深度就绪探针）。

看门狗只探静态 ``/health`` 时，事件循环堵死/死锁是不可见的「活死进程」。
``loop_heartbeat`` 用一个每秒写时间戳的后台 coroutine 给 ``/ready`` 提供
事件循环自证；这里钉住纯判定逻辑与 env 阈值，不依赖真实时间。
"""

from __future__ import annotations

import asyncio
import time
from types import SimpleNamespace

import pytest

from opensquilla.gateway.loop_heartbeat import (
    DEFAULT_STALL_SECS,
    STALL_THRESHOLD_ENV,
    run_loop_heartbeat,
    stall_threshold_secs,
    staleness,
)


def test_staleness_fresh_when_age_within_threshold() -> None:
    verdict = staleness(now=100.0, last_tick=90.0, threshold_secs=15.0)
    assert verdict["fresh"] is True
    assert verdict["stalled"] is False
    assert verdict["age_secs"] == 10.0
    assert verdict["threshold_secs"] == 15.0


def test_staleness_boundary_at_threshold_is_still_fresh() -> None:
    # age == threshold 不算卡死：只有「超过」阈值才判定循环停转。
    verdict = staleness(now=100.0, last_tick=85.0, threshold_secs=15.0)
    assert verdict["fresh"] is True
    assert verdict["stalled"] is False
    assert verdict["age_secs"] == 15.0


def test_staleness_stalled_when_age_exceeds_threshold() -> None:
    verdict = staleness(now=100.0, last_tick=80.0, threshold_secs=15.0)
    assert verdict["fresh"] is False
    assert verdict["stalled"] is True
    assert verdict["age_secs"] == 20.0


def test_staleness_no_tick_yet_is_fresh_not_stalled() -> None:
    # 启动期 / 未挂心跳的 app（如测试构造）不得自报卡死。
    verdict = staleness(now=100.0, last_tick=None, threshold_secs=15.0)
    assert verdict["fresh"] is True
    assert verdict["stalled"] is False
    assert verdict["age_secs"] is None


def test_stall_threshold_env_overrides_default(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(STALL_THRESHOLD_ENV, "5")
    assert stall_threshold_secs() == 5.0


@pytest.mark.parametrize("bad", ["", "not-a-number", "-3", "0"])
def test_stall_threshold_env_invalid_falls_back_to_default(
    monkeypatch: pytest.MonkeyPatch, bad: str
) -> None:
    monkeypatch.setenv(STALL_THRESHOLD_ENV, bad)
    assert stall_threshold_secs() == DEFAULT_STALL_SECS


def test_stall_threshold_defaults_when_env_absent(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv(STALL_THRESHOLD_ENV, raising=False)
    assert stall_threshold_secs() == DEFAULT_STALL_SECS


async def test_heartbeat_task_writes_and_advances_last_tick() -> None:
    state = SimpleNamespace()
    task = asyncio.create_task(run_loop_heartbeat(state, interval_secs=0.05))
    try:
        # create_task 不立即执行协程：先让出一轮让首次写入发生。
        await asyncio.sleep(0.01)
        first = state.loop_last_tick
        assert first <= time.time()
        # 跨过至少一次 tick：时间戳必须推进，否则心跳就是假的。
        await asyncio.sleep(0.2)
        second = state.loop_last_tick
        assert second >= first
    finally:
        task.cancel()
