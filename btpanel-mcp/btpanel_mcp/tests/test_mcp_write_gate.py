"""MCP 写入/上传路径的语法门禁「接线」测试。

区别于 test_code_check.py（只测校验器本身），本文件验证的是：
校验器确实被挂在 _h_file_write / _h_file_upload 的落盘路径上，
且被拒绝时**不会**调用底层 api.write_file / api.upload_file。
"""
from __future__ import annotations

import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from btpanel_mcp.bt_api import BaotaConfig  # noqa: E402
from btpanel_mcp.mcp_server import (  # noqa: E402
    BtMCP,
    _h_file_upload,
    _h_file_write,
)


class _FakeAPI:
    """记录调用的假 API：任何一次落盘调用都会被记录下来。"""

    def __init__(self):
        self.writes: list[tuple[str, str]] = []
        self.uploads: list[tuple[str, str]] = []

    def has_credentials(self) -> bool:
        return True

    def write_file(self, path, content, encoding="utf-8"):
        self.writes.append((path, content))
        return {"ok": True}

    def upload_file(self, local, remote, chunk):
        self.uploads.append((local, remote))
        return {"ok": True}


class _FakeSrv(BtMCP):
    def __init__(self, api):
        super().__init__(BaotaConfig(base_url="http://127.0.0.1:8888"))
        self._fake = api

    @property
    def api(self):
        return self._fake


class FileWriteGateTest(unittest.TestCase):
    def setUp(self):
        os.environ.pop("BAOTA_SKIP_CODE_CHECK", None)
        self.api = _FakeAPI()
        self.srv = _FakeSrv(self.api)

    def test_broken_python_is_rejected_before_write(self):
        text, is_error = _h_file_write(
            self.srv, {"path": "/www/wwwroot/app.py", "content": "def foo(:\n    pass\n"}
        )
        self.assertTrue(is_error)
        self.assertIn("已拒绝写入", text)
        self.assertEqual(self.api.writes, [], "语法错误时绝不能落盘")

    def test_valid_python_reaches_write(self):
        text, is_error = _h_file_write(
            self.srv, {"path": "/www/wwwroot/app.py", "content": "def foo():\n    return 1\n"}
        )
        self.assertFalse(is_error, text)
        self.assertEqual(len(self.api.writes), 1, "合法代码应正常落盘")

    def test_non_code_file_is_not_false_flagged(self):
        # 纯文本里的不平衡括号不应被误判为语法错误
        text, is_error = _h_file_write(
            self.srv, {"path": "/www/wwwroot/notes.txt", "content": "备注 ((( 未闭合 {{{"}
        )
        self.assertFalse(is_error, text)
        self.assertEqual(len(self.api.writes), 1)

    def test_high_risk_payload_is_rejected(self):
        text, is_error = _h_file_write(
            self.srv,
            {"path": "/www/wwwroot/evil.py", "content": "import os\neval(os.environ['X'])\n"},
        )
        self.assertTrue(is_error)
        self.assertEqual(self.api.writes, [], "高危载荷不得落盘")

    def test_skip_flag_allows_override(self):
        os.environ["BAOTA_SKIP_CODE_CHECK"] = "1"
        try:
            _, is_error = _h_file_write(
                self.srv, {"path": "/www/wwwroot/app.py", "content": "def foo(:\n"}
            )
            self.assertFalse(is_error)
            self.assertEqual(len(self.api.writes), 1)
        finally:
            os.environ.pop("BAOTA_SKIP_CODE_CHECK", None)


class FileUploadGateTest(unittest.TestCase):
    def setUp(self):
        os.environ.pop("BAOTA_SKIP_CODE_CHECK", None)
        self.api = _FakeAPI()
        self.srv = _FakeSrv(self.api)
        self._tmp: list[Path] = []

    def tearDown(self):
        for p in self._tmp:
            try:
                p.unlink()
            except OSError:
                pass

    def _write_tmp(self, suffix: str, data: bytes) -> str:
        fd, name = tempfile.mkstemp(suffix=suffix)
        with os.fdopen(fd, "wb") as f:
            f.write(data)
        self._tmp.append(Path(name))
        return name

    def test_broken_local_file_is_rejected_before_upload(self):
        local = self._write_tmp(".py", b"def broken(:\n    pass\n")
        text, is_error = _h_file_upload(
            self.srv, {"local_path": local, "remote_dir": "/www/wwwroot"}
        )
        self.assertTrue(is_error)
        self.assertIn("已拒绝上传", text)
        self.assertEqual(self.api.uploads, [], "语法错误时绝不能上传")

    def test_valid_local_file_uploads(self):
        local = self._write_tmp(".py", b"def ok():\n    return 1\n")
        text, is_error = _h_file_upload(
            self.srv, {"local_path": local, "remote_dir": "/www/wwwroot"}
        )
        self.assertFalse(is_error, text)
        self.assertEqual(len(self.api.uploads), 1)

    def test_binary_file_is_not_blocked(self):
        # 二进制内容无法 UTF-8 解码 → 跳过语法校验，正常上传
        local = self._write_tmp(".bin", bytes([0x7F, 0x45, 0x4C, 0x46, 0x00, 0xFF, 0xFE]))
        text, is_error = _h_file_upload(
            self.srv, {"local_path": local, "remote_dir": "/www/wwwroot"}
        )
        self.assertFalse(is_error, text)
        self.assertEqual(len(self.api.uploads), 1, "二进制产物不应被语法门禁误杀")


if __name__ == "__main__":
    unittest.main()
