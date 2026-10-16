#!/usr/bin/env python3
"""网关方法契约离线对账（零依赖，可在 CI 门禁调用）。

校验三件事（对应 core/src/gateway 的三份清单 + 两条 dispatch 路径）：

  [A] 正向：RUST_FIRST_METHODS / PY_FIRST_METHODS ⊆ SUPPORTED_METHODS
      —— 已实装但未登记 = WebUI `supportsMethod()` 探测不到 → 功能静默黑洞。

  [B] 反向：SUPPORTED_METHODS ⊆ dispatch 可达
      —— 登记但未实装 = 前端点了报 NOT_IMPLEMENTED → 假能力。

  [C] 同步可达性：SUPPORTED_METHODS 中未被 dispatch_sync 覆盖的方法，
      必须登记进 ASYNC_ONLY_METHODS，否则 Rust 单测
      `test_supported_methods_subset_of_dispatch` 会失败。

用法：
    python scripts/audit-gateway-methods.py            # 报告模式
    python scripts/audit-gateway-methods.py --strict   # 门禁模式（有缺口 → exit 1）
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PROTOCOL = ROOT / "core" / "src" / "gateway" / "protocol.rs"
METHODS = ROOT / "core" / "src" / "gateway" / "methods.rs"

# dispatch 函数起始行（1-based），用于切分同步 / 异步可达区间
FN_DISPATCH_ASYNC = "pub async fn dispatch("
FN_DISPATCH_RUST = "fn dispatch_rust("
FN_DISPATCH_SYNC = "pub fn dispatch_sync("


def read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


def extract_list(src: str, name: str) -> list[str]:
    """抽取 `NAME: &[&str] = &[ "a", "b", ];` 形式的字符串数组（保序）。"""
    m = re.search(rf"{re.escape(name)}\s*:\s*&\[&str\]\s*=\s*&\[(.*?)\n\];", src, re.S)
    if not m:
        raise SystemExit(f"[FATAL] 未找到清单 {name}")
    body = m.group(1)
    # 去掉行注释，避免注释里的 "xxx" 被当成条目
    body = re.sub(r"//[^\n]*", "", body)
    return re.findall(r'"([^"]+)"', body)


def line_of(src: str, needle: str) -> int:
    idx = src.find(needle)
    if idx < 0:
        raise SystemExit(f"[FATAL] 未找到函数 {needle}")
    return src.count("\n", 0, idx) + 1


def match_arms(src: str, start_line: int, end_line: int) -> set[str]:
    """抽取 [start_line, end_line) 区间内的方法可达点。

    覆盖两种写法：
      1. match arm：`"a.b" =>` / `"a" | "b" =>`
      2. 前置守卫：`if method == "a.b" {`（如 onboarding.provider.probe 在
         async dispatch 里用 if 单独拦截，而非 match arm）
    """
    lines = src.splitlines()
    seg = "\n".join(lines[start_line - 1 : end_line - 1])
    out: set[str] = set()
    for m in re.finditer(r'((?:"[a-z][\w.\-]*"\s*\|\s*)*"[a-z][\w.\-]*")\s*=>', seg):
        out.update(re.findall(r'"([^"]+)"', m.group(1)))
    for m in re.finditer(r'method\s*==\s*"([a-z][\w.\-]*)"', seg):
        out.add(m.group(1))
    return out


def report(title: str, items: list[str]) -> None:
    print(f"\n[{title}] = {len(items)}")
    for it in items:
        print(f"    {it}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--strict", action="store_true", help="发现缺口时退出码 1（CI 门禁）")
    args = ap.parse_args()

    proto = read(PROTOCOL)
    meth = read(METHODS)

    supported = extract_list(proto, "SUPPORTED_METHODS")
    rust_first = extract_list(meth, "RUST_FIRST_METHODS")
    py_first = extract_list(meth, "PY_FIRST_METHODS")
    async_only = extract_list(meth, "ASYNC_ONLY_METHODS")

    l_async = line_of(meth, FN_DISPATCH_ASYNC)
    l_rust = line_of(meth, FN_DISPATCH_RUST)
    l_sync = line_of(meth, FN_DISPATCH_SYNC)
    total = meth.count("\n") + 2

    arms_async = match_arms(meth, l_async, l_rust)
    arms_rust = match_arms(meth, l_rust, l_sync)
    arms_sync_fn = match_arms(meth, l_sync, total)
    # dispatch_sync 会转调 dispatch_rust，故同步可达 = 两者并集
    reachable_sync = arms_rust | arms_sync_fn
    reachable_any = reachable_sync | arms_async

    sup = set(supported)
    print(f"SUPPORTED_METHODS   = {len(supported)} 条（去重 {len(sup)}）")
    print(f"RUST_FIRST_METHODS  = {len(rust_first)} 条")
    print(f"PY_FIRST_METHODS    = {len(py_first)} 条")
    print(f"ASYNC_ONLY_METHODS  = {len(async_only)} 条")
    print(f"dispatch(async) arms= {len(arms_async)}；dispatch_rust/sync arms = {len(reachable_sync)}")

    gaps = 0

    dups = sorted({m for m in supported if supported.count(m) > 1})
    if dups:
        report("X0 SUPPORTED_METHODS 重复项", dups)
        gaps += len(dups)

    a1 = sorted(m for m in rust_first if m not in sup)
    a2 = sorted(m for m in py_first if m not in sup)
    if a1:
        report("A1 RUST_FIRST 未登记 SUPPORTED（前端探测不到）", a1)
        gaps += len(a1)
    if a2:
        report("A2 PY_FIRST 未登记 SUPPORTED（前端探测不到）", a2)
        gaps += len(a2)

    b = sorted(m for m in supported if m not in reachable_any)
    if b:
        report("B  SUPPORTED 登记但 dispatch 不可达（疑似假能力）", b)
        gaps += len(b)

    c = sorted(m for m in supported if m not in reachable_sync and m not in async_only)
    if c:
        report("C  仅异步可达但未登记 ASYNC_ONLY_METHODS（Rust 子集单测会失败）", c)
        gaps += len(c)

    if gaps == 0:
        print("\n[OK] 网关方法契约三向自洽：无缺口。")
        return 0

    print(f"\n[FAIL] 合计 {gaps} 处契约缺口。")
    return 1 if args.strict else 0


if __name__ == "__main__":
    sys.exit(main())
