"""
上传前代码校验：语法 + 质量 + 风险启发式。

对齐 Marguerite core L2 恶意模式扫描的设计思路（零第三方依赖）：
- 语法：Python 用 ast.parse（带行号）；JSON 用 json.loads；JS/TS/HTML/CSS 做括号平衡检查
- 质量：超长行 / 空 except / TODO 遗留 等轻量启发式
- 风险：eval/exec 动态执行、shell 拼接、敏感路径读写、混淆载荷 等危险模式

调用方通过环境变量 BAOTA_SKIP_CODE_CHECK=1 可关闭（默认开启）。
"""
from __future__ import annotations

import ast
import json
import os
import re
from dataclasses import dataclass, field
from pathlib import Path

# --------------------------------------------------------------------------- #
# 风险启发式模式（对齐 L2 MalwareScanner 的典型恶意载荷）
# --------------------------------------------------------------------------- #

# 高危（block）级模式：命中即拒绝写入
_HIGH_RISK_PATTERNS: list[tuple[re.Pattern[str], str]] = [
    (re.compile(r"\beval\s*\([^)]*(input|raw_input|request|os\.|subprocess|__import__)"),
     "eval 动态执行外部输入"),
    (re.compile(r"\bexec\s*\([^)]*(input|raw_input|request|os\.|subprocess|__import__)"),
     "exec 动态执行外部输入"),
    (re.compile(r"\bos\.system\s*\(\s*[f'\"]?[^'\")]*[;|&$]"), "os.system 命令拼接（shell 注入风险）"),
    (re.compile(r"\bsubprocess\.[a-z_]+\([^)]*shell\s*=\s*True"), "subprocess 开启 shell=True"),
    (re.compile(r"\brm\s+-rf\s+/\s*$"), "递归删除根目录"),
    (re.compile(r"\bdd\s+if=[^ ]+\s+of=/dev/(sd[a-z]|hd[a-z]|nvme[^ ]+)"), "直接写磁盘设备"),
    (re.compile(r"\bmkfs\.|format\s+(c:|d:|/dev/)", re.IGNORECASE), "格式化磁盘"),
    (re.compile(r"open\s*\(\s*['\"](/etc/passwd|/etc/shadow|/etc/sudoers|/root/\.ssh[^'\"]*)['\"]\s*,\s*['\"]w"),
     "写入系统敏感文件"),
    (re.compile(r"base64\.b64decode\([^)]*\)\s*\+\s*(os\.|subprocess|eval|exec|__import__)"),
     "base64 混淆载荷"),
    (re.compile(r"__import__\s*\(\s*['\"]os['\"]\s*\)"), "动态导入 os 模块"),
]

# 中危（warn）级模式：命中则提示但放行
_MEDIUM_RISK_PATTERNS: list[tuple[re.Pattern[str], str]] = [
    (re.compile(r"\beval\s*\("), "eval 使用"),
    (re.compile(r"\bexec\s*\("), "exec 使用"),
    (re.compile(r"\brm\s+-rf\b"), "rm -rf 使用"),
    (re.compile(r"\bwget\s+http://|curl\s+http://"), "明文 HTTP 下载"),
    (re.compile(r"updatexml|extractvalue|sleep\s*\(\s*\d+\s*\)|information_schema\.tables"), "SQL 注入特征"),
    (re.compile(r"\bDROP\s+TABLE|DROP\s+DATABASE", re.IGNORECASE), "SQL 删除语句"),
    (re.compile(r"\bpassword\s*=\s*['\"][^'\"]{1,8}['\"]", re.IGNORECASE), "弱密码硬编码"),
    (re.compile(r"api[_-]?key|secret[_-]?key|access[_-]?token"), "疑似密钥硬编码（请确认）"),
]

# --------------------------------------------------------------------------- #
# 质量启发式
# --------------------------------------------------------------------------- #

_MAX_LINE_LEN = 200
_EMPTY_EXCEPT_RE = re.compile(r"except[^:]*:\s*\n\s*(pass|continue)\s*\n")
_TODO_RE = re.compile(r"#\s*(TODO|FIXME|XXX)\b", re.IGNORECASE)


@dataclass
class Issue:
    severity: str  # 'error' | 'warn'
    line: int
    message: str

    def to_dict(self) -> dict:
        return {"severity": self.severity, "line": self.line, "message": self.message}


@dataclass
class CheckResult:
    ok: bool
    syntax_ok: bool
    issues: list[Issue] = field(default_factory=list)
    language: str = ""

    def to_dict(self) -> dict:
        return {
            "ok": self.ok,
            "syntax_ok": self.syntax_ok,
            "language": self.language,
            "issues": [i.to_dict() for i in self.issues],
        }


def detect_language(path: str) -> str:
    """按文件扩展名识别语言；未知返回 ''（不校验语法）。"""
    suffix = Path(path).suffix.lower()
    return {
        ".py": "python",
        ".json": "json",
        ".js": "javascript",
        ".mjs": "javascript",
        ".cjs": "javascript",
        ".ts": "typescript",
        ".tsx": "typescript",
        ".jsx": "javascript",
        ".html": "html",
        ".htm": "html",
        ".css": "css",
        ".php": "php",
        ".sh": "shell",
        ".yaml": "yaml",
        ".yml": "yaml",
        ".toml": "toml",
    }.get(suffix, "")


def _syntax_check(language: str, content: str) -> tuple[bool, list[Issue]]:
    """按语言做语法校验，返回 (是否通过, 问题列表)。"""
    issues: list[Issue] = []
    if language == "python":
        try:
            ast.parse(content)
        except SyntaxError as e:
            issues.append(Issue("error", e.lineno or 1, f"Python 语法错误: {e.msg}"))
            return False, issues
    elif language == "json":
        try:
            json.loads(content)
        except json.JSONDecodeError as e:
            issues.append(Issue("error", e.lineno, f"JSON 语法错误: {e.msg}"))
            return False, issues
    elif language in ("javascript", "typescript", "html", "css", "php", "shell", "yaml", "toml"):
        # 轻量括号/引号平衡检查（不引入第三方解析器）
        stack: list[str] = []
        pairs = {")": "(", "]": "[", "}": "{"}
        line = 1
        in_string: str | None = None
        i = 0
        n = len(content)
        while i < n:
            ch = content[i]
            if ch == "\n":
                line += 1
                i += 1
                continue
            if in_string:
                if ch == in_string and (i == 0 or content[i - 1] != "\\"):
                    in_string = None
                i += 1
                continue
            if ch in ("'", '"', "`"):
                in_string = ch
                i += 1
                continue
            if ch == "#" and language == "shell":
                while i < n and content[i] != "\n":
                    i += 1
                continue
            if ch in ("(", "[", "{"):
                stack.append(ch)
            elif ch in (")", "]", "}"):
                if not stack or stack[-1] != pairs[ch]:
                    issues.append(Issue("error", line, f"括号不匹配: 多余的 {ch!r}"))
                    return False, issues
                stack.pop()
            i += 1
        if in_string:
            issues.append(Issue("error", line, "字符串未闭合"))
            return False, issues
        if stack:
            issues.append(Issue("error", line, f"括号未闭合: 缺少 {''.join(pairs.keys()).strip()}"))
            return False, issues
    return True, issues


def _quality_check(language: str, content: str) -> list[Issue]:
    """轻量质量启发式（不阻断，仅提示）。"""
    issues: list[Issue] = []
    for lineno, line in enumerate(content.splitlines(), start=1):
        if len(line) > _MAX_LINE_LEN:
            issues.append(Issue("warn", lineno, f"行过长（{len(line)} 字符，建议 ≤ {_MAX_LINE_LEN}）"))
            break  # 只提示一次，避免刷屏
    m = _EMPTY_EXCEPT_RE.search(content)
    if m:
        line = content[: m.start()].count("\n") + 1
        issues.append(Issue("warn", line, "空 except 块（静默吞异常，建议记录日志）"))
    for m in _TODO_RE.finditer(content):
        line = content[: m.start()].count("\n") + 1
        issues.append(Issue("warn", line, f"遗留标记: {m.group(1)}"))
    return issues


def _risk_check(content: str) -> list[Issue]:
    """风险启发式：高危 block / 中危 warn。"""
    issues: list[Issue] = []
    for pattern, message in _HIGH_RISK_PATTERNS:
        m = pattern.search(content)
        if m:
            line = content[: m.start()].count("\n") + 1
            issues.append(Issue("error", line, f"[高风险] {message}"))
    for pattern, message in _MEDIUM_RISK_PATTERNS:
        m = pattern.search(content)
        if m:
            line = content[: m.start()].count("\n") + 1
            issues.append(Issue("warn", line, f"[风险] {message}"))
    return issues


def check_code(path: str, content: str) -> CheckResult:
    """对单个文件内容做「语法 + 风险 + 质量」三层校验。

    返回 CheckResult；``ok=False`` 表示存在语法错误或高风险项，调用方应拒绝写入。
    """
    language = detect_language(path)
    result = CheckResult(ok=True, syntax_ok=True, language=language)

    syntax_ok, syntax_issues = _syntax_check(language, content)
    result.syntax_ok = syntax_ok
    result.issues.extend(syntax_issues)

    result.issues.extend(_risk_check(content))
    result.issues.extend(_quality_check(language, content))

    result.ok = result.syntax_ok and not any(i.severity == "error" for i in result.issues)
    return result


def skip_code_check() -> bool:
    """环境变量 BAOTA_SKIP_CODE_CHECK=1 时跳过校验（默认开启）。"""
    return os.environ.get("BAOTA_SKIP_CODE_CHECK", "").strip() in ("1", "true", "True")


def format_report(result: CheckResult, path: str) -> str:
    """把校验结果格式化为可读文本（供 MCP 工具返回）。"""
    lines = [f"代码校验（{result.language or '未知语言'}）: {'通过' if result.ok else '未通过'}"]
    if not result.issues:
        return "\n".join(lines)
    lines.append("校验详情:")
    for issue in result.issues:
        marker = "✗" if issue.severity == "error" else "△"
        loc = f"L{issue.line}" if issue.line else "-"
        lines.append(f"  {marker} [{issue.severity}] {loc} {issue.message}")
    return "\n".join(lines)
