"""Client-disconnect noise must never reach the ERROR log stream.

Windows raises ``ConnectionResetError: [WinError 10054]`` from the Proactor
loop's ``_call_connection_lost`` whenever a browser refreshes, closes a tab or
drops a websocket. That is the normal echo of a hangup, not a gateway fault —
but asyncio reports it at ERROR level, where it used to bury every real
problem. These tests pin both halves of the contract: benign noise stays
silent, and anything outside the whitelist still reaches the default handler.
"""

from __future__ import annotations

import asyncio
import logging
from typing import Any

from opensquilla.gateway.boot import (
    _ASYNCIO_NOISE_FILTER_ATTR,
    _install_asyncio_connection_noise_filter,
    _is_asyncio_connection_noise,
    _make_asyncio_connection_noise_handler,
)


class _RecordingLoop:
    """Minimal event-loop stand-in that records default-handler dispatches."""

    def __init__(self) -> None:
        self.default_calls: list[dict] = []
        self.installed_handler: object | None = None

    def default_exception_handler(self, context: dict) -> None:
        self.default_calls.append(context)

    def set_exception_handler(self, handler: object) -> None:
        self.installed_handler = handler


def _raise_no_running_loop() -> Any:
    """Stand in for a synchronous caller where no loop is spinning yet."""
    raise RuntimeError("no running event loop")


def _connection_reset_context(message: str, winerror: int = 10054) -> dict:
    """Build the exact context shape asyncio passes for WinError 10054."""
    exc = ConnectionResetError(winerror, "远程主机强迫关闭了一个现有的连接")
    exc.winerror = winerror  # type: ignore[attr-defined]
    return {
        "message": message,
        "exception": exc,
        "future": None,
        "task": None,
        "handle": None,
        "protocol": None,
        "socket": None,
    }


def test_installed_filter_silences_benign_connection_reset(caplog) -> None:
    """A WinError 10054 callback report must not reach the default handler."""
    loop = _RecordingLoop()
    handler = _make_asyncio_connection_noise_handler()
    context = _connection_reset_context(
        "Exception in callback _ProactorBasePipeTransport._call_connection_lost()"
    )

    with caplog.at_level(logging.CRITICAL, logger="opensquilla.gateway.boot"):
        handler(loop, context)

    assert loop.default_calls == []
    assert not [r for r in caplog.records if r.levelno >= logging.WARNING]
    assert not [r for r in caplog.records if "10054" in r.getMessage()]


def test_installed_filter_still_reports_real_exceptions() -> None:
    """Reverse control: a non-whitelisted exception MUST be reported as-is.

    This is the guard against turning the whitelist into a wildcard — silently
    swallowing unknown errors would be far worse than the log spam.
    """
    loop = _RecordingLoop()
    handler = _make_asyncio_connection_noise_handler()
    context = {
        "message": "Exception in callback some_real_callback()",
        "exception": RuntimeError("synthetic real failure"),
        "future": None,
    }

    handler(loop, context)

    assert loop.default_calls == [context]


def test_installed_filter_reports_oserror_with_an_unlisted_winerror() -> None:
    """Reverse control for the ``OSError`` arm: an unlisted WinSock code.

    That arm keys on the WinSock code alone (the code *is* the diagnosis), so a
    reset carrying 10061 WSAECONNREFUSED is a real failure and must be
    reported rather than dropped.
    """
    loop = _RecordingLoop()
    handler = _make_asyncio_connection_noise_handler()
    context = _connection_reset_context("transport teardown", winerror=10061)

    handler(loop, context)

    assert loop.default_calls == [context]


def test_installed_filter_reports_non_connection_oserror() -> None:
    """Reverse control: an unrelated WinError is NOT treated as hangup noise."""
    loop = _RecordingLoop()
    handler = _make_asyncio_connection_noise_handler()
    exc = OSError("disk on fire")
    exc.winerror = 5  # type: ignore[attr-defined]
    context = {"message": "Exception in callback disk_writer()", "exception": exc}

    handler(loop, context)

    assert loop.default_calls == [context]


def test_installed_filter_reports_context_without_an_exception() -> None:
    """Reverse control: a message-only report carries no exception to vet."""
    loop = _RecordingLoop()
    handler = _make_asyncio_connection_noise_handler()
    context = {
        "message": "Exception in callback _ProactorBasePipeTransport._call_connection_lost()"
    }

    handler(loop, context)

    assert loop.default_calls == [context]


def test_whitelist_covers_every_teardown_error_and_winerror() -> None:
    for message in (
        "Exception in callback _ProactorBasePipeTransport._call_connection_lost()",
        "Exception in callback _ProactorBasePipeTransport._call_connection_made()",
    ):
        assert _is_asyncio_connection_noise(_connection_reset_context(message)) is True

    for factory in (ConnectionAbortedError, BrokenPipeError):
        assert (
            _is_asyncio_connection_noise(
                {
                    "message": "Exception in callback transport()",
                    "exception": factory("closed"),
                }
            )
            is True
        )

    for winerror in (10054, 10053, 10055, 10038):
        assert (
            _is_asyncio_connection_noise(_connection_reset_context("teardown", winerror))
            is True
        )


def test_benign_noise_is_debug_only_when_debug_is_enabled(caplog) -> None:
    """At DEBUG the suppressed report still leaves one quiet trace line.

    Operators debugging a flaky client need to see that the hangup was seen and
    dropped, but never at INFO or above — that is how it reached ERROR before.
    """
    loop = _RecordingLoop()
    handler = _make_asyncio_connection_noise_handler()
    context = _connection_reset_context(
        "Exception in callback _ProactorBasePipeTransport._call_connection_lost()"
    )

    with caplog.at_level(logging.DEBUG, logger="opensquilla.gateway.boot"):
        handler(loop, context)

    records = [r for r in caplog.records if r.name == "opensquilla.gateway.boot"]
    assert [r.levelno for r in records] == [logging.DEBUG]
    assert "asyncio client disconnect noise suppressed" in records[0].getMessage()
    assert loop.default_calls == []


def test_install_is_idempotent_on_a_running_loop() -> None:
    """Installing twice must not stack handlers or change the outcome."""

    async def _run() -> None:
        loop = asyncio.get_running_loop()
        setattr(loop, _ASYNCIO_NOISE_FILTER_ATTR, False)

        _install_asyncio_connection_noise_filter()
        first = loop.get_exception_handler()
        _install_asyncio_connection_noise_filter()
        second = loop.get_exception_handler()

        assert first is not None
        assert first is second
        assert getattr(loop, _ASYNCIO_NOISE_FILTER_ATTR, False) is True

    asyncio.run(_run())


def test_install_failure_does_not_raise(caplog, monkeypatch) -> None:
    """A hostile loop must not break gateway boot — the filter is hygiene."""

    class _HostileLoop:
        def set_exception_handler(self, handler: object) -> None:
            raise RuntimeError("synthetic install failure")

    monkeypatch.setattr(asyncio, "get_running_loop", _raise_no_running_loop)
    monkeypatch.setattr(asyncio, "get_event_loop", lambda: _HostileLoop())

    with caplog.at_level(logging.WARNING, logger="opensquilla.gateway.boot"):
        _install_asyncio_connection_noise_filter()  # must not raise

    warnings_seen = [
        r for r in caplog.records if "noise filter disabled" in r.getMessage()
    ]
    assert [r.levelno for r in warnings_seen] == [logging.WARNING]
