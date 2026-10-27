"""Logs domain RPC handlers."""

from __future__ import annotations

import os
import re
from pathlib import Path
from typing import Any

from opensquilla.gateway.diagnostics import diagnostics_status_payload
from opensquilla.gateway.rpc import RpcContext, get_dispatcher
from opensquilla.observability.trace import load_trace_events
from opensquilla.observability.turn_call_log import (
    LOG_DIR_ENV,
    TURN_CALL_LOG_DIR_ENV,
    TURN_CALL_LOG_ENABLED_VALUES,
    TURN_CALL_LOG_ENV,
    is_turn_call_log_enabled,
    resolve_turn_call_log_dir_with_source,
)
from opensquilla.paths import default_opensquilla_home

_d = get_dispatcher()

# 级别只按「所处位置」识别，绝不做全文包含判断：
# `skill_catalog.refreshed ... errors=0` 这类结构化字段里含 "error"，
# 全文搜索会把整行误判成 ERROR（真机日志里 16 条 ERROR 绝大多数是误判，
# 前端 LEVEL 统计与级别筛选同时被污染）。
_LEVEL_TOKEN = r"(TRACE|DEBUG|INFO|WARN|WARNING|ERROR|FATAL|CRITICAL)"
_LEVEL_PATTERNS = (
    # `[INFO]` 只允许出现在行首 80 字符内（时间戳 + logger 前缀区），
    # 避免正文里出现的 "[ERROR]" 字样反过来改写整行级别。
    re.compile(r"^.{0,80}?\[" + _LEVEL_TOKEN + r"\]", re.IGNORECASE),
    # 行首级别：`INFO ...` / `INFO: ...`
    re.compile(r"^\s*" + _LEVEL_TOKEN + r"\b", re.IGNORECASE),
    # 时间戳后的裸级别：`2026-10-22 19:19:26,249 INFO ...`
    re.compile(
        r"^\s*\d{4}-\d{2}-\d{2}[T ][\d:.,]+\s+" + _LEVEL_TOKEN + r"\b",
        re.IGNORECASE,
    ),
)


def _normalize_level(raw: str) -> str:
    level = raw.upper()
    if level == "WARNING":
        return "WARN"
    if level in ("FATAL", "CRITICAL"):
        return "ERROR"
    return level


def _line_level(line: str) -> str:
    """返回该行的级别（无法判定时按 INFO，与前端 `guessLevel` 同语义）。"""
    for pattern in _LEVEL_PATTERNS:
        match = pattern.search(line)
        if match:
            return _normalize_level(match.group(1))
    return "INFO"


def _non_empty_env(name: str) -> str | None:
    value = os.environ.get(name)
    if value is None or not value.strip():
        return None
    return value


def _find_log_file() -> Path | None:
    """Find the structlog output file."""
    env_log_dir = _non_empty_env(LOG_DIR_ENV)
    if env_log_dir:
        candidates = [Path(env_log_dir) / "debug.log"]
    else:
        # Check common locations
        candidates = [
            default_opensquilla_home() / "logs" / "debug.log",
            Path("data") / "debug.log",
            Path("debug.log"),
        ]
    for p in candidates:
        if p.exists():
            return p
    return None


def _env_status(name: str, *, truthy_values: frozenset[str] | None = None) -> dict[str, Any]:
    value = os.environ.get(name)
    stripped = value.strip() if value is not None else ""
    result: dict[str, Any] = {
        "name": name,
        "set": value is not None,
        "empty": value is not None and stripped == "",
    }
    if truthy_values is not None:
        result["truthy"] = stripped.lower() in truthy_values
    return result


def _configured_debug_log_path() -> tuple[Path, str]:
    log_dir = _non_empty_env(LOG_DIR_ENV)
    if log_dir is not None:
        return Path(log_dir) / "debug.log", LOG_DIR_ENV
    return default_opensquilla_home() / "logs" / "debug.log", "default"


def _configured_trace_log_dir() -> tuple[Path, str]:
    log_dir = _non_empty_env(LOG_DIR_ENV)
    if log_dir is not None:
        return Path(log_dir), LOG_DIR_ENV
    return default_opensquilla_home() / "logs", "default"


def _config_value(ctx: RpcContext, name: str, default: Any) -> Any:
    config = getattr(ctx, "config", None)
    if config is None:
        return default
    return getattr(config, name, default)


def _build_logs_status(ctx: RpcContext) -> dict[str, Any]:
    raw_dir, raw_dir_source = resolve_turn_call_log_dir_with_source()
    configured_debug_log, configured_debug_log_source = _configured_debug_log_path()
    trace_dir, trace_dir_source = _configured_trace_log_dir()
    trace_files = sorted(trace_dir.glob("traces-*.jsonl")) if trace_dir.is_dir() else []
    active_tail_path = _find_log_file()

    diagnostics_status = diagnostics_status_payload(
        getattr(ctx, "diagnostics_state", None),
        getattr(ctx, "config", None),
    )

    return {
        "raw_turn_call_log": {
            "enabled": is_turn_call_log_enabled(getattr(ctx, "diagnostics_state", None)),
            "source": diagnostics_status["raw_turn_call"]["source"],
            "enable_env": _env_status(
                TURN_CALL_LOG_ENV,
                truthy_values=TURN_CALL_LOG_ENABLED_VALUES,
            ),
            "enabled_values": sorted(TURN_CALL_LOG_ENABLED_VALUES),
            "directory": {
                "path": str(raw_dir),
                "source": raw_dir_source,
                "exists": raw_dir.exists(),
            },
        },
        "gateway_file_log": {
            "enabled": bool(_config_value(ctx, "log_file_enabled", True)),
            "level": str(_config_value(ctx, "log_level", "DEBUG")),
            "path": str(configured_debug_log),
            "path_source": configured_debug_log_source,
            "exists": configured_debug_log.exists(),
            "active_tail_path": str(active_tail_path) if active_tail_path is not None else None,
            "active_tail_path_exists": active_tail_path.exists() if active_tail_path else False,
        },
        "trace_log": {
            "directory": {
                "path": str(trace_dir),
                "source": trace_dir_source,
                "exists": trace_dir.exists(),
            },
            "file_count": len(trace_files),
            "latest_path": str(trace_files[-1]) if trace_files else None,
        },
        "diagnostics_enabled": {
            "configured": bool(_config_value(ctx, "diagnostics_enabled", False)),
            "effective": diagnostics_status["enabled"],
            "detail": diagnostics_status["detail"],
            "controls_raw_turn_call": diagnostics_status["raw_turn_call"]["source"] == "runtime",
            "raw_source": diagnostics_status["raw_turn_call"]["source"],
        },
        "diagnostics": diagnostics_status,
        "env": {
            TURN_CALL_LOG_ENV: _env_status(
                TURN_CALL_LOG_ENV,
                truthy_values=TURN_CALL_LOG_ENABLED_VALUES,
            ),
            TURN_CALL_LOG_DIR_ENV: _env_status(TURN_CALL_LOG_DIR_ENV),
            LOG_DIR_ENV: _env_status(LOG_DIR_ENV),
        },
    }


@_d.method("logs.status", scope="operator.read")
async def _handle_logs_status(params: dict | None, ctx: RpcContext) -> dict[str, Any]:
    """Report log-related runtime switches without mutating filesystem state."""

    return _build_logs_status(ctx)


@_d.method("logs.trace", scope="operator.read")
async def _handle_logs_trace(params: dict | None, ctx: RpcContext) -> dict[str, Any]:
    """Return safe trace events for one trace id."""

    p = params or {}
    trace_id = str(p.get("trace_id") or "").strip()
    try:
        limit = max(1, min(int(p.get("limit", 1000)), 5000))
    except (TypeError, ValueError):
        limit = 1000
    if not trace_id:
        return {"trace_id": "", "events": [], "count": 0, "total": 0}

    events = load_trace_events(trace_id)
    limited = events[-limit:]
    return {
        "trace_id": trace_id,
        "events": [event.to_dict() for event in limited],
        "count": len(limited),
        "total": len(events),
    }


@_d.method("logs.tail", scope="operator.read")
async def _handle_logs_tail(params: dict | None, ctx: RpcContext) -> dict[str, Any]:
    """Tail log file with cursor-based pagination and level filter."""
    p = params or {}
    # limit 同样容错：`limit: null` 会让 min(None, 1000) 抛 TypeError，
    # 整个 logs.tail 退化成 INTERNAL_ERROR（与 cursor 是同一类边界缺陷）。
    raw_limit = p.get("limit")
    try:
        limit = min(max(1, int(raw_limit)), 1000) if raw_limit is not None else 100
    except (TypeError, ValueError):
        limit = 100
    level_filter = str(p.get("level") or "").strip().upper()
    # 游标容错：WebUI 首轮会显式传 `cursor: null`（尚无可续位置）。
    # 原先 `p.get("cursor", 0)` 在键存在但值为 None 时返回 None，随后
    # `cursor >= file_size` 抛 TypeError → 整个 logs.tail 返回 INTERNAL_ERROR，
    # 日志页首屏因此永远空白（真机实测：
    # "'>=' not supported between instances of 'NoneType' and 'int'"）。
    raw_cursor = p.get("cursor")
    try:
        cursor = max(0, int(raw_cursor)) if raw_cursor is not None else 0
    except (TypeError, ValueError):
        cursor = 0

    log_file = _find_log_file()
    if log_file is None or not log_file.exists():
        return {"lines": [], "cursor": 0, "has_more": False}

    file_size = log_file.stat().st_size
    if cursor >= file_size:
        return {"lines": [], "cursor": file_size, "has_more": False}

    with open(log_file, encoding="utf-8", errors="replace") as f:
        f.seek(cursor)
        raw_lines = f.readlines()
        new_cursor = f.tell()

    # Apply level filter if specified（按级别位置判定，不做全文包含匹配）
    if level_filter:
        wanted = _normalize_level(level_filter) if level_filter else ""
        filtered = [ln for ln in raw_lines if _line_level(ln) == wanted]
    else:
        filtered = raw_lines

    # Limit output
    has_more = len(filtered) > limit
    lines = [ln.rstrip() for ln in filtered[-limit:]]

    return {"lines": lines, "cursor": new_cursor, "has_more": has_more}
