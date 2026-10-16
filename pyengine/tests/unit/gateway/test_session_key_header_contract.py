"""会话键请求头的跨实现契约（品牌改名只做了一半 → 下载全 404）。

控制台把会话键放在**请求头**里，而不是 URL：`webui/src/utils/chat/attachmentAccess.ts`
在发请求前会删掉所有带 ``token``/``session`` 的查询参数（``CREDENTIAL_QUERY_KEYS``），
所以 URL 携带的会话键必然被丢弃。改名到品牌名 ``x-Marguerite-session-key`` 后，网关
仍只读旧名 ``x-opensquilla-session-key`` —— 于是每个按会话鉴权的附件/产物下载都解析成
「没有会话」，直接 404。这里钉住两侧必须互相认账。
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

from opensquilla.gateway.session_key import (
    BRANDED_SESSION_HEADER,
    LEGACY_SESSION_HEADER,
    request_session_key,
)


def _repo_root() -> Path:
    """仓库根（含 `webui/` 与 `pyengine/` 的那层）。

    不用固定 `parents[n]`：这个文件从 `tests/test_gateway/` 挪到 `tests/unit/gateway/`
    时层数就变过一次，写死层数会在下次挪动时静默指向错误目录（本测试比对的正是
    跨目录契约，指错就等于没测）。
    """
    for candidate in Path(__file__).resolve().parents:
        if (candidate / "webui" / "src").is_dir() and (candidate / "pyengine" / "src").is_dir():
            return candidate
    raise AssertionError("找不到仓库根（同时含 webui/src 与 pyengine/src）")


REPO_ROOT = _repo_root()
CONTROL_UI_SESSION_KEY_SENDERS = (
    REPO_ROOT / "webui" / "src" / "utils" / "chat" / "attachmentAccess.ts",
    REPO_ROOT / "webui" / "src" / "utils" / "chat" / "artifactAccess.ts",
)


class _FakeSessionManager:
    def __init__(self, session_id: str) -> None:
        self.session_id = session_id

    async def get_session(self, session_key: str) -> object | None:
        if session_key == "agent:main:webchat:ok":
            return type("S", (), {"session_id": self.session_id})()
        return None


def _app(tmp_path: Path):
    pytest.importorskip("starlette.testclient")
    from starlette.applications import Starlette

    from opensquilla.gateway.artifacts import register_artifact_routes
    from opensquilla.gateway.config import AttachmentsConfig, AuthConfig, GatewayConfig
    from opensquilla.gateway.middleware import AuthMiddleware

    config = GatewayConfig(
        host="127.0.0.1",
        auth=AuthConfig(mode="token", token="secret"),
        attachments=AttachmentsConfig(media_root=str(tmp_path)),
    )
    app = Starlette(debug=False)
    register_artifact_routes(app, config=config, session_manager=_FakeSessionManager("session-1"))
    app.add_middleware(AuthMiddleware, config=config)
    return app


def _publish(tmp_path: Path):
    from opensquilla.artifacts import ArtifactStore

    return ArtifactStore(tmp_path).publish_bytes(
        b"hello artifact",
        session_id="session-1",
        session_key="agent:main:webchat:ok",
        name="report final.txt",
        mime="text/plain",
        source="publish_artifact",
    )


def test_artifact_download_accepts_the_branded_session_header(tmp_path: Path) -> None:
    """控制台真正发出的那个头必须能授权下载（缺陷时这里 404）。"""
    pytest.importorskip("starlette.testclient")
    from starlette.testclient import TestClient

    ref = _publish(tmp_path)

    with TestClient(_app(tmp_path)) as client:
        auth = {"Authorization": "Bearer secret"}
        branded = client.get(
            f"/api/v1/artifacts/{ref.id}",
            headers={**auth, BRANDED_SESSION_HEADER: "agent:main:webchat:ok"},
        )
        legacy = client.get(
            f"/api/v1/artifacts/{ref.id}",
            headers={**auth, LEGACY_SESSION_HEADER: "agent:main:webchat:ok"},
        )
        no_session = client.get(f"/api/v1/artifacts/{ref.id}", headers=auth)

    assert branded.status_code == 200, "品牌名会话头必须能授权下载（控制台只发这个）"
    assert legacy.status_code == 200, "旧名必须继续可用（外部客户端/历史客户端）"
    assert no_session.status_code == 404, "没有会话键时仍须拒绝"


def test_request_session_key_prefers_header_over_query() -> None:
    """解析顺序：品牌名头 > 旧名头 > 查询参数（URL 里的凭据会被前端剥掉，只能兜底）。"""
    pytest.importorskip("starlette.testclient")
    from starlette.requests import Request

    def resolve(headers: dict[str, str], query: str = ""):
        raw = [(k.lower().encode(), v.encode()) for k, v in headers.items()]
        scope = {
            "type": "http",
            "method": "GET",
            "path": "/",
            "query_string": query.encode(),
            "headers": raw,
        }
        return request_session_key(Request(scope))

    assert resolve({BRANDED_SESSION_HEADER: "branded"}) == "branded"
    assert resolve({LEGACY_SESSION_HEADER: "legacy"}) == "legacy"
    assert resolve({}, "sessionKey=query") == "query"
    assert resolve({}, "session_key=query") == "query"
    assert resolve({BRANDED_SESSION_HEADER: "branded"}, "sessionKey=query") == "branded"
    assert resolve({}) == ""


def test_control_ui_senders_use_an_accepted_header_spelling() -> None:
    """跨实现守卫：控制台发的会话键头名必须在网关的接受清单里。

    少了这条，「又改一次品牌名」会再次静默把下载打断（本次就是：前端改了、网关没改）。
    以源码为准比对，而不是靠人记得同步两边。
    """
    accepted = {BRANDED_SESSION_HEADER.lower(), LEGACY_SESSION_HEADER.lower()}
    found: dict[str, set[str]] = {}
    for path in CONTROL_UI_SESSION_KEY_SENDERS:
        text = path.read_text(encoding="utf-8")
        names = {m.lower() for m in re.findall(r"['\"](x-[A-Za-z-]*session-key)['\"]", text)}
        assert names, f"{path.name} 里找不到会话键头名（契约形态已变，请同步本守卫）"
        found[path.name] = names

    for name, names in found.items():
        assert names <= accepted, (
            f"{name} 发送的会话键头名 {sorted(names - accepted)} 网关不认识；"
            f"接受清单 = {sorted(accepted)}"
        )
