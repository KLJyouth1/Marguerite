"""
开发规范加载与精简：把仓库上级 docs/ 下的开发规范注入系统提示。

设计要点：
- 定位：环境变量 ``MARGUERITE_CODING_GUIDELINES_DIR`` 显式指定，否则从 cwd 上溯
  查找含 ``后端开发规范_v1.1.md`` 的 ``docs`` 目录；找不到返回 None（不注入，向后兼容）。
- 精简：只提取 ``##`` 标题 + 每节首段「原则」，丢弃大段 Marguerite 案例与门禁全文，
  控制注入体积（默认每份 ≤ 1600 字符，总 ≤ 7000）。
- 缓存：模块级 mtime 感知缓存，docs 未变时不重复读盘。
"""
from __future__ import annotations

import os
import re
from pathlib import Path

# 每份规范注入字符预算
_PER_FILE_CHARS = 1600
_TOTAL_CHARS = 7000

# 规范文件名（v1.1 优先，缺失时回退 v1.0）
_BACKEND_CANDIDATES = ("后端开发规范_v1.1.md", "后端开发规范_v1.0.md")
_FRONTEND_CANDIDATES = ("前端开发规范_v1.1.md", "前端开发规范_v1.0.md")

_HEADING_RE = re.compile(r"^##\s+(.+)$", re.MULTILINE)

_cache: dict[str, tuple[float, str | None]] = {}


def _find_docs_dir() -> Path | None:
    """定位 docs 目录：env 优先，否则从 cwd 上溯最多 8 层。"""
    explicit = os.environ.get("MARGUERITE_CODING_GUIDELINES_DIR", "").strip()
    if explicit:
        candidate = Path(explicit)
        if candidate.is_dir() and (candidate / _BACKEND_CANDIDATES[0]).exists():
            return candidate
    current = Path.cwd().resolve()
    for _ in range(8):
        docs = current / "docs"
        if docs.is_dir() and (docs / _BACKEND_CANDIDATES[0]).exists():
            return docs
        parent = current.parent
        if parent == current:
            break
        current = parent
    return None


def _pick_file(docs_dir: Path, candidates: tuple[str, ...]) -> Path | None:
    for name in candidates:
        candidate = docs_dir / name
        if candidate.is_file():
            return candidate
    return None


def _summarize_sections(text: str, max_chars: int) -> str:
    """按 `##` 标题切分，每节保留标题 + 首个非空段落，压缩到预算内。"""
    # 定位正文起点（跳过文件头的标题/引言行）
    body_start = 0
    m = re.search(r"^##\s+", text, re.MULTILINE)
    if m:
        body_start = m.start()

    sections: list[str] = []
    for heading in _HEADING_RE.finditer(text):
        heading_start = heading.start()
        if heading_start < body_start:
            continue
        # 标题下的第一个非空段落
        chunk = text[heading_start + len(heading.group(0)):]
        para_lines: list[str] = []
        for line in chunk.splitlines():
            if not line.strip():
                continue
            if line.strip().startswith("**") and "案例**" in line:
                break  # Marguerite 案例细节不注入，只保留原则
            if line.lstrip().startswith(("#", "---", "|", ">", "- [", "```")):
                # 表格/清单/引用/代码块太长，仅保留简短行
                if line.lstrip().startswith("- ") and len(line) <= 120:
                    para_lines.append(line.strip())
                continue
            para_lines.append(line.strip())
            if len(" ".join(para_lines)) >= 220:
                break
        if not para_lines:
            continue
        section_text = f"{heading.group(1).strip()}: " + " ".join(para_lines)
        sections.append(section_text)

    if not sections:
        # 无 `##` 标题（v1.0 可能不同），直接截断
        return text[:max_chars].strip()

    # 尽量保留每节开头，压缩总长
    per_section = max(120, max_chars // max(1, len(sections)))
    parts: list[str] = []
    budget = max_chars
    for section in sections:
        if budget <= 0:
            break
        piece = section[:per_section].rstrip("。；;,")
        parts.append(piece)
        budget -= len(piece)
    return "\n".join(parts).strip()


def load_coding_guidelines() -> str | None:
    """加载并精简四份开发规范；未定位到 docs 目录时返回 None。"""
    docs_dir = _find_docs_dir()
    if docs_dir is None:
        return None
    stamp = docs_dir.stat().st_mtime_ns
    cached = _cache.get(str(docs_dir))
    if cached is not None and cached[0] == stamp:
        return cached[1]

    sections: list[str] = []
    total = 0
    for label, candidates in (("后端", _BACKEND_CANDIDATES), ("前端", _FRONTEND_CANDIDATES)):
        file_path = _pick_file(docs_dir, candidates)
        if file_path is None:
            continue
        try:
            text = file_path.read_text(encoding="utf-8")
        except OSError:
            continue
        summary = _summarize_sections(text, _PER_FILE_CHARS)
        if summary:
            header = f"[{label}开发规范 · {file_path.name.replace('.md', '')}]"
            sections.append(f"{header}\n{summary}")
            total += len(summary) + len(header)

    result: str | None
    if not sections:
        result = None
    else:
        body = "\n\n".join(sections)[:_TOTAL_CHARS]
        result = (
            "以下是 Marguerite 用户通用开发规范的精简要点，生成代码时必须遵循：\n\n" + body
        )
    _cache[str(docs_dir)] = (stamp, result)
    return result
