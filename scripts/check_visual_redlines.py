#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""视觉红线扫描器（Phase D 门禁）。

扫描前端源码（desktop / tui / core::tui）中的 P0 视觉红线违规：
  R1  emoji 作为功能图标（UI 字符串中的 emoji 字符）
  R2  紫→粉渐变主视觉（Indigo→Pink 组合）
  R3  硬编码颜色（绕过 theme.rs / workbench::theme 令牌）
  R4  AI 模板味占位文案（Lorem ipsum / Welcome to Our App 等）

退出码：0 = 无违规；1 = 存在违规。
用法：
  python scripts/check_visual_redlines.py            # 扫描默认目标
  python scripts/check_visual_redlines.py --verbose  # 打印每条命中行
"""
from __future__ import annotations

import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# 扫描目标：两个原生前端 + core 内的 TUI 实现
TARGETS = [
    os.path.join(ROOT, "desktop", "src"),
    os.path.join(ROOT, "tui", "src"),
    os.path.join(ROOT, "core", "src", "tui"),
]

# 双前端批准字形契约的 Rust SSOT（用于 --self-test 漂移校验）
GLYPH_RS = os.path.join(ROOT, "core", "src", "tui", "glyph.rs")

# ── R1: emoji ────────────────────────────────────────────────────────────
# 覆盖表情/符号/象形/旗帜/变体选择符/零宽连接符等区段。
EMOJI_RE = re.compile(
    "["
    "\U0001F300-\U0001F9FF"
    "\U00002600-\U000026FF"
    "\U00002700-\U000027BF"
    "\U0000FE00-\U0000FE0F"
    "\U0001F000-\U0001F0FF"
    "\U0001F100-\U0001F64F"
    "\U0001F680-\U0001F6FF"
    "\U0001FA00-\U0001FAFF"
    "\U00002300-\U000023FF"  # 杂项符号与箭头：⏳⌛⏰⏱⏲ 等计时类 emoji（补 J5 同款缺口）
    "\U0000200D"
    "\U000020E3"
    "]"
)

# 设计系统明确批准的单色几何状态字形（非 emoji 功能图标）。
# SSOT = core/src/tui/glyph.rs 的 APPROVED_GLYPHS（Rust 唯一真相源）。
# 本副本由脚本 --self-test 解析 glyph.rs 校验，防止两份真相漂移。
APPROVED_GLYPHS = frozenset("▮✖◉·●○►▸›‹█▓▒░")

# 注意：U+2500-U+257F 制表符、U+2580-U+259F 块元素不在 EMOJI_RE 范围内，无需豁免。

# ── R2: 紫粉渐变 ─────────────────────────────────────────────────────────
PURPLE_HEX = re.compile(r"0x7C,\s*0x3A,\s*0xED|#7C3AED|0x6366F1|#6366F1|0xA855F7|#A855F7", re.I)
PINK_HEX = re.compile(r"0xEC,\s*0x48,\s*0x99|#EC4899|0xDB2777|#DB2777", re.I)

# ── R3: 硬编码颜色 ───────────────────────────────────────────────────────
# 仅当括号内首个实参是「数字字面量」时才算构造颜色；
# 若为标识符（如 `Color::Rgb(r, g, b)`）则是模式匹配解构，不是硬编码，跳过。
HARDCODE_EGUI = re.compile(r"Color32::from_rgb\s*\(\s*(?:0x[0-9A-Fa-f]+|\d+)")
HARDCODE_RATATUI = re.compile(r"Color::Rgb\s*\(\s*(?:0x[0-9A-Fa-f]+|\d+)")
# 允许的白/黑（唯一例外）
ALLOW_BW = re.compile(r"0xFF,\s*0xFF,\s*0xFF|0x00,\s*0x00,\s*0x00|\(\s*255,\s*255,\s*255\s*\)|\(\s*0,\s*0,\s*0\s*\)")

# theme / 契约文件豁免 R3（令牌定义处本就该写字面色值）
THEME_EXEMPT = ("theme.rs", "glyph.rs")

# ── R4: AI 模板味文案 ────────────────────────────────────────────────────
SLOP_RE = re.compile(
    r"Lorem ipsum|Welcome to Our App|Sign up today|Your journey begins|"
    r"Unleash the power|Get started today|placeholder text",
    re.I,
)


def iter_rust_files():
    for target in TARGETS:
        if not os.path.isdir(target):
            continue
        for dirpath, _dirnames, filenames in os.walk(target):
            for fn in filenames:
                if fn.endswith(".rs"):
                    yield os.path.join(dirpath, fn)


def rel(path: str) -> str:
    return os.path.relpath(path, ROOT).replace("\\", "/")


def scan():
    """扫描全部前端源码。

    返回 (violations, advisories)：
      violations —— 生产渲染路径上的违规，阻断门禁。
      advisories —— `#[cfg(test)]` 测试区内的命中，仅提示（测试数据/检测器自身的
                    禁用清单不会渲染给用户，一刀切会淹没真实问题）。
    """
    violations = {"R1": [], "R2": [], "R3": [], "R4": []}
    advisories = {"R1": [], "R2": [], "R3": [], "R4": []}

    for path in iter_rust_files():
        base = os.path.basename(path)
        is_theme = base in THEME_EXEMPT
        try:
            with open(path, "r", encoding="utf-8") as fh:
                lines = fh.readlines()
        except (OSError, UnicodeDecodeError) as exc:
            print("WARN unreadable %s: %s" % (rel(path), exc))
            continue

        saw_purple = False
        saw_pink = False
        in_test = False

        for lineno, line in enumerate(lines, start=1):
            stripped = line.strip()

            # 进入测试区后，本文件剩余内容均按 advisory 计（Rust 惯例：测试模块置于文件末尾）
            if stripped.startswith("#[cfg(test)]"):
                in_test = True
            bucket = advisories if in_test else violations

            # R1 emoji（白名单内的单色几何状态字形不算功能图标）
            for m in EMOJI_RE.finditer(line):
                ch = m.group(0)
                if ch in APPROVED_GLYPHS:
                    continue
                bucket["R1"].append((rel(path), lineno, repr(ch), stripped[:100]))

            # R2 紫粉渐变（同文件同时出现紫与粉才算渐变组合；测试区的禁用清单不计）
            if not in_test:
                if PURPLE_HEX.search(line):
                    saw_purple = True
                if PINK_HEX.search(line):
                    saw_pink = True

            # R3 硬编码颜色（theme/契约文件豁免；纯黑白豁免；模式匹配已由正则排除）
            if not is_theme and (HARDCODE_EGUI.search(line) or HARDCODE_RATATUI.search(line)):
                if not ALLOW_BW.search(line):
                    bucket["R3"].append((rel(path), lineno, "hardcoded color", stripped[:100]))

            # R4 模板味文案
            m4 = SLOP_RE.search(line)
            if m4:
                bucket["R4"].append((rel(path), lineno, m4.group(0), stripped[:100]))

        if saw_purple and saw_pink:
            violations["R2"].append((rel(path), 0, "purple+pink in same file", ""))

    return violations, advisories


LABELS = {
    "R1": "emoji 作功能图标",
    "R2": "紫->粉渐变主视觉",
    "R3": "硬编码颜色（应走设计令牌）",
    "R4": "AI 模板味占位文案",
}


def _print_hits(hits, verbose):
    if hits and (verbose or len(hits) <= 40):
        for path, lineno, tok, ctx in hits:
            loc = "%s:%d" % (path, lineno) if lineno else path
            print("       %s  %s" % (loc, tok))
            if ctx:
                print("           | %s" % ctx)
    elif hits:
        print("       (%d 处，加 --verbose 查看明细)" % len(hits))


def self_test() -> int:
    """校验本脚本的 APPROVED_GLYPHS 与 core/src/tui/glyph.rs 的 SSOT 不漂移。"""
    try:
        with open(GLYPH_RS, "r", encoding="utf-8") as fh:
            src = fh.read()
    except OSError as exc:
        print("SELF-TEST FAIL: 无法读取 %s: %s" % (GLYPH_RS, exc))
        return 1
    m = re.search(r"pub const APPROVED_GLYPHS:\s*&\[char\]\s*=\s*&\[([^\]]*)\];", src)
    if not m:
        print("SELF-TEST FAIL: 未在 glyph.rs 找到 APPROVED_GLYPHS 定义")
        return 1
    ssot = frozenset(re.findall(r"'([^'\\]*)'", m.group(1)))
    if ssot == APPROVED_GLYPHS:
        print("SELF-TEST PASS: 扫描器 APPROVED_GLYPHS 与 glyph.rs SSOT 一致 (%d 个字形)" % len(ssot))
        return 0
    missing = APPROVED_GLYPHS - ssot
    extra = ssot - APPROVED_GLYPHS
    print("SELF-TEST FAIL: 两份真相漂移")
    if missing:
        print("  扫描器多余而 SSOT 缺: %s" % " ".join(sorted(missing)))
    if extra:
        print("  SSOT 多余而扫描器缺: %s" % " ".join(sorted(extra)))
    return 1


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    verbose = "--verbose" in sys.argv
    violations, advisories = scan()
    total = sum(len(v) for v in violations.values())
    adv_total = sum(len(v) for v in advisories.values())

    print("=" * 68)
    print("Phase D 视觉红线扫描  (root=%s)" % ROOT)
    print("=" * 68)
    print("生产渲染路径（阻断门禁）")
    for code in ("R1", "R2", "R3", "R4"):
        hits = violations[code]
        status = "PASS" if not hits else "FAIL"
        print("[%s] %s  %s : %d 处" % (status, code, LABELS[code], len(hits)))
        _print_hits(hits, verbose)

    if adv_total:
        print("-" * 68)
        print("测试区提示（不阻断；测试数据与检测器自身清单不渲染给用户）")
        for code in ("R1", "R2", "R3", "R4"):
            hits = advisories[code]
            if not hits:
                continue
            print("[ADVISORY] %s  %s : %d 处" % (code, LABELS[code], len(hits)))
            _print_hits(hits, verbose)

    print("-" * 68)
    if total == 0:
        print("结论: 生产路径视觉红线全部通过（另有 %d 处测试区提示）。" % adv_total)
        return 0
    print("结论: 生产路径发现 %d 处违规，必须修复后方可通过 Phase D 门禁。" % total)
    return 1


if __name__ == "__main__":
    sys.exit(main())
