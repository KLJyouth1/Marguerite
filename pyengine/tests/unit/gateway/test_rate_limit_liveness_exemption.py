"""R61 守卫：存活探测端点绝不能被限流。

现场（webui e2e 全量、4 worker）：侧车 `/health` 在负载下被自己的限流中间件回了
**429**（实测探针 21/90 次），而 Rust 侧车看门狗把「非 2xx」读成「引擎已死」→ 杀掉健康
引擎并重启（实测 21s 空窗，期间控制台所有 RPC 失败、批量 e2e 变红）。

本用例用真实 app + 真实中间件，且把桶调到**一次就会被撑满**，因此：
- 修复前：第 2 次起 `/health` 直接 429 → 断言红；
- 修复后：`/health` 任意次数都 200，而普通 API 仍会在第 2 次被限流（证明限流本身没被关掉）。
"""

from __future__ import annotations

from starlette.applications import Starlette
from starlette.responses import JSONResponse
from starlette.routing import Route

from opensquilla.gateway.config import GatewayConfig
from opensquilla.gateway.middleware import RateLimitMiddleware


def _build_app() -> Starlette:
    async def health(_request):  # type: ignore[no-untyped-def]
        return JSONResponse({"status": "ok"})

    async def api_config(_request):  # type: ignore[no-untyped-def]
        return JSONResponse({"ok": True})

    app = Starlette(
        routes=[
            Route("/health", health, methods=["GET"]),
            Route("/api/config", api_config, methods=["GET"]),
        ]
    )
    config = GatewayConfig()
    config.rate_limit.enabled = True
    config.rate_limit.max_requests = 1
    config.rate_limit.window_seconds = 60
    app.add_middleware(RateLimitMiddleware, config=config)
    return app


def _get(app, path: str):  # type: ignore[no-untyped-def]
    from starlette.testclient import TestClient

    with TestClient(app) as client:
        return [client.get(path) for _ in range(5)]


def test_health_endpoint_is_never_rate_limited() -> None:
    app = _build_app()
    responses = _get(app, "/health")
    assert [r.status_code for r in responses] == [200] * 5, (
        "存活探测必须永远可答（429 会被看门狗当成死亡信号）："
        + str([r.status_code for r in responses])
    )


def test_api_health_endpoint_is_never_rate_limited() -> None:
    app = _build_app()
    # /api/health 也属于存活探测别名（见 middleware._LIVENESS_PATHS）
    async def api_health(_request):  # type: ignore[no-untyped-def]
        return JSONResponse({"status": "ok"})

    app.routes.append(Route("/api/health", api_health, methods=["GET"]))
    responses = _get(app, "/api/health")
    assert [r.status_code for r in responses] == [200] * 5


def test_ordinary_api_is_still_rate_limited() -> None:
    """反向自证：豁免没有把限流整体关掉。"""
    app = _build_app()
    responses = _get(app, "/api/config")
    codes = [r.status_code for r in responses]
    assert codes[0] == 200, f"第一次必须放行：{codes}"
    assert codes[1] == 429, f"第二次（超过 max_requests=1）必须 429：{codes}"
