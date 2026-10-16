"""Tests for the Python gateway's read-only ``ops.*`` RPC surface.

The contract mirrors the Rust ``marguerite-core`` ops adapter
(``core/src/gateway/ops_dispatch.rs``) so the WebUI OpsPanel receives real
``{kind, data}`` payloads instead of ``METHOD_NOT_FOUND``. Only read-only
operations are exposed: every write op is rejected with
``OPS_FORBIDDEN_WRITE``, unsafe paths with ``OPS_UNSAFE_PATH``, unknown
``ops.*`` with ``OPS_INVALID_REQUEST``. Credentials (``credential_ref`` or
key material) never cross the response/error boundary.
"""

from __future__ import annotations

import pytest

from opensquilla.gateway.auth import Principal
from opensquilla.gateway.config import GatewayConfig
from opensquilla.gateway.protocol import ERROR_UNAUTHORIZED
from opensquilla.gateway.rpc import RpcContext, get_dispatcher
from opensquilla.gateway.scopes import METHOD_SCOPES, READ_SCOPE

OPS_METHODS = (
    "ops.list_hosts",
    "ops.status",
    "ops.list_files",
    "ops.tail_logs",
    "ops.inspect_process",
)


def _ctx(config: GatewayConfig | None = None) -> RpcContext:
    return RpcContext(conn_id="test-conn", config=config or GatewayConfig())


def _restricted_ctx(config: GatewayConfig | None = None) -> RpcContext:
    """A caller holding an ``operator.*`` scope that does NOT imply read.

    ``operator.pairing`` is a real scope in the same namespace but (unlike
    ``operator.admin`` / ``operator.write``) it satisfies neither ``operator.read``
    nor ``operator.write``. ``authenticated=True`` keeps ``GuestRpcPolicy`` out
    of the way so the scope gate inside ``RpcRegistry.dispatch`` is what decides.
    We deliberately avoid ``operator.write``: ``operator_scope_satisfies`` maps
    ``operator.read => operator.write`` (write implies read), so a write-only
    principal would NOT be rejected and would make this test vacuously pass.
    """
    return RpcContext(
        conn_id="test-conn",
        config=config or GatewayConfig(),
        principal=Principal(
            role="operator",
            scopes=frozenset(["operator.pairing"]),
            is_owner=False,
            authenticated=True,
        ),
    )


@pytest.mark.asyncio
async def test_all_ops_methods_are_read_scoped() -> None:
    for method in OPS_METHODS:
        assert METHOD_SCOPES[method] == READ_SCOPE, f"{method} must be operator.read"


@pytest.mark.asyncio
async def test_ops_list_hosts_empty_config_returns_empty_hosts() -> None:
    response = await get_dispatcher().dispatch("req-1", "ops.list_hosts", {}, _ctx())
    assert response.ok is True
    assert response.payload == {"kind": "Hosts", "data": []}


@pytest.mark.asyncio
async def test_ops_list_hosts_returns_configured_hosts_without_credential_ref() -> None:
    cfg = GatewayConfig()
    cfg.ops.hosts = [
        {"id": "node-1", "address": "deploy@10.0.0.5:22", "credential_ref": "cred-ref-1"},
        {"id": "node-2", "address": "10.0.0.6", "credential_ref": "cred-ref-2"},
    ]
    response = await get_dispatcher().dispatch("req-1", "ops.list_hosts", {}, _ctx(cfg))
    assert response.ok is True
    assert response.payload["kind"] == "Hosts"
    hosts = response.payload["data"]
    assert len(hosts) == 2
    assert hosts[0]["id"] == "node-1"
    assert hosts[0]["address"] == "deploy@10.0.0.5:22"
    # credential_ref 绝不跨响应边界泄露
    assert "credential_ref" not in response.model_dump_json()
    assert "cred-ref-1" not in response.model_dump_json()
    assert "cred-ref-2" not in response.model_dump_json()


@pytest.mark.asyncio
async def test_ops_status_without_host_returns_structured_error_without_credential_ref() -> None:
    cfg = GatewayConfig()
    cfg.ops.hosts = [
        {"id": "node-1", "address": "deploy@10.0.0.5:22", "credential_ref": "cred-ref-1"}
    ]
    response = await get_dispatcher().dispatch("req-1", "ops.status", {}, _ctx(cfg))
    assert response.ok is False
    assert response.error.code.startswith("OPS_")
    assert "credential_ref" not in response.model_dump_json()
    assert "cred-ref-1" not in response.model_dump_json()


@pytest.mark.asyncio
async def test_write_operations_are_forbidden() -> None:
    for method in (
        "ops.delete_file",
        "ops.write_file",
        "ops.execute",
        "ops.restart",
        "ops.shutdown",
    ):
        response = await get_dispatcher().dispatch("req-1", method, {}, _ctx())
        assert response.ok is False
        assert response.error.code == "OPS_FORBIDDEN_WRITE"
        assert "credential_ref" not in response.model_dump_json()


@pytest.mark.asyncio
async def test_unknown_ops_method_is_invalid_request() -> None:
    response = await get_dispatcher().dispatch("req-1", "ops.nope", {}, _ctx())
    assert response.ok is False
    assert response.error.code == "OPS_INVALID_REQUEST"


@pytest.mark.asyncio
async def test_unsafe_path_is_rejected_before_ssh() -> None:
    cfg = GatewayConfig()
    cfg.ops.hosts = [{"id": "node-1", "address": "10.0.0.6", "credential_ref": "cred-ref-1"}]
    for path in ("../secret", "/etc/passwd", "a;id"):
        response = await get_dispatcher().dispatch(
            "req-1", "ops.list_files", {"host": "10.0.0.6", "path": path}, _ctx(cfg)
        )
        assert response.ok is False
        assert response.error.code == "OPS_UNSAFE_PATH", f"should reject path {path!r}"


@pytest.mark.asyncio
async def test_unsafe_host_address_is_rejected() -> None:
    for address in ("host;id", "host|id", "host$(whoami)", "host pwd"):
        response = await get_dispatcher().dispatch(
            "req-1", "ops.status", {"host": address}, _ctx()
        )
        assert response.ok is False
        assert response.error.code == "OPS_UNSAFE_PATH", f"should reject host {address!r}"


@pytest.mark.asyncio
async def test_ops_fallback_does_not_catch_unrelated_methods() -> None:
    # A non-``ops.*`` method must still be METHOD_NOT_FOUND: the prefix
    # fallback is namespaced to ``ops`` and must not swallow other surfaces.
    for method in ("foo.bar", "jobs.list"):
        response = await get_dispatcher().dispatch("req-1", method, {}, _ctx())
        assert response.ok is False
        assert response.error.code == "METHOD_NOT_FOUND", f"unrelated method {method!r}"


@pytest.mark.asyncio
async def test_unsafe_path_rejected_for_all_per_host_ops() -> None:
    cfg = GatewayConfig()
    cfg.ops.hosts = [{"id": "node-1", "address": "10.0.0.6", "credential_ref": "cred-ref-1"}]
    for method in ("ops.list_files", "ops.tail_logs"):
        response = await get_dispatcher().dispatch(
            "req-1",
            method,
            {"host": "10.0.0.6", "path": "../../etc/passwd"},
            _ctx(cfg),
        )
        assert response.ok is False
        assert response.error.code == "OPS_UNSAFE_PATH", f"should reject unsafe path for {method}"


@pytest.mark.asyncio
async def test_unsafe_process_arg_is_rejected() -> None:
    # A process name carrying a shell metacharacter is rejected before ssh.
    for name in ("node;id", "node|id", "node$(whoami)"):
        response = await get_dispatcher().dispatch(
            "req-1",
            "ops.inspect_process",
            {"host": "10.0.0.6", "process": name},
            _ctx(),
        )
        assert response.ok is False
        assert response.error.code == "OPS_INVALID_REQUEST", f"should reject process {name!r}"


@pytest.mark.asyncio
async def test_unknown_ops_operator_conflicting_operation_is_rejected() -> None:
    # The operation is selected by the authenticated gateway method, never by
    # user input; a conflicting ``operation`` field is rejected.
    response = await get_dispatcher().dispatch(
        "req-1",
        "ops.status",
        {"operation": "delete_file"},
        _ctx(),
    )
    assert response.ok is False
    assert response.error.code == "OPS_INVALID_REQUEST"


@pytest.mark.asyncio
async def test_ops_rejects_caller_without_read_scope() -> None:
    # General contract: an operator with a non-read ``operator.*`` scope cannot
    # call the read-only ``ops.*`` surface. ``operator.pairing`` is a real scope
    # that does not imply read (write does — see operator_scope_satisfies), so a
    # caller holding only it must be denied at the scope gate, before the handler.
    response = await get_dispatcher().dispatch("req-1", "ops.list_hosts", {}, _restricted_ctx())
    assert response.ok is False
    assert response.error.code == ERROR_UNAUTHORIZED


@pytest.mark.asyncio
async def test_ops_list_hosts_requires_read_scope_even_for_read_op() -> None:
    # Read-only method still demands ``operator.read``: the fact that
    # ``ops.list_hosts`` never mutates state does not waive the read gate.
    response = await get_dispatcher().dispatch("req-1", "ops.status", {}, _restricted_ctx())
    assert response.ok is False
    assert response.error.code == ERROR_UNAUTHORIZED
