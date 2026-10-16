"""Regression tests for the version + provider readouts in gateway status surfaces.

Pins two fixes:
  * The displayed version derives from installed package metadata
    (``importlib.metadata``) rather than a hardcoded literal that silently
    goes stale — it had been frozen at ``"0.1.0"`` across config, the status
    RPC, and gateway identity while the package was at 0.2.1.
  * The provider readout reports the *configured* provider id (e.g.
    ``"openrouter"``), not the OpenAI-compatible backend class that physically
    serves it. OpenRouter/DeepSeek/Gemini all run through ``OpenAIProvider``,
    so introspecting the instance used to mislabel them as ``"openai"``.
"""

from __future__ import annotations

from importlib.metadata import PackageNotFoundError
from importlib.metadata import version as _pkg_version
from types import SimpleNamespace

import pytest

import opensquilla
from opensquilla.gateway.rpc.registry import RpcContext, _gateway_identity_get, _status
from opensquilla.provider.selector import ModelSelector, ProviderConfig, SelectorConfig


def _ctx(**kwargs: object) -> RpcContext:
    return RpcContext(conn_id="c", principal=SimpleNamespace(role="operator"), **kwargs)


def _openrouter_selector() -> ModelSelector:
    return ModelSelector(
        SelectorConfig(
            primary=ProviderConfig(
                provider="openrouter",
                model="deepseek/deepseek-v4-flash",
                api_key="test-key",
            )
        )
    )


def test_package_version_tracks_metadata_not_stale_literal() -> None:
    try:
        installed = _pkg_version("opensquilla")
    except PackageNotFoundError:
        # 源码树（pytest 走 ``pythonpath = ["src"]``，从未 pip install）没有 dist
        # 元数据，按 __init__.py 的设计只能拿到哨兵值。此时仍然咬住那条真契约：
        # **不得**把哨兵换成硬编码 semver——那正是这个模块存在的理由（它曾冻结在
        # 0.1.0，而包已经在 0.2.x）。
        assert opensquilla.__version__ == "0.0.0+unknown"
        return
    assert opensquilla.__version__ == installed
    assert opensquilla.__version__ != "0.1.0"


def test_active_provider_id_reports_configured_id() -> None:
    assert _openrouter_selector().active_provider_id == "openrouter"


def test_active_provider_id_follows_failover() -> None:
    sel = ModelSelector(
        SelectorConfig(
            primary=ProviderConfig(provider="openrouter", model="m"),
            fallbacks=[ProviderConfig(provider="anthropic", model="claude")],
        )
    )
    assert sel.active_provider_id == "openrouter"
    sel.next_fallback()
    assert sel.active_provider_id == "anthropic"


@pytest.mark.asyncio
async def test_status_rpc_reports_metadata_version_and_configured_provider() -> None:
    result = await _status(None, _ctx(provider_selector=_openrouter_selector()))
    assert result["version"] == opensquilla.__version__
    # Configured id surfaced — NOT the "OpenAIProvider" backend class / "openai".
    assert result["provider"] == "openrouter"


@pytest.mark.asyncio
async def test_status_rpc_provider_is_none_without_selector() -> None:
    result = await _status(None, _ctx(provider_selector=None))
    assert result["provider"] is None
    assert result["version"] == opensquilla.__version__


@pytest.mark.asyncio
async def test_gateway_identity_reports_metadata_version() -> None:
    result = await _gateway_identity_get(None, _ctx())
    assert result["version"] == opensquilla.__version__
