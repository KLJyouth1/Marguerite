"""Small asyncio helpers for test-friendly background task spawning."""

from __future__ import annotations

import asyncio
import functools
import logging
import sys
from collections.abc import Coroutine
from typing import Any

log = logging.getLogger(__name__)

_WINDOWS_PROACTOR_PATCH_INSTALLED = False

_PROACTOR_CALLBACK_PATTERNS = (
    "_ProactorBasePipeTransport._call_connection_lost",
    "_ProactorBasePipeTransport._call_connection_made",
)


def _is_benign_proactor_error(context: dict[str, Any]) -> bool:
    message = str(context.get("message", ""))
    exception = context.get("exception")
    if not isinstance(exception, ConnectionResetError):
        return False
    return any(pat in message for pat in _PROACTOR_CALLBACK_PATTERNS)


def install_windows_proactor_patch() -> None:
    """Suppress benign ConnectionResetError from Windows ProactorEventLoop callbacks.

    Python's asyncio on Windows uses ProactorEventLoop which raises
    ``ConnectionResetError`` inside ``_call_connection_lost`` when the remote
    host has already force-closed the socket.  The connection is already gone
    so the error is meaningless — but asyncio logs it at ERROR level and
    pollutes the operator's log stream.

    Installs a custom event loop policy so that every new loop automatically
    downgrades those specific errors to DEBUG.  Safe to call multiple times.
    """
    global _WINDOWS_PROACTOR_PATCH_INSTALLED
    if _WINDOWS_PROACTOR_PATCH_INSTALLED:
        return
    if sys.platform != "win32":
        return

    _install_proactor_exception_handler()
    _WINDOWS_PROACTOR_PATCH_INSTALLED = True


def _install_proactor_exception_handler() -> None:
    """Monkey-patch ProactorBasePipeTransport to suppress the noisy error."""
    try:
        from asyncio.proactor_events import ProactorBasePipeTransport
    except ImportError:
        return

    original_call_connection_lost = ProactorBasePipeTransport._call_connection_lost

    @functools.wraps(original_call_connection_lost)
    def _patched_call_connection_lost(self: Any, exc: BaseException | None) -> None:
        try:
            original_call_connection_lost(self, exc)
        except ConnectionResetError:
            pass

    ProactorBasePipeTransport._call_connection_lost = _patched_call_connection_lost


def create_background_task(coro: Coroutine[Any, Any, Any]) -> Any:
    """Create a background task and close unconsumed coroutines in tests."""
    try:
        task = asyncio.create_task(coro)
    except Exception:
        frame = getattr(coro, "cr_frame", None)
        if frame is not None:
            coro.close()
        raise
    frame = getattr(coro, "cr_frame", None)
    if frame is not None and not isinstance(task, asyncio.Task):
        coro.close()
    return task
