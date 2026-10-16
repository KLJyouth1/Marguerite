"""Read-only ``ops.*`` RPC surface for the gateway.

This module mirrors the Rust ``marguerite-core`` ops adapter
(``core/src/gateway/ops_dispatch.rs``) so the WebUI OpsPanel receives real
``{kind, data}`` payloads instead of ``METHOD_NOT_FOUND``. It exposes exactly
the read-only operations the WebUI calls:

* ``ops.list_hosts``
* ``ops.status``
* ``ops.list_files``
* ``ops.tail_logs``
* ``ops.inspect_process``

Pure validation/redaction helpers live in :mod:`opensquilla.gateway.rpc_ops_util`;
this module only dispatches, executes the read-only command over the system
``ssh`` client, and registers the handlers.
"""

from __future__ import annotations

from typing import Any

from opensquilla.gateway.protocol import ResFrame, make_error_res
from opensquilla.gateway.rpc import RpcContext, RpcHandlerError, get_dispatcher
from opensquilla.gateway.rpc_ops_util import (
    OpsError,
    Target,
    build_command,
    is_read_op,
    is_write_op,
    parse_files,
    redact_credentials,
    resolve_address,
    sanitize,
    status_detail,
)

_d = get_dispatcher()

# Gateway method -> operation. Only read-only operations are reachable.
_OPS_METHODS: dict[str, str] = {
    "ops.list_hosts": "list_hosts",
    "ops.status": "status",
    "ops.list_files": "list_files",
    "ops.tail_logs": "tail_logs",
    "ops.inspect_process": "inspect_process",
}


def _ops_hosts(ctx: RpcContext) -> list[dict[str, Any]]:
    """Return the configured ``ops.hosts`` as redacted dicts.

    Reads from ``ctx.config.ops.hosts`` (the :class:`OpsConfig` block on
    :class:`GatewayConfig`). Each entry is stripped of ``credential_ref``
    before it is returned so it never crosses the response boundary.
    """
    config = getattr(ctx, "config", None)
    ops = getattr(config, "ops", None)
    hosts = getattr(ops, "hosts", None) or []
    redacted: list[dict[str, Any]] = []
    for entry in hosts:
        if hasattr(entry, "id"):
            redacted.append({"id": entry.id, "address": entry.address})
        elif isinstance(entry, dict):
            redacted.append(
                {"id": entry.get("id", ""), "address": entry.get("address", "")}
            )
    return redacted


def _raise_from_error(exc: OpsError) -> None:
    """Raise an :class:`RpcHandlerError` carrying the stable ``OPS_*`` code."""
    raise RpcHandlerError(code=exc.code, message=sanitize(exc.message))


def _run_remote(target: Target, command: str) -> str:
    """Run ``command`` on ``target`` via the system ``ssh`` client.

    The host is passed as ``ssh -o BatchMode=yes -p <port> <host> <command>``
    where ``<host>`` is a single argument (never interpolated) and ``<command>``
    is a single argument string. ``BatchMode=yes`` forbids password prompts so
    a missing key fails fast instead of hanging.
    """
    import subprocess

    proc = subprocess.run(
        ["ssh", "-o", "BatchMode=yes", "-p", str(target.port), target.to_ssh_host(), command],
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )
    if proc.returncode != 0:
        raise RpcHandlerError(
            code="OPS_CONNECTION_FAILED", message=sanitize(proc.stderr or proc.stdout)
        )
    return proc.stdout


def _list_hosts_res(ctx: RpcContext) -> dict[str, Any]:
    return {"kind": "Hosts", "data": _ops_hosts(ctx)}


def _per_host_res(operation: str, params: dict[str, Any], ctx: RpcContext) -> dict[str, Any]:
    host_param = params.get("host")
    if not isinstance(host_param, str) or not host_param.strip():
        _raise_from_error(OpsError("OPS_INVALID_REQUEST", "host is required for this operation"))
    target = resolve_address(host_param)
    command = build_command(operation, params)

    if operation == "status":
        try:
            output = _run_remote(target, command)
        except RpcHandlerError as exc:
            return {
                "kind": "Status",
                "data": {"host_id": host_param, "online": False, "detail": exc.message},
            }
        return {
            "kind": "Status",
            "data": {"host_id": host_param, "online": True, "detail": status_detail(output)},
        }

    output = _run_remote(target, command)
    if operation == "list_files":
        return {"kind": "Files", "data": parse_files(output)}
    if operation == "tail_logs":
        return {"kind": "Logs", "data": sanitize(output)}
    if operation == "inspect_process":
        return {"kind": "Process", "data": sanitize(output)}
    _raise_from_error(OpsError("OPS_INVALID_REQUEST", "unknown operation"))


def _dispatch_ops(operation: str, params: Any, ctx: RpcContext) -> dict[str, Any]:
    try:
        return _dispatch_ops_inner(operation, params, ctx)
    except OpsError as exc:
        _raise_from_error(exc)


def _dispatch_ops_inner(operation: str, params: Any, ctx: RpcContext) -> dict[str, Any]:
    if params is not None and not isinstance(params, dict):
        _raise_from_error(OpsError("OPS_INVALID_REQUEST", "params must be an object"))
    params = params or {}

    # The operation is selected by the authenticated gateway method, never by
    # user input, so a conflicting field is rejected rather than used.
    if params.get("operation") is not None:
        _raise_from_error(
            OpsError("OPS_INVALID_REQUEST", "operation is controlled by gateway method")
        )

    if operation == "list_hosts":
        return _list_hosts_res(ctx)

    if is_write_op(operation):
        _raise_from_error(OpsError("OPS_FORBIDDEN_WRITE", "write operations are not permitted"))

    if not is_read_op(operation):
        _raise_from_error(OpsError("OPS_INVALID_REQUEST", "unknown operation"))

    return _per_host_res(operation, params, ctx)


def _register(method: str, operation: str) -> None:
    async def handler(params: Any, ctx: RpcContext) -> dict[str, Any]:
        return redact_credentials(_dispatch_ops(operation, params, ctx))

    _d.method(method, scope="operator.read")(handler)


for _method, _operation in _OPS_METHODS.items():
    _register(_method, _operation)


async def _fallback(req_id: str, method: str, params: Any, ctx: RpcContext) -> ResFrame:
    """Classify an unregistered ``ops.*`` member.

    Mirrors the Rust adapter: a write op is rejected with
    ``OPS_FORBIDDEN_WRITE``; any other unknown ``ops.*`` is
    ``OPS_INVALID_REQUEST``. Callers that reach here already passed the
    read-scope gate (any ``operator.read`` caller may attempt an ops method),
    so no extra authorization is needed.
    """
    operation = method.split("ops.", 1)[-1]
    if is_write_op(operation):
        return make_error_res(req_id, "OPS_FORBIDDEN_WRITE", "write operations are not permitted")
    return make_error_res(req_id, "OPS_INVALID_REQUEST", "unknown operation")


_d.register_prefix_fallback("ops", _fallback)
