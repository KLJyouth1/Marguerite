"""Frozen-aware launch, dispatch, and packaged developer-runtime discovery."""

from __future__ import annotations

import os
import sys
from collections.abc import Callable, Mapping, Sequence
from enum import StrEnum
from pathlib import Path
from typing import Any

from opensquilla.sandbox.policy_models import RuntimePolicySettings
from opensquilla.sandbox.run_mode import RunMode
from opensquilla.sandbox.runtime_manifest import (
    BundledRuntimeResolver,
    RuntimeManifest,
    RuntimeManifestError,
    split_path,
)


class ChildRole(StrEnum):
    """Fixed internal roles that a packaged Gateway may execute."""

    FILESYSTEM_WORKER = "filesystem-worker"
    LINUX_HELPER = "linux-helper"
    WINDOWS_DEFAULT_RUNNER = "windows-default-runner"
    DIRECTORY_PICKER = "directory-picker"


class InternalChildDispatchError(ValueError):
    """Raised when an internal-child request does not name a fixed role."""


_ROLE_MODULES: dict[ChildRole, str] = {
    ChildRole.FILESYSTEM_WORKER: "opensquilla.sandbox.filesystem_worker",
    ChildRole.LINUX_HELPER: "opensquilla.sandbox.backend.linux_helper",
    ChildRole.WINDOWS_DEFAULT_RUNNER: "opensquilla.sandbox.backend.windows_default_runner",
    ChildRole.DIRECTORY_PICKER: "opensquilla.gateway.windows_directory_picker",
}

_RUNTIME_ROOT_ENV = "OPENSQUILLA_BUNDLED_RUNTIME_ROOT"
_RUNTIME_MANIFEST_ENV = "OPENSQUILLA_RUNTIME_MANIFEST"


def discover_bundled_runtime_layout() -> tuple[Path, Path] | None:
    """Return ``(manifest, developer-root)`` without downloading anything."""

    explicit_root = os.environ.get(_RUNTIME_ROOT_ENV)
    explicit_manifest = os.environ.get(_RUNTIME_MANIFEST_ENV)
    if explicit_root or explicit_manifest:
        root = Path(explicit_root).expanduser() if explicit_root else None
        if explicit_manifest:
            manifest = Path(explicit_manifest).expanduser()
        elif root is not None:
            manifest = root.parent / "runtime-manifest.json"
        else:  # pragma: no cover - guarded by the outer condition
            return None
        if root is None:
            root = manifest.parent / "developer"
        return manifest.absolute(), root.absolute()

    executable = Path(sys.executable).absolute()
    candidates = [executable.parent, *executable.parents]
    for candidate in candidates:
        manifest = candidate / "runtime-manifest.json"
        developer_root = candidate / "developer"
        if manifest.is_file() and developer_root.is_dir():
            return manifest, developer_root
        runtime_manifest = candidate / "runtime" / "runtime-manifest.json"
        runtime_developer = candidate / "runtime" / "developer"
        if runtime_manifest.is_file() and runtime_developer.is_dir():
            return runtime_manifest, runtime_developer
    return None


def bundled_runtime_resolver() -> BundledRuntimeResolver | None:
    layout = discover_bundled_runtime_layout()
    if layout is None:
        return None
    manifest_path, developer_root = layout
    try:
        manifest = RuntimeManifest.from_path(manifest_path)
        return BundledRuntimeResolver(manifest, resource_root=developer_root)
    except RuntimeManifestError:
        return None


def apply_bundled_runtime_path(
    environment: Mapping[str, str] | None,
    *,
    mode: RunMode | str,
    policy: RuntimePolicySettings | Mapping[str, Any] | None = None,
    require_bundled: bool = False,
) -> dict[str, str]:
    """Apply Safe/Full PATH precedence to a child environment.

    Safe places enabled packaged tools first; Full keeps host PATH first.
    ``require_bundled`` is used by guest-safe runs, which must keep their
    already-isolated PATH rather than inheriting host tools when a package is
    incomplete.
    """

    result = dict(environment or {})
    path_key = next((key for key in result if key.casefold() == "path"), "PATH")
    resolver = bundled_runtime_resolver()
    if resolver is None:
        return result
    resolved = resolver.path_for(mode, split_path(result.get(path_key)), policy=policy)
    result[path_key] = os.pathsep.join(str(path) for path in resolved)
    return result


def _coerce_role(role: ChildRole | str) -> ChildRole:
    if isinstance(role, ChildRole):
        return role
    try:
        return ChildRole(str(role))
    except ValueError as exc:
        raise ValueError(f"unknown internal child role: {role!r}") from exc


def internal_child_argv(
    role: ChildRole | str,
    *,
    args: Sequence[str] = (),
) -> tuple[str, ...]:
    """Build argv for an internal child in source and frozen runtimes."""

    child_role = _coerce_role(role)
    child_args = tuple(str(arg) for arg in args)
    executable = str(sys.executable)
    if bool(getattr(sys, "frozen", False)):
        return (
            executable,
            "--internal-child",
            child_role.value,
            *child_args,
        )
    return (
        executable,
        "-m",
        _ROLE_MODULES[child_role],
        *child_args,
    )


def internal_child_env(base: Mapping[str, str] | None = None) -> dict[str, str]:
    """Environment for an internal child, with the package root on ``PYTHONPATH``.

    ``-m opensquilla.…`` 只在**子进程自己**能 import 到包时才成立。父进程里
    `pytest` 的 `pythonpath = ["src"]` 只改父进程的 `sys.path`，不会写进环境变量，
    于是干净检出（CI 的 ubuntu 作业）上子进程直接
    `ModuleNotFoundError: No module named 'opensquilla'` —— 实测 3 条用例因此判红，
    而在「先 pip install -e .」的开发机上永远复现不出来。

    这里按**本文件的真实位置**推导包根（`src/`），只在缺失时前插，不覆盖调用方
    已有的 `PYTHONPATH`（原生打包运行时靠它定位随包分发的依赖）。
    """

    env = dict(os.environ if base is None else base)
    root = str(Path(__file__).resolve().parents[2])
    parts = [p for p in env.get("PYTHONPATH", "").split(os.pathsep) if p]
    if root not in parts:
        parts.insert(0, root)
    env["PYTHONPATH"] = os.pathsep.join(parts)
    return env


def _run_filesystem_worker(args: Sequence[str]) -> int:
    from opensquilla.sandbox.filesystem_worker import main

    main(args)
    return 0


def _run_linux_helper(args: Sequence[str]) -> int:
    from opensquilla.sandbox.backend.linux_helper import main

    return int(main(list(args)))


def _run_windows_default_runner(args: Sequence[str]) -> int:
    from opensquilla.sandbox.backend.windows_default_runner import main

    main(args)
    return 0


def _run_directory_picker(args: Sequence[str]) -> int:
    from opensquilla.gateway.windows_directory_picker import main

    return int(main(args))


_ROLE_HANDLERS: dict[ChildRole, Callable[[Sequence[str]], int]] = {
    ChildRole.FILESYSTEM_WORKER: _run_filesystem_worker,
    ChildRole.LINUX_HELPER: _run_linux_helper,
    ChildRole.WINDOWS_DEFAULT_RUNNER: _run_windows_default_runner,
    ChildRole.DIRECTORY_PICKER: _run_directory_picker,
}


def dispatch_internal_child(argv: Sequence[str]) -> int:
    """Dispatch a packaged internal child without entering the public CLI."""

    args = tuple(str(arg) for arg in argv)
    if not args:
        raise InternalChildDispatchError("missing internal child role")
    try:
        role = ChildRole(args[0])
    except ValueError as exc:
        raise InternalChildDispatchError(f"unknown internal child role: {args[0]!r}") from exc
    return _ROLE_HANDLERS[role](args[1:])


__all__ = [
    "ChildRole",
    "InternalChildDispatchError",
    "dispatch_internal_child",
    "internal_child_argv",
]
