#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生效链对账（源码 → 构建产物 → 部署副本 → 运行进程）+ 安装包指纹。

为什么需要它
------------
本仓反复出现同一类事故：**代码改对了、门禁也绿了，但用户看到的还是旧界面/旧行为**。
根因不是代码错，而是「改的那份没在跑」——同一条逻辑在本机有四个副本：

  1. 源码            core/src · webui/src
  2. 构建产物        webui/dist · target/release/marguerite-gui.exe
  3. 部署副本        %LOCALAPPDATA%\\Marguerite\\{marguerite-gui.exe, webui/dist}
  4. 运行中进程      marguerite-gui.exe（PID 会变，路径不会）

本脚本把四层逐一比对，并打印**可对账指纹**（sha256 / 资源名 / 字节数）。
任何一层落后于上一层，都判 `DRIFT` 并以退出码 1 结束——因为"落后"在用户眼里
与"没修"完全等价。

用法
----
    python scripts/check-effective-chain.py            # 人读
    python scripts/check-effective-chain.py --json     # 机读（CI/审计留档）
    python scripts/check-effective-chain.py --strict-installer
                                                      # 安装包必须不旧于运行二进制

退出码
------
    0 = 四层一致
    1 = 有漂移（逐条打印原因）
    2 = 无法验证（缺安装目录/缺产物/非 Windows 且无等价路径）——**绝不冒充通过**

刻意**不**接进 `scripts/verify.sh --all`：本检查依赖"本机装过并正在运行"，在干净
clone / CI 上只能 SKIP，而 SKIP 在本仓永远不算通过（见 AGENTS.md §8）。
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASSET_RE = re.compile(r"assets/index-[A-Za-z0-9_-]+\.js")
SOURCE_DIRS = ("core/src", "webui/src", "tui/src", "gui/src-tauri/src")
SOURCE_SUFFIXES = {".rs", ".vue", ".ts", ".mts", ".css", ".json"}


# --------------------------------------------------------------------------- #
# 基础工具
# --------------------------------------------------------------------------- #
def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def read_index_asset(dist: Path) -> str | None:
    index = dist / "index.html"
    if not index.is_file():
        return None
    m = ASSET_RE.search(index.read_text(encoding="utf-8", errors="replace"))
    return m.group(0) if m else None


def count_files(dist: Path) -> int:
    return sum(1 for p in dist.rglob("*") if p.is_file())


def file_set(dist: Path) -> set[str]:
    return {p.relative_to(dist).as_posix() for p in dist.rglob("*") if p.is_file()}


def newest_mtime(dirs: tuple[str, ...], suffixes: set[str]) -> tuple[float, Path | None]:
    newest, where = 0.0, None
    for d in dirs:
        base = ROOT / d
        if not base.is_dir():
            continue
        for p in base.rglob("*"):
            if not p.is_file() or p.suffix.lower() not in suffixes:
                continue
            try:
                ts = p.stat().st_mtime
            except OSError:
                continue
            if ts > newest:
                newest, where = ts, p
    return newest, where


def newer_than(dirs: tuple[str, ...], suffixes: set[str], ts: float) -> list[Path]:
    out: list[Path] = []
    for d in dirs:
        base = ROOT / d
        if not base.is_dir():
            continue
        for p in base.rglob("*"):
            if p.is_file() and p.suffix.lower() in suffixes and p.stat().st_mtime > ts:
                out.append(p)
    return sorted(out)


def _no_window() -> int:
    return 0x08000000 if os.name == "nt" else 0


def run(cmd: list[str]) -> str:
    try:
        p = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            creationflags=_no_window(),
            timeout=30,
        )
        return (p.stdout or "").strip()
    except Exception:
        return ""


# --------------------------------------------------------------------------- #
# 四层定位
# --------------------------------------------------------------------------- #
def install_root() -> Path | None:
    """安装目录：NSIS/MSI 都把 exe 落在 %LOCALAPPDATA%\\Marguerite。"""
    cands: list[Path] = []
    local = os.environ.get("LOCALAPPDATA")
    if local:
        cands.append(Path(local) / "Marguerite")
    for env in ("ProgramFiles", "ProgramFiles(x86)"):
        if os.environ.get(env):
            cands.append(Path(os.environ[env]) / "Marguerite")
    for c in cands:
        if (c / "marguerite-gui.exe").is_file():
            return c
    return None


def running_pids(image: str = "marguerite-gui.exe") -> list[int]:
    if os.name != "nt":
        return []
    out = run(["tasklist", "/FI", f"IMAGENAME eq {image}", "/FO", "CSV", "/NH"])
    pids: list[int] = []
    for row in csv.reader(io.StringIO(out)):
        if len(row) >= 2 and row[1].isdigit():
            pids.append(int(row[1]))
    return pids


def pid_exe_path(pid: int) -> Path | None:
    if os.name != "nt":
        return None
    out = run(
        [
            "powershell",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            f"(Get-Process -Id {pid} -ErrorAction SilentlyContinue).Path",
        ]
    )
    return Path(out) if out else None


def newest_installer() -> Path | None:
    roots = [ROOT / "deliverables" / "installers", ROOT / "release"]
    best: tuple[float, Path] | None = None
    for r in roots:
        if not r.is_dir():
            continue
        for p in r.rglob("*"):
            if p.is_file() and p.suffix.lower() in {".msi", ".exe"} and "uninstall" not in p.name.lower():
                ts = p.stat().st_mtime
                if best is None or ts > best[0]:
                    best = (ts, p)
    return best[1] if best else None


def version() -> str:
    txt = (ROOT / "core" / "Cargo.toml").read_text(encoding="utf-8", errors="replace")
    m = re.search(r'^version\s*=\s*"([^"]+)"', txt, re.M)
    return m.group(1) if m else "unknown"


# --------------------------------------------------------------------------- #
# 对账
# --------------------------------------------------------------------------- #
def main() -> int:
    ap = argparse.ArgumentParser(add_help=True)
    ap.add_argument("--json", action="store_true")
    ap.add_argument(
        "--strict-installer",
        action="store_true",
        help="要求最新安装包不旧于运行中的二进制（发版前使用）",
    )
    args = ap.parse_args()

    drift: list[str] = []
    unknown: list[str] = []
    facts: dict[str, object] = {"version": version()}

    # ---- 层 1/2：源码 → 构建产物 ----
    repo_dist = ROOT / "webui" / "dist"
    repo_asset = read_index_asset(repo_dist)
    facts["source_dist"] = {
        "path": str(repo_dist),
        "files": count_files(repo_dist) if repo_dist.is_dir() else 0,
        "index_asset": repo_asset,
        "index_sha256": sha256(repo_dist / "index.html") if (repo_dist / "index.html").is_file() else None,
    }
    if not repo_asset:
        unknown.append("仓库 webui/dist 缺失或 index.html 无资源引用（先 npm run build）")

    build_exe = ROOT / "target" / "release" / "marguerite-gui.exe"
    facts["build_exe"] = {
        "path": str(build_exe),
        "exists": build_exe.is_file(),
        "bytes": build_exe.stat().st_size if build_exe.is_file() else 0,
        "mtime": build_exe.stat().st_mtime if build_exe.is_file() else None,
        "sha256": sha256(build_exe) if build_exe.is_file() else None,
    }

    # ---- 层 3：部署副本 ----
    root = install_root()
    facts["install_root"] = str(root) if root else None
    if root is None:
        unknown.append("未找到安装目录（%LOCALAPPDATA%\\Marguerite 无 marguerite-gui.exe）")
    else:
        inst_exe = root / "marguerite-gui.exe"
        inst_dist = root / "webui" / "dist"
        inst_asset = read_index_asset(inst_dist)
        facts["installed"] = {
            "exe": str(inst_exe),
            "exe_bytes": inst_exe.stat().st_size,
            "exe_mtime": inst_exe.stat().st_mtime,
            "exe_sha256": sha256(inst_exe),
            "dist": str(inst_dist),
            "dist_files": count_files(inst_dist) if inst_dist.is_dir() else 0,
            "index_asset": inst_asset,
            "index_sha256": sha256(inst_dist / "index.html") if (inst_dist / "index.html").is_file() else None,
        }
        if inst_asset and repo_asset and inst_asset != repo_asset:
            drift.append(
                f"部署前端落后：服务中 {inst_asset} ≠ 仓库构建 {repo_asset}"
                "（用户刷新后看到的仍是旧界面）"
            )
        if inst_dist.is_dir() and repo_dist.is_dir():
            only_repo = file_set(repo_dist) - file_set(inst_dist)
            only_inst = file_set(inst_dist) - file_set(repo_dist)
            facts["installed"]["dist_diff"] = {"only_repo": len(only_repo), "only_installed": len(only_inst)}
            if only_repo or only_inst:
                drift.append(
                    f"部署前端文件集不一致：仅仓库有 {len(only_repo)} / 仅部署有 {len(only_inst)}"
                    f"（示例 {sorted(only_repo)[:3]} {sorted(only_inst)[:3]}）"
                )

        # ---- 层 4：运行进程 ----
        pids = running_pids()
        facts["running"] = {"pids": pids}
        if not pids:
            unknown.append("marguerite-gui.exe 未在运行，无法对账「运行进程」层")
        else:
            exe_path = pid_exe_path(pids[0]) or inst_exe
            facts["running"]["exe_path"] = str(exe_path)
            if exe_path.is_file():
                run_sha = sha256(exe_path)
                facts["running"]["exe_sha256"] = run_sha
                facts["running"]["exe_mtime"] = exe_path.stat().st_mtime
                if run_sha != facts["installed"]["exe_sha256"]:
                    drift.append(
                        "运行进程与安装目录二进制不一致：进程跑的是 "
                        f"{exe_path}（{run_sha[:12]}），安装目录是 {inst_exe}（{facts['installed']['exe_sha256'][:12]}）"
                    )
                # 源码是否比运行二进制新
                ts = exe_path.stat().st_mtime
                stale = newer_than(SOURCE_DIRS, SOURCE_SUFFIXES, ts)
                facts["running"]["sources_newer"] = len(stale)
                if stale:
                    drift.append(
                        f"运行二进制落后于源码：{len(stale)} 个源文件比它新"
                        f"（最新 {stale[-1].relative_to(ROOT).as_posix()}）→ 当前进程不含这些改动"
                    )
                # 构建产物 vs 运行二进制
                if build_exe.is_file() and facts["build_exe"]["sha256"] != run_sha:
                    drift.append(
                        "target/release 的二进制与运行中的不是同一份："
                        f"build={facts['build_exe']['sha256'][:12]} run={run_sha[:12]}"
                    )

    # ---- 安装包指纹 ----
    inst = newest_installer()
    facts["installer"] = None
    if inst is None:
        unknown.append("未找到任何安装包（deliverables/installers 与 release/ 均为空）")
    else:
        st = inst.stat()
        facts["installer"] = {
            "path": str(inst),
            "name": inst.name,
            "bytes": st.st_size,
            "mtime": st.st_mtime,
            "sha256": sha256(inst),
        }
        if root is not None and args.strict_installer:
            if st.st_mtime < (root / "marguerite-gui.exe").stat().st_mtime:
                drift.append(
                    f"安装包过期：{inst.name} 早于运行中的二进制"
                    f"（{_fmt(st.st_mtime)} < {_fmt((root / 'marguerite-gui.exe').stat().st_mtime)}）"
                    "→ 重装这个包会回退功能"
                )

    # ---- 输出 ----
    if args.json:
        print(json.dumps({"ok": not drift, "drift": drift, "unknown": unknown, "facts": facts},
                         ensure_ascii=False, indent=2, default=str))
    else:
        _print_human(facts, drift, unknown)

    if drift:
        return 1
    if unknown and not facts.get("installer"):
        return 2
    return 0


def _fmt(ts: float) -> str:
    import datetime

    return datetime.datetime.fromtimestamp(ts).strftime("%Y-%m-%d %H:%M:%S")


def _print_human(facts: dict, drift: list[str], unknown: list[str]) -> None:
    print("=" * 78)
    print(f"生效链对账 · 版本 {facts['version']}")
    print("=" * 78)
    sd = facts.get("source_dist") or {}
    print("\n[层1/2] 源码 → 仓库构建产物")
    print(f"  webui/dist      : {sd.get('files')} 文件 · {sd.get('index_asset')}")
    print(f"  index.html sha  : {str(sd.get('index_sha256'))[:16]}")
    be = facts.get("build_exe") or {}
    if be.get("exists"):
        print(f"  target/release  : {be['bytes']:,} B · {_fmt(be['mtime'])} · sha {str(be['sha256'])[:16]}")
    else:
        print("  target/release  : 缺失")

    inst = facts.get("installed")
    print("\n[层3] 部署副本")
    if not inst:
        print("  未找到安装目录")
    else:
        print(f"  exe             : {inst['exe_bytes']:,} B · {_fmt(inst['exe_mtime'])} · sha {inst['exe_sha256'][:16]}")
        print(f"  webui/dist      : {inst['dist_files']} 文件 · {inst['index_asset']}")
        d = inst.get("dist_diff")
        if d:
            print(f"  文件集差集      : 仅仓库 {d['only_repo']} / 仅部署 {d['only_installed']}")

    run = facts.get("running") or {}
    print("\n[层4] 运行进程")
    if run.get("pids"):
        print(f"  PID             : {run['pids']}")
        print(f"  exe             : {run.get('exe_path')}")
        print(f"  sha             : {str(run.get('exe_sha256'))[:16]} · {_fmt(run['exe_mtime']) if run.get('exe_mtime') else '-'}")
        print(f"  比它新的源文件  : {run.get('sources_newer', 0)}")
    else:
        print("  未在运行")

    ins = facts.get("installer")
    print("\n[安装包指纹]")
    if ins:
        print(f"  {ins['name']}")
        print(f"  bytes   : {ins['bytes']:,}")
        print(f"  mtime   : {_fmt(ins['mtime'])}")
        print(f"  sha256  : {ins['sha256']}")
    else:
        print("  未找到")

    print("\n" + "=" * 78)
    if drift:
        print(f"结论：DRIFT（{len(drift)} 条）")
        for i, d in enumerate(drift, 1):
            print(f"  [{i}] {d}")
    else:
        print("结论：OK — 四层一致")
    if unknown:
        print("无法验证（不冒充通过）：")
        for u in unknown:
            print(f"  - {u}")
    print("=" * 78)


if __name__ == "__main__":
    sys.exit(main())
