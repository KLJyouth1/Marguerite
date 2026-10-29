"""未就绪部署的日志必须按 (provider, reason) 去重。

就绪/状态面板按请求轮询解析器，逐次打点会在 DEBUG 级别下刷出成百上千条
同一行（真机日志里 openrouter / missing_credential 的「爆发」就是这么来的）。
"""

from __future__ import annotations

from structlog.testing import capture_logs

from opensquilla.provider import deployment as dep


def _unresolved(logs: list[dict]) -> list[dict]:
    return [entry for entry in logs if entry.get("event") == "provider_deployment.unresolved"]


def test_unready_is_logged_once_per_provider_and_reason() -> None:
    dep._LOGGED_UNREADY.clear()
    with capture_logs() as logs:
        for _ in range(5):
            result = dep._unready("openrouter", "some-model", "missing_credential")
            assert result.ready is False
            assert result.reason == "missing_credential"
        # 换一个 reason 必须仍然可见：去重的是「同一条事实」，不是整个 provider。
        dep._unready("openrouter", "some-model", "runtime_unsupported")
        dep._unready("", "some-model", "unknown_provider")

    entries = _unresolved(logs)
    assert len(entries) == 3, entries
    assert {(e["provider"], e["reason"]) for e in entries} == {
        ("openrouter", "missing_credential"),
        ("openrouter", "runtime_unsupported"),
        ("", "unknown_provider"),
    }


def test_unready_log_never_raises_when_called_concurrently() -> None:
    import threading

    dep._LOGGED_UNREADY.clear()
    with capture_logs() as logs:
        threads = [
            threading.Thread(target=dep._unready, args=("p", "m", "missing_credential"))
            for _ in range(16)
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
    assert len(_unresolved(logs)) == 1
