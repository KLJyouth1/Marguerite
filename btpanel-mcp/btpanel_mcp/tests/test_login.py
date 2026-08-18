"""baota_login / 凭据文件 单测（mock urllib，标准库 unittest）。"""
from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from btpanel_mcp.bt_api import BaotaAPI, BaotaConfig  # noqa: E402


def _html_with_linux_token() -> bytes:
    return b'<html>window.__token__ = "abc123"; var request_token="linux_tok_9";</html>'


class _FakeResp:
    def __init__(self, payload: bytes, status: int = 200, set_cookies: list[str] | None = None):
        self._payload = payload
        self.status = status
        self._set_cookies = set_cookies or []

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def get_all(self, name: str) -> list[str]:
        return self._set_cookies if name.lower() == "set-cookie" else []

    def read(self) -> bytes:
        return self._payload


class LoginTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.cred_file = str(Path(self.tmp.name) / "btpanel-credentials.json")
        self.config = BaotaConfig(
            base_url="http://127.0.0.1:8888/ABCD1234",
            credentials_file=self.cred_file,
        )

    def tearDown(self):
        self.tmp.cleanup()

    def test_login_success_saves_credentials(self):
        """登录成功：提取 session cookie，写入凭据文件，密码不回显于状态。"""
        api = BaotaAPI(self.config)
        # 首页 token 探测 + login 两个请求
        calls = []

        def fake_urlopen(req, timeout=30):
            calls.append(req.full_url)
            if req.full_url.rstrip("/").endswith("/login"):
                body = {"ok": True, "msg": "登录成功", "data": {"request_token": "win_tok_7"}}
                return _FakeResp(json.dumps(body).encode(), set_cookies=["session=abc123; Path=/"])
            return _FakeResp(_html_with_linux_token())

        with mock.patch("urllib.request.urlopen", side_effect=fake_urlopen):
            r = api.login("admin", "secret")

        self.assertTrue(r.get("ok"), r)
        self.assertEqual(api._cookies.get("session"), "abc123")
        # 凭据文件已落盘
        saved = json.loads(Path(self.cred_file).read_text(encoding="utf-8"))
        self.assertEqual(saved["username"], "admin")
        self.assertEqual(saved["password"], "secret")
        self.assertEqual(saved["cookies"]["session"], "abc123")
        # 状态不暴露密码
        status = api.credentials_status()
        self.assertNotIn("password", json.dumps(status))
        self.assertTrue(status["credentials_file_exists"])

    def test_login_failure_no_credentials(self):
        """登录失败：无 session/token 时返回 ok=False，不写凭据。"""
        api = BaotaAPI(self.config)

        def fake_urlopen(req, timeout=30):
            if req.full_url.rstrip("/").endswith("/login"):
                return _FakeResp('{"ok": false, "msg": "账号或密码错误"}'.encode("utf-8"), status=200)
            return _FakeResp(_html_with_linux_token())

        with mock.patch("urllib.request.urlopen", side_effect=fake_urlopen):
            r = api.login("admin", "wrong")

        self.assertFalse(r.get("ok"))
        self.assertFalse(Path(self.cred_file).exists())

    def test_ensure_token_auto_relogin_from_file(self):
        """凭据文件只有账号密码时，首次请求自动登录。"""
        # 预写仅账号密码的凭据文件
        Path(self.cred_file).write_text(json.dumps({
            "base_url": "http://127.0.0.1:8888/ABCD1234",
            "username": "admin",
            "password": "secret",
            "cookies": {},
        }), encoding="utf-8")
        api = BaotaAPI(self.config)  # __init__ 会加载文件

        def fake_urlopen(req, timeout=30):
            if req.full_url.rstrip("/").endswith("/login"):
                return _FakeResp(json.dumps({"ok": True, "msg": "ok"}).encode(),
                                 set_cookies=["session=auto_9; Path=/"])
            return _FakeResp(_html_with_linux_token())

        with mock.patch("urllib.request.urlopen", side_effect=fake_urlopen):
            api._ensure_token()

        self.assertEqual(api._cookies.get("session"), "auto_9")

    def test_clear_credentials_removes_file(self):
        """登出：删除凭据文件并清空会话。"""
        Path(self.cred_file).write_text(json.dumps({
            "base_url": "http://127.0.0.1:8888/ABCD1234",
            "cookies": {"session": "x"},
        }), encoding="utf-8")
        api = BaotaAPI(self.config)
        self.assertTrue(api.has_credentials())
        self.assertTrue(api.clear_credentials())
        self.assertFalse(Path(self.cred_file).exists())
        self.assertFalse(api.has_credentials())

    def test_no_credentials_path_without_home(self):
        """无凭据文件配置且 home 不可用时，resolved 返回 None（不崩溃）。"""
        cfg = BaotaConfig(base_url="http://127.0.0.1:8888", credentials_file="")
        with mock.patch("pathlib.Path.home", side_effect=RuntimeError("no home")):
            self.assertIsNone(cfg.resolved_credentials_path())


if __name__ == "__main__":
    unittest.main()
