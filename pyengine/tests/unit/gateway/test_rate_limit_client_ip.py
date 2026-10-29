"""限流键必须来自传输层，而不是调用方随手写的 `X-Forwarded-For`。

`RateLimitMiddleware._get_client_ip` 此前无条件采信 `X-Forwarded-For`，于是任何
本机调用方每次换一个值就得到一个全新的桶 —— 限流形同虚设。浏览器拿不到这个头
（属于 forbidden header），但桌面客户端、脚本与同机进程都能发。

口径与 origin guard 一致：代理元数据不能替一个客户端可控的标识背书。只有
`trusted-proxy` 模式（配置里显式声明了代理）才认这个头。
"""

from __future__ import annotations

from starlette.applications import Starlette
from starlette.responses import JSONResponse
from starlette.routing import Route

from opensquilla.gateway.config import GatewayConfig
from opensquilla.gateway.middleware import RateLimitMiddleware


def _build_app(auth_mode: str) -> Starlette:
    async def api_config(_request):  # type: ignore[no-untyped-def]
        return JSONResponse({"ok": True})

    app = Starlette(routes=[Route("/api/config", api_config, methods=["GET"])])
    config = GatewayConfig()
    config.auth.mode = auth_mode
    config.auth.trusted_proxy = "127.0.0.1"
    config.rate_limit.enabled = True
    config.rate_limit.max_requests = 1
    config.rate_limit.window_seconds = 60
    app.add_middleware(RateLimitMiddleware, config=config)
    return app


def _get_with_forwarded(app, forwarded_values: list[str]):  # type: ignore[no-untyped-def]
    from starlette.testclient import TestClient

    with TestClient(app) as client:
        return [
            client.get("/api/config", headers={"X-Forwarded-For": value})
            for value in forwarded_values
        ]


def test_token_mode_ignores_forged_forwarded_for() -> None:
    """换 XFF 不能换来新桶：同一个 peer 共享一个窗口。"""
    responses = _get_with_forwarded(_build_app("token"), ["10.0.0.1", "10.0.0.2", "10.0.0.3"])
    codes = [r.status_code for r in responses]
    assert codes[0] == 200, f"第一次必须放行：{codes}"
    assert codes[1] == 429, f"伪造 XFF 不应得到新桶：{codes}"
    assert codes[2] == 429, f"伪造 XFF 不应得到新桶：{codes}"


def test_trusted_proxy_mode_still_honours_forwarded_for() -> None:
    """反向自证：显式信任代理时，真实客户端 IP 仍用于分桶（修复没有把它关掉）。"""
    responses = _get_with_forwarded(
        _build_app("trusted-proxy"), ["10.0.0.1", "10.0.0.2", "10.0.0.1"]
    )
    codes = [r.status_code for r in responses]
    assert codes[0] == 200, f"客户端 A 第一次放行：{codes}"
    assert codes[1] == 200, f"客户端 B 应有独立桶：{codes}"
    assert codes[2] == 429, f"客户端 A 第二次必须被限流：{codes}"
