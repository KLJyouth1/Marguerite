"""Helpers for turning router runtime load failures into operator guidance."""

from __future__ import annotations

from typing import Any

ROUTER_ASSETS_MISSING = "router_assets_missing"
ROUTER_WEIGHTS_NOT_FETCHED = "router_weights_not_fetched"
MACOS_LIBOMP_MISSING = "macos_libomp_missing"
WINDOWS_VC_RUNTIME_MISSING = "windows_vc_runtime_missing"
ROUTER_PYTHON_DEPENDENCY_MISSING = "router_python_dependency_missing"
ROUTER_NATIVE_DEPENDENCY_MISSING = "router_native_dependency_missing"
ROUTER_RUNTIME_UNAVAILABLE = "router_runtime_unavailable"

#: 降级种类中**「一条命令就能修好」**的那几种。
#:
#: 判据不是"严重程度"，而是**「安装是否完整」**：
#:   · ``router_weights_not_fetched`` —— bundle 布局完好，只是权重没拉（``git lfs pull``）；
#:   · ``router_python_dependency_missing`` —— 源码完好，只是可选增强的依赖没装
#:     （``pip install opensquilla[recommended]``）。
#: 两种情况都**不该**在启动日志里记 ERROR：``RouterBundleDegraded`` 的 docstring
#: 已写明 ML 路由是可选增强、规则路由仍能服务每一轮；把它们记成 ERROR 会让
#: 「可选增强没装」看起来像「安装损坏」，用户看到一串 ERROR 却找不到坏在哪。
#:
#: 反过来，``router_assets_missing``（bundle 布局缺失）等**不在此列** ——
#: 那是真损坏，必须保留 ERROR。
REPAIRABLE_ROUTER_DEGRADATIONS = frozenset(
    {
        ROUTER_WEIGHTS_NOT_FETCHED,
        ROUTER_PYTHON_DEPENDENCY_MISSING,
    }
)

#: Marker inside a bundle error message that proves the file on disk is a
#: Git LFS pointer rather than the artifact the manifest describes.
LFS_POINTER_MARKER = "git lfs pointer"


class RouterBundleDegraded(RuntimeError):
    """Bundle layout is intact but its weights are unusable.

    Raised by the gateway's *shallow* asset validation so the caller decides
    what to do. The doctor lets it propagate (it means "report this"); boot
    catches it (it means "degrade loudly, do not refuse to start"), because ML
    routing is an optional enhancement and the rule-based router still serves
    every turn. A bundle whose *layout* is missing raises a plain
    ``RuntimeError`` instead — that one is a genuinely broken installation.
    """



def _error_text(error: BaseException | str | None) -> str:
    if error is None:
        return ""
    parts: list[str] = []
    current: Any = error
    seen: set[int] = set()
    while current is not None and id(current) not in seen:
        seen.add(id(current))
        parts.append(str(current))
        if isinstance(current, BaseException):
            current = current.__cause__ or current.__context__
        else:
            current = None
    return " ".join(part for part in parts if part)


def _has_import_error(error: BaseException | str | None) -> bool:
    current: Any = error
    seen: set[int] = set()
    while isinstance(current, BaseException) and id(current) not in seen:
        seen.add(id(current))
        if isinstance(current, ImportError):
            return True
        current = current.__cause__ or current.__context__
    return False


def classify_router_runtime_error(error: BaseException | str | None) -> str:
    """Return a stable public-ish code for a SquillaRouter runtime failure."""

    text = _error_text(error)
    lower = text.lower()
    if "missing v4 bundle files" in lower:
        return ROUTER_ASSETS_MISSING
    if LFS_POINTER_MARKER in lower or "incomplete v4 router artifact bundle" in lower:
        # The bundle *layout* is fine; the bytes are Git LFS pointers because
        # the weights were never pulled. Classifying this as a generic
        # "runtime unavailable" hid a one-command fix behind "reinstall".
        return ROUTER_WEIGHTS_NOT_FETCHED
    if (
        "libomp.dylib" in lower
        and (
            "library not loaded" in lower
            or "@rpath/libomp.dylib" in lower
            or "lib_lightgbm" in lower
            or "image not found" in lower
        )
    ):
        return MACOS_LIBOMP_MISSING
    if "onnxruntime_pybind11_state" in lower or (
        "dll load failed" in lower and "onnxruntime" in lower
    ):
        return WINDOWS_VC_RUNTIME_MISSING
    if (
        _has_import_error(error)
        or "modulenotfounderror" in lower
        or "no module named" in lower
    ):
        return ROUTER_PYTHON_DEPENDENCY_MISSING
    if (
        "lib_lightgbm" in lower
        or "library not loaded" in lower
        or "cannot open shared object file" in lower
        or "dlopen" in lower
    ):
        return ROUTER_NATIVE_DEPENDENCY_MISSING
    return ROUTER_RUNTIME_UNAVAILABLE


def router_runtime_hint(kind: str) -> str:
    if kind == MACOS_LIBOMP_MISSING:
        return (
            "macOS LightGBM runtime is missing libomp.dylib. Install it with "
            "`brew install libomp`, then run `opensquilla gateway restart`."
        )
    if kind == WINDOWS_VC_RUNTIME_MISSING:
        return (
            "Microsoft Visual C++ Redistributable 2015-2022 x64 is required for "
            "the bundled ONNX router. Install it manually: "
            "https://aka.ms/vs/17/release/vc_redist.x64.exe. After installing, "
            "reopen PowerShell and restart OpenSquilla."
        )
    if kind == ROUTER_ASSETS_MISSING:
        return (
            "Bundled SquillaRouter assets are missing. Reinstall "
            "`opensquilla[recommended]`, or if running from source, run "
            '`git lfs pull --include="src/opensquilla/squilla_router/models/**"`.'
        )
    if kind == ROUTER_WEIGHTS_NOT_FETCHED:
        return (
            "SquillaRouter model weights are Git LFS pointers, not real model "
            "data (the bundle layout is intact, the bytes were never pulled). "
            'Fix with `git lfs install && git lfs pull --include='
            '"src/opensquilla/squilla_router/models/**"`, or reinstall a release '
            "build that ships the weights. Routing falls back to the default "
            "tier until then."
        )
    if kind == ROUTER_PYTHON_DEPENDENCY_MISSING:
        return (
            "Router Python dependencies are missing; install or reinstall "
            "`opensquilla[recommended]`."
        )
    if kind == ROUTER_NATIVE_DEPENDENCY_MISSING:
        return (
            "A native library required by the router failed to load. Reinstall "
            "`opensquilla[recommended]` and verify platform runtime dependencies."
        )
    return (
        "Router runtime is unavailable; reinstall `opensquilla[recommended]` "
        "or disable the router."
    )


def router_runtime_operator_message(error: BaseException | str | None) -> str:
    kind = classify_router_runtime_error(error)
    return (
        "OpenSquilla router fallback active: local ML router runtime failed to load. "
        "OpenSquilla can still run with safe router fallback using default-tier "
        "routing, but local ML routing is disabled until the runtime is available. "
        f"{router_runtime_hint(kind)}"
    )
