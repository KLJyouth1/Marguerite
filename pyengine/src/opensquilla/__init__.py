"""OpenSquilla — multi-channel LLM gateway and agent runtime."""

from __future__ import annotations

from importlib.metadata import PackageNotFoundError
from importlib.metadata import version as _resolve_version

try:
    __version__ = _resolve_version("opensquilla")
except PackageNotFoundError:  # pragma: no cover - source tree without dist metadata
    # Editable/source checkouts without installed distribution metadata fall
    # back to a sentinel rather than a hardcoded semver that silently goes
    # stale (the bug this module exists to prevent).
    __version__ = "0.0.0+unknown"

# 必须早于任何 Pydantic 模型所在模块的导入：否则编译成 cyfunction 的方法会被
# Pydantic 当成未标注字段而拒绝建类（见 _native_compat 的说明）。
from ._native_compat import install_cython_function_compat as _install_cython_function_compat
from ._native_compat import install_starlette_route_compat as _install_starlette_route_compat

_install_cython_function_compat()
# 必须早于任何 starlette.routing.Route 的构造：否则编译后的函数端点会被当成
# ASGI 类，以 (scope, receive, send) 调用（见 _native_compat 的说明）。
_install_starlette_route_compat()

__all__ = ["__version__"]
