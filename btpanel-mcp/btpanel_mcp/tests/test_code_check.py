"""code_check 模块单测（标准库 unittest，无需第三方依赖）。"""
from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from btpanel_mcp.code_check import (  # noqa: E402
    check_code,
    detect_language,
    format_report,
)


class SyntaxCheckTest(unittest.TestCase):
    def test_python_syntax_error_detected(self):
        result = check_code("app.py", "def foo(:\n    pass\n")
        self.assertFalse(result.ok)
        self.assertFalse(result.syntax_ok)
        self.assertTrue(any(i.severity == "error" and "语法" in i.message for i in result.issues))

    def test_python_benign_passes(self):
        result = check_code("app.py", "def add(a, b):\n    return a + b\n")
        self.assertTrue(result.ok)
        self.assertTrue(result.syntax_ok)

    def test_json_syntax_error_detected(self):
        result = check_code("config.json", '{"a": 1,,}')
        self.assertFalse(result.ok)
        self.assertTrue(any(i.severity == "error" for i in result.issues))

    def test_json_valid_passes(self):
        result = check_code("config.json", '{"a": 1, "b": [2, 3]}')
        self.assertTrue(result.ok)

    def test_js_unbalanced_brackets_detected(self):
        result = check_code("main.js", "const x = [1, 2;\nconsole.log(x);")
        self.assertFalse(result.ok)
        self.assertTrue(any(i.severity == "error" and "括号" in i.message for i in result.issues))

    def test_binary_file_skipped_via_language(self):
        # 二进制文件无扩展名识别 → 不做语法校验，仅风险扫描
        self.assertEqual(detect_language("logo.bin"), "")


class RiskCheckTest(unittest.TestCase):
    def test_eval_os_system_blocked(self):
        content = 'import os\nx = eval("os.system(\'rm -rf /tmp/a\')")\n'
        result = check_code("evil.py", content)
        self.assertFalse(result.ok)
        self.assertTrue(any("eval 动态执行" in i.message for i in result.issues))

    def test_rm_rf_root_blocked(self):
        result = check_code("a.sh", "rm -rf /\n")
        self.assertFalse(result.ok)
        self.assertTrue(any("删除根目录" in i.message for i in result.issues))

    def test_subprocess_shell_true_warned(self):
        result = check_code("s.py", "import subprocess\nsubprocess.call('ls', shell=True)\n")
        self.assertFalse(result.ok)
        self.assertTrue(any("shell=True" in i.message for i in result.issues))

    def test_sensitive_file_write_blocked(self):
        result = check_code("x.py", "open('/etc/passwd', 'w').write('root:x')\n")
        self.assertFalse(result.ok)
        self.assertTrue(any("敏感文件" in i.message for i in result.issues))

    def test_warn_level_does_not_block(self):
        # 仅中危告警（SQL 注入特征）不阻断写入（.sql 不做语法校验）
        result = check_code("q.sql", "SELECT * FROM users WHERE id = extractvalue(1, 2)\n")
        self.assertTrue(result.ok)
        self.assertTrue(any(i.severity == "warn" and "SQL 注入" in i.message for i in result.issues))


class QualityAndReportTest(unittest.TestCase):
    def test_long_line_warns(self):
        result = check_code("app.py", "x = '" + "a" * 250 + "'\n")
        self.assertTrue(result.ok)
        self.assertTrue(any(i.severity == "warn" and "行过长" in i.message for i in result.issues))

    def test_empty_except_warns(self):
        result = check_code("app.py", "try:\n    pass\nexcept Exception:\n    pass\n")
        self.assertTrue(result.ok)
        self.assertTrue(any("空 except" in i.message for i in result.issues))

    def test_format_report_contains_summary(self):
        result = check_code("bad.py", "def foo(:\n")
        text = format_report(result, "bad.py")
        self.assertIn("未通过", text)
        self.assertIn("L1", text)


if __name__ == "__main__":
    unittest.main()
