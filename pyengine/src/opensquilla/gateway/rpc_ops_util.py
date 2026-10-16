"""Pure helpers for the read-only ``ops.*`` RPC surface.

This module never touches I/O and never registers handlers. It owns the
command-injection defence (host address / path / argument validation), the
credential redaction, and the read-only command builder so
:mod:`opensquilla.gateway.rpc_ops` stays a thin dispatch layer.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any


class OpsError(Exception):
    """Structured ``OPS_*`` error raised by the validation layer."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


_WRITE_OPS = frozenset({"delete_file", "write_file", "execute", "restart", "shutdown"})
_READ_OPS = frozenset({"list_hosts", "status", "list_files", "tail_logs", "inspect_process"})
_UNSAFE_CHARS = frozenset(";&|`$>< \t\n\r")


@dataclass(frozen=True)
class Target:
    """A validated ``[user@]host[:port]`` SSH target."""

    user: str
    host: str
    port: int

    def to_ssh_host(self) -> str:
        user_prefix = f"{self.user}@" if self.user else ""
        return f"{user_prefix}{self.host}"


def sanitize(value: str) -> str:
    """Redact credential-shaped tokens from a string."""
    lower = value.lower()
    if any(
        token in lower
        for token in ("token=", "password=", "secret", "credential_ref", "auth denied")
    ):
        return "[redacted]"
    return value


def redact_credentials(value: Any) -> Any:
    """Recursively strip ``credential_ref`` from a response value."""
    if isinstance(value, dict):
        return {
            key: redact_credentials(item) if key != "credential_ref" else None
            for key, item in value.items()
            if key != "credential_ref"
        }
    if isinstance(value, list):
        return [redact_credentials(item) for item in value]
    if isinstance(value, str):
        return sanitize(value)
    return value


def reject_shell_metachar(value: str) -> bool:
    """True if ``value`` contains a shell metacharacter."""
    return any(char in _UNSAFE_CHARS for char in value)


def resolve_address(address: str) -> Target:
    """Resolve ``[user@]host[:port]`` into a validated :class:`Target`."""
    raw = address.strip()
    if not raw:
        raise OpsError("OPS_UNSAFE_PATH", "host address is empty")
    if reject_shell_metachar(raw):
        raise OpsError("OPS_UNSAFE_PATH", "host address contains unsafe shell metacharacter")

    user = "root"
    host = raw
    port = 22

    if raw.count("@") > 1:
        raise OpsError("OPS_UNSAFE_PATH", "invalid host address")
    if "@" in raw:
        user, host = raw.rsplit("@", 1)
        if not user or not host:
            raise OpsError("OPS_UNSAFE_PATH", "invalid host address")
    if ":" in host:
        possible_port = host.rsplit(":", 1)[1]
        if not possible_port.isdigit():
            raise OpsError("OPS_UNSAFE_PATH", "invalid host address")
        port = int(possible_port)
        host = host.rsplit(":", 1)[0]
        if not host:
            raise OpsError("OPS_UNSAFE_PATH", "invalid host address")
    if not (1 <= port <= 65535):
        raise OpsError("OPS_UNSAFE_PATH", "invalid host address")

    return Target(user=user, host=host, port=port)


def reject_unsafe_path(path: str) -> None:
    """Reject absolute/traversal paths and shell metacharacters."""
    p = path.strip()
    if (
        p.startswith("/")
        or p.startswith("\\\\")
        or p.startswith("~")
        or ".." in p
        or reject_shell_metachar(p)
    ):
        raise OpsError("OPS_UNSAFE_PATH", f"unsafe path: {path}")


def reject_unsafe_arg(arg: str) -> None:
    """Reject empty, oversized, or shell-metachar process names."""
    a = arg.strip()
    if not a or len(a) > 128 or reject_shell_metachar(a):
        raise OpsError("OPS_INVALID_REQUEST", "unsafe argument")


def shell_quote(arg: str) -> str:
    """Single-quote a value for safe shell use."""
    escaped = arg.replace("\\", "\\\\").replace("'", "'\\''")
    return f"'{escaped}'"


def build_command(operation: str, params: dict[str, Any]) -> str:
    """Build a read-only remote command for an operation."""
    if operation == "list_hosts":
        return ""
    if operation == "status":
        return "uname -a && id && whoami"
    if operation == "list_files":
        path = params.get("path")
        if not isinstance(path, str) or not path:
            raise OpsError("OPS_INVALID_REQUEST", "path is required")
        reject_unsafe_path(path)
        return f"ls -l -- {shell_quote(path)}"
    if operation == "tail_logs":
        path = params.get("path")
        if not isinstance(path, str) or not path:
            raise OpsError("OPS_INVALID_REQUEST", "path is required")
        reject_unsafe_path(path)
        max_lines = params.get("max_lines", 200)
        if not isinstance(max_lines, int) or max_lines <= 0:
            max_lines = 200
        return f"tail -n {max_lines} -- {shell_quote(path)}"
    if operation == "inspect_process":
        name = params.get("process")
        if not isinstance(name, str) or not name:
            name = params.get("path")
        if not isinstance(name, str) or not name:
            raise OpsError("OPS_INVALID_REQUEST", "process name is required")
        reject_unsafe_arg(name)
        return f"pgrep -af {shell_quote(name)}"
    raise OpsError("OPS_INVALID_REQUEST", "unknown operation")


def is_write_op(operation: str) -> bool:
    return operation in _WRITE_OPS


def is_read_op(operation: str) -> bool:
    return operation in _READ_OPS


def parse_files(output: str) -> list[dict[str, Any]]:
    """Parse ``ls -l`` output into ``{path, size_bytes}`` entries."""
    entries: list[dict[str, Any]] = []
    for line in output.splitlines()[1:]:
        tokens = line.split()
        if len(tokens) < 9:
            continue
        try:
            size = int(tokens[4])
        except ValueError:
            continue
        path = " ".join(tokens[8:])
        if path:
            entries.append({"path": path, "size_bytes": size})
    entries.sort(key=lambda entry: entry["path"])
    return entries


def status_detail(output: str) -> str:
    """Extract a redacted one-line status detail from command output."""
    first = next((line for line in output.splitlines() if line), "") or ""
    return sanitize(first)
