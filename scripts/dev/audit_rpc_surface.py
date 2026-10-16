#!/usr/bin/env python3
"""审计一：SUPPORTED_METHODS 声明 vs dispatch 实现差集 + 孤立 pub fn"""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2] / "core" / "src"

# 1. 提取 SUPPORTED_METHODS 声明清单
proto = (ROOT / "gateway" / "protocol.rs").read_text(encoding="utf-8")
m = re.search(r"pub const SUPPORTED_METHODS: &\[&str\] = &\[(.*?)\];", proto, re.S)
declared = set(re.findall(r'"([^"]+)"', m.group(1)))

# 2. 提取 dispatch_rust 的 match 分支（同步）
methods_rs = (ROOT / "gateway" / "methods.rs").read_text(encoding="utf-8")
# dispatch_rust 函数体
dm = re.search(r"fn dispatch_rust\(.*?\)-> Value \{(.*?)\n\}", methods_rs, re.S)
rust_sync_arms = set(re.findall(r'"([a-z][a-zA-Z0-9_.]*)"\s*=>', dm.group(1))) if dm else set()

# 3. 异步分派里的 if method == 分支 + match 分支
async_block = methods_rs[methods_rs.find("pub async fn dispatch"):methods_rs.find("/// Python 侧车代理调用")]
async_ifs = set(re.findall(r'if method == "([a-zA-Z0-9_.]+)"', async_block))
# async match arms（在 dispatch 函数体内的 "xxx" =>）
am = re.search(r"pub async fn dispatch\((.*?)\n\}\n\n/// ", methods_rs, re.S)
async_match_arms = set()
if am:
    body = am.group(1)
    # 只取 match method { ... } 内的顶层分支（简化：所有 "name" => 形式）
    for mm in re.finditer(r'"([a-z][a-zA-Z0-9_.]*)"\s*=>', body):
        async_match_arms.add(mm.group(1))

implemented = rust_sync_arms | async_ifs | async_match_arms

# 4. RUST_FIRST / PY_FIRST / ASYNC_ONLY 清单
def extract_list(name):
    mm = re.search(name + r": &\[&str\] = &\[(.*?)\];", methods_rs, re.S)
    return set(re.findall(r'"([^"]+)"', mm.group(1))) if mm else set()

rust_first = extract_list("static RUST_FIRST_METHODS")
py_first = extract_list("static PY_FIRST_METHODS")
py_prefixes = [p.rstrip('.') for p in extract_list("static PY_ENGINE_PREFIXES")]
async_only = extract_list("ASYNC_ONLY_METHODS")

print("=" * 70)
print(f"声明(SUPPORTED_METHODS): {len(declared)} | 同步match: {len(rust_sync_arms)} | 异步if: {len(async_ifs)} | 异步match: {len(async_match_arms)}")
print("=" * 70)

# A. 声明了但既不在任何实现路径，也不在 PY 前缀/ASYNC_ONLY → 空壳声明（调用必 NOT_IMPLEMENTED）
def covered_by_py(method):
    return any(method.startswith(p) or method == p for p in py_prefixes) and method not in implemented

shell = sorted(x for x in declared if x not in implemented and not covered_by_py(x))
print(f"\n[A] 声明但无 Rust 实现 且 非 Python 前缀/ASYNC_ONLY（调用必失败）: {len(shell)}")
for x in shell: print("   -", x)

# B. 实现了但未声明 → supportsMethod 探测不到（功能静默）
undeclared = sorted(x for x in implemented if x not in declared)
print(f"\n[B] 有实现但未在 SUPPORTED_METHODS 声明（UI 探测不到）: {len(undeclared)}")
for x in undeclared: print("   -", x)

# C. ASYNC_ONLY 但也在 dispatch_rust 同步实现 → 冗余/矛盾
conflict = sorted(async_only & rust_sync_arms)
print(f"\n[C] 同时在 ASYNC_ONLY 与 dispatch_rust 同步实现（矛盾）: {len(conflict)}")
for x in conflict: print("   -", x)

# D. gateway 各模块孤立 pub(crate)/pub fn —— 定义了但从未被 methods.rs 或其他模块引用
orphan_report = []
gw_dir = ROOT / "gateway"
for rs in gw_dir.glob("*.rs"):
    if rs.name in ("methods.rs", "protocol.rs", "server.rs", "_helpers.rs", "mod.rs"):
        continue
    text = rs.read_text(encoding="utf-8")
    mod_name = rs.stem
    for fn_m in re.finditer(r"pub(?:\((?:crate|super|in crate::gateway)\))?\s+(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)", text):
        fname = fn_m.group(1)
        if fname.startswith("test") or "_test" in fname:
            continue
        # 全仓引用计数（排除定义处）
        pat = re.compile(r"(?<!fn )" + re.escape(mod_name) + "::" + re.escape(fname) + r"\b|" + re.escape(fname) + r"\(")
        uses = 0
        for f2 in ROOT.rglob("*.rs"):
            t2 = f2.read_text(encoding="utf-8", errors="ignore")
            if f2.name == rs.name:
                # 同文件内：去掉定义行再数
                body_wo = "\n".join(l for l in t2.splitlines() if f"fn {fname}" not in l)
                uses += len(pat.findall(body_wo))
            else:
                uses += len(re.findall(re.escape(mod_name) + "::" + re.escape(fname) + r"\b", t2))
        if uses == 0:
            orphan_report.append(f"{mod_name}.rs::{fname}")

print(f"\n[D] 孤立函数（gateway 模块内定义但全仓零调用）: {len(orphan_report)}")
for x in orphan_report[:40]: print("   -", x)

# E. PY_ENGINE_PREFIXES 转发但侧车离线时必然 NOT_IMPLEMENTED 且无 Rust 回退的前缀方法（从声明里找）
py_only_declared = sorted(
    d for d in declared
    if any(d.startswith(p) for p in ["channels.", "router.dec", "meta.", "wizard.", "secrets.", "migration.", "uploads.", "diagnostics.", "logs.", "commands.", "bundle."])
    and d not in implemented
)
print(f"\n[E] 声明且仅依赖 Python 侧车的方法（侧车离线时不可用——已知降级）: {len(py_only_declared)}")
for x in py_only_declared[:20]: print("   -", x)
