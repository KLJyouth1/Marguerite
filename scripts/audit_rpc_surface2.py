#!/usr/bin/env python3
"""审计二（修正版）：完整分派链路 = dispatch_rust 同步 arms + async dispatch match/if + RUST_FIRST 路径"""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "core" / "src"
methods_rs = (ROOT / "gateway" / "methods.rs").read_text(encoding="utf-8")
proto = (ROOT / "gateway" / "protocol.rs").read_text(encoding="utf-8")

m = re.search(r"pub const SUPPORTED_METHODS: &\[&str\] = &\[(.*?)\];", proto, re.S)
declared = set(re.findall(r'"([^"]+)"', m.group(1)))

# dispatch_rust arms
idx = methods_rs.find("fn dispatch_rust")
seg = methods_rs[idx:idx + 30000]
end = seg.find("\n}\n", seg.find("match method"))
sync_arms = set(re.findall(r'"([a-z][a-zA-Z0-9_.]+)"\s*=>', seg[:end]))

# async dispatch：从 pub async fn dispatch 到函数尾
start = methods_rs.find("pub async fn dispatch")
end_a = methods_rs.find("\n}\n", start)
async_body = methods_rs[start:end_a]
async_ifs = set(re.findall(r'if method == "([a-zA-Z0-9_.]+)"', async_body))
async_match = set(re.findall(r'"([a-z][a-zA-Z0-9_.]+)"\s*=>', async_body))

def extract_list(name):
    mm = re.search(name + r"[^=]*= &\[(.*?)\];", methods_rs, re.S)
    return set(re.findall(r'"([^"]+)"', mm.group(1))) if mm else set()

rust_first = extract_list("static RUST_FIRST_METHODS")
py_first = extract_list("static PY_FIRST_METHODS")
async_only = extract_list("ASYNC_ONLY_METHODS")
py_prefixes_raw = re.search(r"static PY_ENGINE_PREFIXES: &\[&str\] = &\[(.*?)\];", methods_rs, re.S)
py_prefixes = [p.rstrip('.').rstrip('"') for p in re.findall(r'"([^"]*)"', py_prefixes_raw.group(1))] if py_prefixes_raw else []

# 顶部 memory.import / artifacts 等前置 match（在 dispatch 开头）
pre_match = set(re.findall(r'"(memory\.import\.[a-z]+|artifacts\.list)"\s*=>', methods_rs))

implemented_sync_or_async = sync_arms | async_ifs | async_match | pre_match

print(f"声明 {len(declared)} | sync {len(sync_arms)} | async-if {len(async_ifs)} | async-match {len(async_match)}")

# A. 声明了但完全无分派路径 → NOT_IMPLEMENTED（除非命中 PY 前缀转发）
def py_forwardable(x):
    return any(x.startswith(p) for p in py_prefixes)

gaps = sorted(d for d in declared if d not in implemented_sync_or_async and not py_forwardable(d))
print(f"\n[A] 声明但无任何 Rust 分派且不可 Python 转发（真死声明）: {len(gaps)}")
for x in gaps:
    tag = " [async-only]" if x in async_only else ""
    print(f"   - {x}{tag}")

# B. 未声明但有实现（探测不到）
undeclared = sorted(i for i in implemented_sync_or_async if i not in declared)
print(f"\n[B] 有实现未声明: {len(undeclared)}")
for x in undeclared: print(f"   - {x}")

# C. RUST_FIRST 声明但 dispatch_rust 无对应 arm 且无 async if（RUST_FIRST 白名单失效）
rust_first_broken = sorted(r for r in rust_first if r not in implemented_sync_or_async)
print(f"\n[C] RUST_FIRST_METHODS 列了但无实现分支: {len(rust_first_broken)}")
for x in rust_first_broken: print(f"   - {x}")

# D. 孤立 gateway 函数（零调用）
print("\n[D] 孤立函数复查:")
orphan_report = []
gw_dir = ROOT / "gateway"
for rs in gw_dir.glob("*.rs"):
    if rs.name in ("methods.rs", "protocol.rs", "server.rs", "_helpers.rs", "mod.rs"):
        continue
    text = rs.read_text(encoding="utf-8")
    mod_name = rs.stem
    for fn_m in re.finditer(r"pub(?:\(crate\))?\s+(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)", text):
        fname = fn_m.group(1)
        if "test" in fname:
            continue
        uses = len(re.findall(re.escape(mod_name) + r"::" + re.escape(fname) + r"\b", methods_rs))
        # 其他文件引用
        for f2 in ROOT.rglob("*.rs"):
            if f2.name == rs.name or f2.name == "methods.rs":
                continue
            t2 = f2.read_text(encoding="utf-8", errors="ignore")
            uses += len(re.findall(re.escape(mod_name) + r"::" + re.escape(fname) + r"\b", t2))
        if uses == 0:
            orphan_report.append(f"{mod_name}.rs::{fname}")
for x in orphan_report:
    print(f"   - {x}")
