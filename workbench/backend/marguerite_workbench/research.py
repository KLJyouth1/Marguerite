"""联网 research endpoint.

When ``SERPER_API_KEY`` (or ``UNIV_SEARCH_KEY``) is configured, performs a real
web search via the Serper API.  Otherwise it degrades gracefully to a local-KB
search and clearly labels the response as an offline fallback.  It must never
crash offline (Bloom R-rule: no swallow, log + fallback).
"""

from __future__ import annotations

import logging
import os
from typing import Optional

from marguerite_workbench.llm import llm_or_fallback
from marguerite_workbench.models import ResearchResult

logger = logging.getLogger("marguerite.research")

ENV_SERPER = "SERPER_API_KEY"
ENV_UNIV_SEARCH = "UNIV_SEARCH_KEY"
SERPER_URL = "https://google.serper.dev/search"

# Local KB handle is injected at call time (avoids an import cycle with api.py).
_KB = None


def set_kb(kb) -> None:
    global _KB
    _KB = kb


def _serper_key() -> Optional[str]:
    return os.environ.get(ENV_SERPER) or os.environ.get(ENV_UNIV_SEARCH)


def _serper_search(query: str, api_key: str) -> list[dict]:
    import httpx  # lazy (optional dep)

    resp = httpx.post(
        SERPER_URL,
        headers={"X-API-KEY": api_key, "Content-Type": "application/json"},
        json={"q": query, "gl": "cn", "hl": "zh-cn"},
        timeout=30,
    )
    resp.raise_for_status()
    data = resp.json()
    results: list[dict] = []
    for item in data.get("organic", [])[:8]:
        results.append({
            "title": item.get("title", ""),
            "url": item.get("link", ""),
            "snippet": (item.get("snippet") or item.get("description") or "")[:280],
        })
    return results


def research(query: str) -> ResearchResult:
    key = _serper_key()
    if key:
        try:
            results = _serper_search(query, key)

            def _synthesize() -> str:
                if not results:
                    return "联网检索未返回结果。"
                lines = [f"{i+1}. {r['title']} — {r['snippet']}" for i, r in enumerate(results[:5])]
                return "联网检索结果摘要：\n" + "\n".join(lines)

            synthesized = llm_or_fallback(
                f"综合以下检索结果回答用户问题「{query}」：\n"
                + "\n".join(f"- {r['title']}: {r['snippet']}" for r in results[:5]),
                _synthesize,
                system="You summarize web search results concisely in the user's language.",
            )
            return ResearchResult(query=query, results=results, synthesized=synthesized)
        except Exception as exc:  # noqa: BLE001 — fall back, never crash
            logger.warning("serper search failed (%s); using local KB fallback", exc)

    # --- Graceful local-KB fallback ---
    hits = _KB.search(query, k=5) if _KB is not None else []
    results = [
        {"title": h["source"], "url": f"local://kb/{h.get('source', '')}", "snippet": h["text"][:280]}
        for h in hits
    ]
    synthesized = (
        f"[离线回退] 未配置联网搜索密钥（SERPER_API_KEY / UNIV_SEARCH_KEY），"
        f"已对本地知识库执行检索，命中 {len(results)} 条。\n"
        + ("\n".join(f"- {h['source']}: {h['text'][:160]}" for h in hits[:5]) if hits else "本地知识库也无相关记录。")
    )
    return ResearchResult(query=query, results=results, synthesized=synthesized)
