"""会话键解析（Control UI 下载路由的唯一入口）。

控制台把会话键放在**请求头**，不放在 URL：`webui/src/utils/chat/attachmentAccess.ts`
在发请求前删除所有含 ``token``/``session`` 的查询参数（``CREDENTIAL_QUERY_KEYS``），
URL 携带的键必然被丢掉。头部曾改名到品牌名 ``x-Marguerite-session-key``，而网关只读
旧名 ``x-opensquilla-session-key`` —— 每个按会话鉴权的附件/产物下载都解析成「没有会话」
并回 404。这里统一接受两种拼写（品牌名优先），并保留查询参数给非浏览器客户端。
"""

from __future__ import annotations

from starlette.requests import Request

#: 控制台当前发送的头名（`webui/src/utils/chat/*Access.ts`）
BRANDED_SESSION_HEADER = "x-Marguerite-session-key"
#: 改名前的头名，外部/历史客户端仍可能使用
LEGACY_SESSION_HEADER = "x-opensquilla-session-key"

_HEADERS = (BRANDED_SESSION_HEADER, LEGACY_SESSION_HEADER)
_QUERY_KEYS = ("sessionKey", "session_key")


def request_session_key(request: Request) -> str:
    """请求里的会话键；没有任何来源时返回空串。

    顺序：品牌名头 → 旧名头 → 查询参数。头优先是刻意的：URL 里的凭据会进日志与
    Referer，控制台已经统一把它们剥掉，查询参数只为兼容非浏览器调用方保留。
    """
    for name in _HEADERS:
        value = request.headers.get(name)
        if value:
            return value
    for name in _QUERY_KEYS:
        value = request.query_params.get(name)
        if value:
            return value
    return ""
