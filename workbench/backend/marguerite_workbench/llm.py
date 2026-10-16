"""Optional OpenAI-compatible LLM client.

The whole workbench is callable with **zero** API keys: every consumer that
*might* use an LLM routes through :func:`llm_or_fallback`, which attempts a real
call only when ``UNIV_LLM_BASE_URL`` + ``UNIV_LLM_KEY`` are configured, and
otherwise invokes the supplied deterministic fallback.  This honours the
contract's "determinism without keys" rule and the Bloom R3 rule (we log the
degradation instead of silently swallowing).
"""

from __future__ import annotations

import logging
import os
from typing import Callable, Optional

logger = logging.getLogger("marguerite.llm")

ENV_BASE_URL = "UNIV_LLM_BASE_URL"
ENV_API_KEY = "UNIV_LLM_KEY"
ENV_MODEL = "UNIV_LLM_MODEL"
DEFAULT_MODEL = "gpt-4o-mini"


class LLMClient:
    """Thin OpenAI-compatible chat client.  ``enabled`` is False when unconfigured."""

    def __init__(self) -> None:
        self.base_url: Optional[str] = os.environ.get(ENV_BASE_URL)
        self.api_key: Optional[str] = os.environ.get(ENV_API_KEY)
        self.model: str = os.environ.get(ENV_MODEL, DEFAULT_MODEL)
        self.enabled: bool = bool(self.base_url and self.api_key)

    def complete(
        self,
        prompt: str,
        *,
        system: Optional[str] = None,
        max_tokens: int = 1024,
        temperature: float = 0.2,
    ) -> str:
        content, _thinking = self.complete_with_trace(
            prompt, system=system, max_tokens=max_tokens, temperature=temperature
        )
        return content

    def complete_with_trace(
        self,
        prompt: str,
        *,
        system: Optional[str] = None,
        max_tokens: int = 1024,
        temperature: float = 0.2,
    ) -> tuple[str, Optional[str]]:
        """Return ``(content, thinking_trace)``.

        ``thinking_trace`` carries the model's chain-of-thought when the
        provider exposes it via the DeepSeek-style ``reasoning_content`` field
        or the OpenAI-style ``reasoning`` alias; it is ``None`` otherwise.
        Per the DeepSeek thinking-mode contract the trace must NOT be fed back
        into multi-turn history — callers keep it for display/telemetry only.
        """
        if not self.enabled:
            raise RuntimeError("LLM not configured (set UNIV_LLM_BASE_URL / UNIV_LLM_KEY)")

        messages = []
        if system:
            messages.append({"role": "system", "content": system})
        messages.append({"role": "user", "content": prompt})

        payload = {
            "model": self.model,
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
        }
        headers = {
            "Authorization": f"Bearer {self.api_key}",
            "Content-Type": "application/json",
        }
        # Lazy import: httpx is an optional dependency.
        import httpx  # noqa: WPS433

        url = f"{self.base_url.rstrip('/')}/chat/completions"
        resp = httpx.post(url, json=payload, headers=headers, timeout=60)
        resp.raise_for_status()
        content, thinking = parse_chat_response(resp.json())
        return content, thinking


def get_client() -> LLMClient:
    return LLMClient()


def parse_chat_response(data: dict) -> tuple[str, Optional[str]]:
    """Extract ``(content, thinking_trace)`` from an OpenAI-compatible body.

    Dual-format thinking support: DeepSeek exposes the chain-of-thought as
    ``choices[0].message.reasoning_content`` while several OpenAI-compatible
    providers alias it as ``reasoning``.  Missing fields yield ``None`` —
    never an empty string masquerading as "no thinking".
    """
    try:
        message = data["choices"][0]["message"]
    except (KeyError, IndexError, TypeError) as exc:
        raise ValueError(f"malformed chat completion response: {exc}") from exc
    content = message.get("content") or ""
    thinking = message.get("reasoning_content") or message.get("reasoning")
    return str(content), (str(thinking) if thinking else None)


def llm_or_fallback(
    prompt: str,
    fallback: Callable[[], str],
    *,
    system: Optional[str] = None,
) -> str:
    """Return an LLM completion when configured, else the deterministic ``fallback``.

    Network/configuration failures degrade gracefully to ``fallback`` (logged,
    never silently swallowed per Bloom R3).
    """
    client = LLMClient()
    if not client.enabled:
        logger.info("LLM disabled — using deterministic fallback")
        return fallback()
    try:
        return client.complete(prompt, system=system)
    except Exception as exc:  # noqa: BLE001 — deliberate graceful degradation
        logger.warning("LLM call failed (%s); using deterministic fallback", exc)
        return fallback()
