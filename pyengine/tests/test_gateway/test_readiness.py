from __future__ import annotations

import time

import pytest
from starlette.testclient import TestClient

from opensquilla.gateway.app import create_gateway_app
from opensquilla.gateway.config import GatewayConfig
from opensquilla.gateway.diagnostics import DiagnosticsState


def test_ready_endpoint_reports_starting_until_gateway_marks_ready() -> None:
    app = create_gateway_app(GatewayConfig())
    app.state.gateway_ready = False

    with TestClient(app) as client:
        starting = client.get("/ready")
        assert starting.status_code == 503
        assert starting.json()["ready"] is False

        app.state.gateway_ready = True
        ready = client.get("/ready")
        assert ready.status_code == 200
        assert ready.json()["ready"] is True


def test_create_gateway_app_creates_default_diagnostics_state() -> None:
    app = create_gateway_app(GatewayConfig(diagnostics_enabled=True))

    assert isinstance(app.state.diagnostics_state, DiagnosticsState)
    assert app.state.diagnostics_state.snapshot().effective_enabled is True


def test_ready_reports_loop_stalled_when_heartbeat_tick_is_stale(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("MARGUERITE_SIDECAR_STALL_SECS", "15")
    app = create_gateway_app(GatewayConfig())
    app.state.gateway_ready = True
    # No heartbeat task runs in this test app: backdate the tick so the loop
    # looks wedged 30s ago. The probe must flip to 503 even though the gateway
    # itself finished booting.
    app.state.loop_last_tick = time.time() - 30.0

    with TestClient(app) as client:
        response = client.get("/ready")
        assert response.status_code == 503
        body = response.json()
        assert body["ready"] is False
        assert body["stalled"] is True
        assert body["status"] == "loop_stalled"
        assert body["stall_secs"] >= 29.0


def test_ready_reports_fresh_when_tick_is_recent() -> None:
    app = create_gateway_app(GatewayConfig())
    app.state.gateway_ready = True
    app.state.loop_last_tick = time.time()

    with TestClient(app) as client:
        response = client.get("/ready")
        assert response.status_code == 200
        body = response.json()
        assert body["ready"] is True
        assert body["stalled"] is False
        assert body["status"] == "ready"


def test_readyz_alias_shares_the_stalled_judgement() -> None:
    app = create_gateway_app(GatewayConfig())
    app.state.gateway_ready = True
    app.state.loop_last_tick = time.time() - 999.0

    with TestClient(app) as client:
        response = client.get("/readyz")
        assert response.status_code == 503
        assert response.json()["stalled"] is True
        assert response.json()["ready"] is False


def test_health_endpoint_stays_simple_liveness() -> None:
    # /health keeps its static liveness semantics: unaffected by the heartbeat.
    app = create_gateway_app(GatewayConfig())
    app.state.gateway_ready = True
    app.state.loop_last_tick = time.time() - 999.0

    with TestClient(app) as client:
        response = client.get("/health")
        assert response.status_code == 200
        assert response.json() == {"ok": True, "status": "live"}
