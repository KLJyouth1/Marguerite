"""Capability SID storage for Windows sandbox roots."""

from __future__ import annotations

import json
import ntpath
import os
import re
import secrets
import time
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path

_RESTRICTING_SID_RE = re.compile(r"^S-1-5-21-(\d+)-(\d+)-(\d+)-(\d+)$")

# Windows 上「打开锁文件」与「加锁」都可能瞬时失败，这不是逻辑错误：
#   - 杀软 / 搜索索引器 / 备份代理会在文件刚创建时短暂独占句柄；
#   - 另一线程正用 `os.replace` 换同一目录下的文件时，句柄共享也会被拒。
# 之前的实现把这种瞬时争用当成致命错误直接抛出，于是并发写能力库的用例在
# 真实机器上偶发 `PermissionError: [Errno 13]`（实测约 1/5 概率）。
# 这里改成**有界重试**：只在瞬时错误（winerror 5/32/33）上重试，其余照旧上抛 ——
# 不能把「路径真的没权限」这种真错误也拖成 10 秒再报。
_LOCK_RETRY_BUDGET_SECS = 10.0
_LOCK_RETRY_FIRST_DELAY = 0.005
_LOCK_RETRY_MAX_DELAY = 0.05
_LOCK_TRANSIENT_WINERRORS = (5, 32, 33)  # ACCESS_DENIED / SHARING_VIOLATION / LOCK_VIOLATION


def _is_transient_lock_error(exc: BaseException) -> bool:
    if isinstance(exc, PermissionError):
        return True
    return getattr(exc, "winerror", None) in _LOCK_TRANSIENT_WINERRORS


@dataclass(frozen=True)
class CapabilityStore:
    root_sids: dict[str, str]


def load_capability_store(path: Path) -> CapabilityStore:
    with _capability_store_lock(path):
        return _load_capability_store_unlocked(path)


def _load_capability_store_unlocked(path: Path) -> CapabilityStore:
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, OSError, json.JSONDecodeError):
        return CapabilityStore(root_sids={})
    if not isinstance(raw, dict):
        return CapabilityStore(root_sids={})
    roots = raw.get("rootSids")
    if not isinstance(roots, dict):
        return CapabilityStore(root_sids={})
    clean: dict[str, str] = {}
    for key, value in roots.items():
        if not isinstance(key, str) or not key or not isinstance(value, str):
            continue
        if not re.match(r"^(?:rx|rwx)\|.+$", key, re.IGNORECASE):
            continue
        if _is_create_restricted_token_compatible_sid(value):
            clean[key] = value
    return CapabilityStore(root_sids=clean)


def save_capability_store(path: Path, store: CapabilityStore) -> None:
    with _capability_store_lock(path):
        _save_capability_store_unlocked(path, store)


def _save_capability_store_unlocked(path: Path, store: CapabilityStore) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    try:
        temp.write_text(
            json.dumps({"rootSids": store.root_sids}, sort_keys=True),
            encoding="utf-8",
        )
        os.replace(temp, path)
    finally:
        temp.unlink(missing_ok=True)


def capability_sid_for_root(
    store_path: Path,
    root: Path,
    *,
    access: str = "RWX",
    sid_factory: Callable[[], str] | None = None,
) -> str:
    with _capability_store_lock(store_path):
        store = _load_capability_store_unlocked(store_path)
        key = _capability_root_key(root, access)
        existing = store.root_sids.get(key)
        if existing:
            return existing
        sid = sid_factory() if sid_factory is not None else _new_capability_sid()
        if not _is_create_restricted_token_compatible_sid(sid):
            sid = _new_capability_sid()
        updated = dict(store.root_sids)
        updated[key] = sid
        _save_capability_store_unlocked(store_path, CapabilityStore(root_sids=updated))
        return sid


@contextmanager
def _capability_store_lock(path: Path) -> Iterator[None]:
    path.parent.mkdir(parents=True, exist_ok=True)
    lock_path = path.with_name(f".{path.name}.lock")
    # ① 打开：单独重试（这一步最容易被杀软/索引器瞬时占用）。
    stream = _open_lock_stream(lock_path)
    with stream:
        if os.name == "nt":
            import msvcrt

            if stream.seek(0, os.SEEK_END) == 0:
                stream.write(b"\0")
                stream.flush()
            stream.seek(0)
            locking = getattr(msvcrt, "locking")
            unlock_mode = getattr(msvcrt, "LK_UNLCK")
            # ② 加锁：`LK_LOCK` 自带 ~10s 阻塞重试，但句柄被外部干扰时仍会抛
            #    `OSError(EACCES)`；用 `LK_NBLCK` + 有界退避把控制权拿回自己手里，
            #    这样「等多久」是我们说了算，也不会在日志里变成一次无解释的失败。
            nonblocking = getattr(msvcrt, "LK_NBLCK")
            deadline = time.monotonic() + _LOCK_RETRY_BUDGET_SECS
            delay = _LOCK_RETRY_FIRST_DELAY
            while True:
                try:
                    locking(stream.fileno(), nonblocking, 1)
                    break
                except OSError:
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(delay)
                    delay = min(delay * 2, _LOCK_RETRY_MAX_DELAY)
            try:
                yield
            finally:
                stream.seek(0)
                locking(stream.fileno(), unlock_mode, 1)
        else:
            import fcntl

            fcntl.flock(stream.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(stream.fileno(), fcntl.LOCK_UN)


def _open_lock_stream(lock_path: Path):
    """打开锁文件；只对 Windows 的瞬时共享冲突重试。"""
    deadline = time.monotonic() + _LOCK_RETRY_BUDGET_SECS
    delay = _LOCK_RETRY_FIRST_DELAY
    while True:
        try:
            return lock_path.open("a+b")
        except OSError as exc:
            if not _is_transient_lock_error(exc) or time.monotonic() >= deadline:
                raise
            time.sleep(delay)
            delay = min(delay * 2, _LOCK_RETRY_MAX_DELAY)


def capability_sids_for_command(
    store_path: Path,
    roots: tuple[Path, ...],
    *,
    accesses: tuple[str, ...] | None = None,
) -> tuple[str, ...]:
    effective_accesses = accesses or tuple("RWX" for _root in roots)
    if len(effective_accesses) != len(roots):
        raise ValueError("capability roots and accesses must have equal length")
    return tuple(
        capability_sid_for_root(store_path, root, access=access)
        for root, access in zip(roots, effective_accesses, strict=True)
    )


def _capability_root_key(root: Path, access: str) -> str:
    normalized_access = access.strip().upper()
    if normalized_access not in {"RX", "RWX"}:
        raise ValueError(f"unsupported capability access: {access!r}")
    raw = str(root)
    if "\\" in raw or re.match(r"^[A-Za-z]:", raw):
        path_key = ntpath.normcase(ntpath.normpath(raw.replace("/", "\\")))
    else:
        path_key = str(root.expanduser().resolve(strict=False)).casefold()
    return f"{normalized_access.casefold()}|{path_key.casefold()}"


def _new_capability_sid() -> str:
    parts = [str(secrets.randbits(32)) for _ in range(4)]
    return "S-1-5-21-" + "-".join(parts)


def _is_create_restricted_token_compatible_sid(value: str) -> bool:
    return _RESTRICTING_SID_RE.match(value) is not None


__all__ = [
    "CapabilityStore",
    "capability_sid_for_root",
    "capability_sids_for_command",
    "load_capability_store",
    "save_capability_store",
]
