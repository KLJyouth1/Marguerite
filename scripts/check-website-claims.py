#!/usr/bin/env python3
"""官网/README 的硬事实对账闸门（纯标准库）。

为什么需要它：官网与 README 里的数字（代码行数、守卫道数、路由条数、测试基线……）
此前**没有任何机械核对**，而这个仓库已经吃过六次「宣传与代码漂移」的亏
（见 AGENTS.md §4：README 双份漂移、文档宣传不存在的门禁、死链指到仓库外）。
2026-10-28 首次全量对账当场查出六处漂移：内置工具写成 85（那是 capabilities 的
real 计数，注册表实际 74）、页脚又写 67、测试基线停在 4220/4816、
`core/src/provider` 这个目录**根本不存在**、代码行数台账落后一个冲刺。

口径（可复现，全部只数**随仓库发布**的那一份）：
  Rust     —— 提交集里 core/ tui/ gui/ 下的 *.rs
  前端     —— 提交集里 webui/src/ 下的 *.vue *.ts *.tsx *.js *.mjs
  Python   —— 提交集里 pyengine/src/ 下的 *.py
  技能文档 —— 提交集里 skills/ 下的 *.md
  工具模块 —— 提交集里 core/src/tools/ 顶层的 *.rs
  其余计数 —— 见各 measure_* 函数，每条都注明真实来源文件

接线：已接进 `.github/workflows/ci.yml` 的 `docs` job（第六项，纯标准库、无需 setup-python）。
**刻意不接进 `verify.sh --all`**：闸门总数（25）印在 README.md / README.zh-Hans.md / AGENTS.md / 官网四处，
并有 `check-readme-claims.py` 比对「文档写的总数 == verify.sh 真正调度的检查数」，
所以加一项要同步改四处；这条检查与 Rust/Node/Python 三个栈都无关，放 CI 更合适。

退出码：0 = 全部对得上；1 = 有漂移（逐条打印「宣称 / 实测」）；
2 = 无法验证（前置缺失、解析不出来）—— 绝不冒充通过（AGENTS.md §8）。
"""

from __future__ import annotations

import io
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def rel(*parts: str) -> str:
    return os.path.join(ROOT, *parts)


def read(path: str) -> str:
    return io.open(path, encoding="utf-8", errors="replace").read()


def commit_set() -> list[str]:
    """`git add -A` 会落地的集合（已跟踪 + 未跟踪但未被忽略），且磁盘上真实存在。"""
    proc = subprocess.run(
        ["git", "ls-files", "-c", "-o", "--exclude-standard", "-z"],
        cwd=ROOT,
        capture_output=True,
    )
    if proc.returncode != 0:
        raise RuntimeError("git ls-files 失败：" + proc.stderr.decode("utf-8", "replace").strip())
    out = []
    for raw in proc.stdout.split(b"\0"):
        if not raw:
            continue
        p = raw.decode("utf-8", "replace")
        if os.path.isfile(os.path.join(ROOT, p.replace("/", os.sep))):
            out.append(p)
    if not out:
        raise RuntimeError("提交集为空 —— 无法验证，按未通过处理")
    return out


def count_lines(files: list[str]) -> tuple[int, int]:
    total = 0
    for p in files:
        try:
            with io.open(os.path.join(ROOT, p.replace("/", os.sep)), "rb") as fh:
                total += sum(1 for _ in fh)
        except OSError:
            raise RuntimeError(f"读不到提交集中的文件：{p}")
    return len(files), total


def scope(files: list[str], prefix: str, exts: tuple[str, ...]) -> list[str]:
    return [f for f in files if f.startswith(prefix) and f.lower().endswith(exts)]


def measure(files: list[str]) -> dict[str, int]:
    m: dict[str, int] = {}

    rust = scope(files, "core/", (".rs",)) + scope(files, "tui/", (".rs",)) + scope(files, "gui/", (".rs",))
    n, l = count_lines(rust)
    m["rust_lines"], m["rust_files"] = l, n

    fe = scope(files, "webui/src/", (".vue", ".ts", ".tsx", ".js", ".mjs"))
    n, l = count_lines(fe)
    m["fe_lines"], m["fe_files"] = l, n

    py = scope(files, "pyengine/src/", (".py",))
    n, l = count_lines(py)
    m["py_lines"], m["py_files"] = l, n

    m["skills_md"] = len(scope(files, "skills/", (".md",)))
    m["tool_modules"] = len(
        [f for f in files if f.startswith("core/src/tools/") and f.count("/") == 3 and f.endswith(".rs")]
    )
    m["views"] = len([f for f in files if re.fullmatch(r"webui/src/views/[^/]+\.vue", f)])
    m["core_dirs"] = len({f.split("/")[2] for f in files if f.startswith("core/src/") and f.count("/") >= 3})
    m["locales"] = len([f for f in files if f.startswith("webui/src/locales/") and f.endswith(".json")])

    # 架构守卫：package.json 里 check:architecture 串起来的脚本数
    pkg = json.loads(read(rel("webui", "package.json")))
    script = pkg["scripts"].get("check:architecture", "")
    guards = re.findall(r"node scripts/([A-Za-z0-9._-]+\.mjs)", script)
    if not guards:
        raise RuntimeError("解析不出 check:architecture 的守卫清单")
    m["guards"] = len(guards)

    # i18n 硬编码中文棘轮基线（逐文件行数之和）
    base = json.loads(read(rel("webui", "scripts", "i18n-cjk-baseline.json")))
    vals = [v for v in base.values() if isinstance(v, int)] if isinstance(base, dict) else []
    if not vals:
        raise RuntimeError("i18n-cjk-baseline.json 解析不出行数列")
    m["cjk_lines"] = sum(vals)

    # CLI 路由：program.rs 的 route_table.register(...)
    program = read(rel("core", "src", "cli", "program.rs"))
    routes = len(re.findall(r"route_table\.register\(", program))
    if routes == 0:
        raise RuntimeError("解析不出 CLI 路由注册数")
    m["cli_routes"] = routes

    # RPC 声明：与 webui/scripts/check-rpc-contract.mjs 同一口径 —— 先剥行注释再取字面量，
    # 否则注释里出现的方法名会被当成声明（实测 451 → 假 454）。
    protocol = read(rel("core", "src", "gateway", "protocol.rs"))
    block = re.search(r"SUPPORTED_METHODS\s*:\s*&\[.*?\]\s*=\s*&\[(.*?)\];", protocol, re.S)
    if not block:
        raise RuntimeError("解析不出 SUPPORTED_METHODS 数组")
    body = re.sub(r"^\s*//.*$", "", block.group(1), flags=re.M)
    m["rpc_methods"] = len(set(re.findall(r'"([^"]+)"', body)))

    # 版本号同步处数：check-version-sync.py 自己报的那一行
    # 子进程必须显式要 UTF-8 —— 本机控制台是 cp936，中文输出按 utf-8 解码会得到乱码，
    # 于是正则永远匹配不上（这正是 AGENTS.md §4「别用终端显示下结论」那条坑的变体）。
    env = dict(os.environ, PYTHONUTF8="1", PYTHONIOENCODING="utf-8")
    vs = subprocess.run(
        [sys.executable, os.path.join("scripts", "check-version-sync.py"), "--print"],
        cwd=ROOT,
        capture_output=True,
        env=env,
    )
    mm = re.search(r"(\d+)\s*\u5904\u7248\u672c", vs.stdout.decode("utf-8", "replace"))
    if not mm:
        raise RuntimeError("check-version-sync.py --print 没报出同步处数（rc=%d）" % vs.returncode)
    m["version_sync"] = int(mm.group(1))

    # 全量门禁项数：verify.sh 的 --all 分支调度的检查数
    m["gate_checks"] = gate_checks()

    # 测试基线以 README 的质量闸门表为单一事实源（官网不得各自漂移）
    readme = read(rel("README.md"))
    mm = re.search(r"`cargo test -p marguerite-core --lib` \| ([\d,]+) passed / 0 failed", readme)
    if not mm:
        raise RuntimeError("README.md 的质量闸门表里找不到核心测试基线")
    m["core_tests"] = int(mm.group(1).replace(",", ""))
    mm = re.search(r"`webui` unit tests \(Vitest\) \| ([\d,]+) passed / 0 failed", readme)
    if not mm:
        raise RuntimeError("README.md 的质量闸门表里找不到控制台测试基线")
    m["webui_tests"] = int(mm.group(1).replace(",", ""))

    # 配乐拍点：beat-map.json 是唯一事实源
    beats = json.loads(read(rel("website", "beat-map.json")))
    m["bpm"] = int(round(beats["beatBpm"]))
    m["film_seconds"] = int(round(beats["duration"]))
    m["phase_ms"] = int(round(float(beats["beatPhase"]) * 1000))

    # 回环端口：`gateway::DEFAULT_PORT` 是唯一事实源（官网四处印 127.0.0.1:18790）
    gmod = read(rel("core", "src", "gateway", "mod.rs"))
    mm = re.search(r"pub const DEFAULT_PORT:\s*u16\s*=\s*([0-9_]+);", gmod)
    if not mm:
        raise RuntimeError("解析不出 gateway::DEFAULT_PORT")
    m["gateway_port"] = int(mm.group(1).replace("_", ""))

    return m


def gate_checks() -> int:
    """verify.sh 里 `MODE_ALL` 分支实际调度的 check_* 数。

    与 scripts/check-readme-claims.py 的 dispatched_checks() 同一套行级解析：
    `--all` 是 `if [ "$MODE_ALL" -eq 1 ]; then … fi`，不是 case 分支，
    拿 `--all)` 去匹配会解析出 0 项（AGENTS.md §8「无法验证 ≠ 通过」）。
    """
    lines = read(rel("scripts", "verify.sh")).splitlines()
    start = next(
        (i for i, ln in enumerate(lines) if ln.startswith('if [ "$MODE_ALL" -eq 1 ]')),
        None,
    )
    if start is None:
        raise RuntimeError("解析不出 verify.sh 的 MODE_ALL 分支")
    body = []
    for line in lines[start + 1 :]:
        if line.startswith("fi"):
            break
        body.append(line)
    calls = re.findall(r"^ {2}(check_\w+)[ \t]*$", "\n".join(body), re.M)
    if not calls:
        raise RuntimeError("MODE_ALL 分支里没找到任何检查调用")
    return len(calls)


def site_claims(site: str) -> dict[str, int | None]:
    """从官网文本里抽出被核对的数字；找不到某项返回 None（由调用方判失败）。"""
    out: dict[str, int | None] = {}
    plain = re.sub(r"<[^>]+>", " ", site)
    plain = re.sub(r"\s+", " ", plain)

    def num(pattern: str) -> int | None:
        m = re.search(pattern, plain)
        return None if m is None else int(m.group(1).replace(",", ""))

    out["rust_lines"] = num(r"([\d,]+) 行 Rust")
    out["rust_files"] = num(r"Rust · ([\d,]+) 个文件")
    out["fe_lines"] = num(r"([\d,]+) 行 前端")
    out["fe_files"] = num(r"前端 · ([\d,]+) 个文件")
    out["py_lines"] = num(r"([\d,]+) 行 Python")
    out["py_files"] = num(r"Python · ([\d,]+) 个文件")
    out["skills_md"] = num(r"([\d,]+) 份 技能文档")
    out["tool_modules"] = num(r"(\d+) 个 工具模块")
    out["views"] = num(r"(\d+) 控制台视图")
    out["core_dirs"] = num(r"(\d+) 内核子系统模块")
    out["rpc_methods"] = num(r"(\d+) RPC 方法声明")
    out["cjk_lines"] = num(r"当前 (\d+) 行")
    out["cli_routes"] = num(r"(\d+) 条路由")
    out["guards"] = num(r"架构守卫 (\d+) 道")
    out["locales"] = num(r"界面语言 (\d+) 种")
    out["version_sync"] = num(r"版本号 (\d+) 处同步")
    out["gate_checks"] = num(r"(\d+) 项发版门禁")
    out["core_tests"] = num(r"(\d+) 核心测试通过")
    out["webui_tests"] = num(r"(\d+) 控制台单测通过")
    out["bpm"] = num(r"(\d+) BPM")
    out["film_seconds"] = num(r"(\d+) 秒影片")
    out["gateway_port"] = num(r"127\.0\.0\.1:(\d+)")
    # 相位是秒小数（0.778s），单独放大成毫秒再比，避免 num() 的 int() 直接吃掉小数
    mm = re.search(r"相位 ([\d.]+)s", plain)
    out["phase_ms"] = None if mm is None else int(round(float(mm.group(1)) * 1000))
    return out


def main() -> int:
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except (OSError, ValueError):
        pass
    try:
        files = commit_set()
        measured = measure(files)
        site = read(rel("website", "index.html"))
    except (RuntimeError, OSError, KeyError, ValueError, json.JSONDecodeError) as exc:
        print("[FAIL] 无法验证（按未通过处理）：%s" % exc)
        return 2

    claims = site_claims(site)
    bad: list[str] = []
    for key, want in claims.items():
        have = measured.get(key)
        if want is None:
            bad.append(f"{key}: 官网里找不到这条数字（结构变了，检查器需要跟着改）")
        elif want != have:
            bad.append(f"{key}: 官网宣称 {want:,} · 实测 {have:,}")

    # 版本号：`core/Cargo.toml` 是单一事实源（AGENTS.md §2）。
    # 只比对「当前版本」会出现的固定位置 —— 开发史里那句「32 天里从 0.2.7 走到 0.2.17」
    # 是历史陈述，不是版本号声明，拿全站 semver 一起比会把它误判成漂移。
    cargo = read(rel("core", "Cargo.toml"))
    mm = re.search(r"^version\s*=\s*\"([^\"]+)\"", cargo, re.M)
    if not mm:
        print("[FAIL] 无法验证（按未通过处理）：core/Cargo.toml 里读不到 version")
        return 2
    canon = mm.group(1)
    current = re.findall(
        r"(?:版本 (?:<code>)?|Marguerite |MARGUERITE · |notice__tag\">|开发到 )(\d+\.\d+\.\d+)",
        site,
    )
    if not current:
        bad.append("version: 官网里找不到「当前版本」位置的版本号（结构变了，检查器要跟着改）")
    for v in set(current):
        if v != canon:
            bad.append(f"version: 官网宣称 {v} · core/Cargo.toml 是 {canon}")

    # 两份 README 的「架构守卫道数」也一起对账：它不在 check-readme-claims.py 的
    # 事实清单里（那条只比对中英两份互相对得上的数字），所以两份都写着 10，
    # 而 package.json 实际串了 13 道 —— 官网写对了、README 落后。
    for doc, pat in (
        ("README.md", r"(\d+) architecture guards green"),
        ("README.zh-Hans.md", r"(\d+) 项架构守卫全绿"),
    ):
        mm = re.search(pat, read(rel(doc)))
        if mm is None:
            bad.append(f"{doc}: 找不到「架构守卫全绿」这一行（表格改了，检查器要跟着改）")
        elif int(mm.group(1)) != measured["guards"]:
            bad.append(f"{doc}: 宣称 {mm.group(1)} 项架构守卫 · 实测 {measured['guards']} 道")

    # 两份 README、CI 注释与官网那句「本页 23 个代码台账数字」必须等于本脚本实际核对的条数。
    # 不钉这一条，「约 20」就会在加第 24 项宣称的那天变成新的漂移源 ——
    # 而这类漂移正是本脚本存在的理由。
    for doc, pat in (
        ("README.md", r"landing page's (\d+) printed numbers"),
        ("README.zh-Hans.md", r"官网印的 (\d+) 个数字"),
        (".github/workflows/ci.yml", r"prints (\d+) hard numbers"),
    ):
        mm = re.search(pat, read(rel(doc)))
        if mm is None:
            bad.append(f"{doc}: 找不到「官网印了几个数」这句话（表格改了，检查器要跟着改）")
        elif int(mm.group(1)) != len(claims):
            bad.append(f"{doc}: 宣称核对 {mm.group(1)} 项 · 本脚本实际核对 {len(claims)} 项")

    # 官网自己那句「本页 N 个代码台账数字」也钉住：台账外的数字必须写明来源，
    # 否则「每个数字都对应一条命令」这句话会在新增一节宣传时悄悄失真。
    mm = re.search(r"本页 (\d+) 个代码台账数字", site)
    if mm is None:
        bad.append("website/index.html: 找不到「本页 N 个代码台账数字」这句话（结构变了，检查器要跟着改）")
    elif int(mm.group(1)) != len(claims):
        bad.append(f"website/index.html: 宣称 {mm.group(1)} 个 · 本脚本实际核对 {len(claims)} 项")

    # 路径指针必须真实存在（core/src/provider 就是这么被抓出来的）。
    # 只判「纯路径」的 code 片段：命令行、glob、带参数的示例一律放过，否则全是假阳性。
    for token in sorted(set(re.findall(r"<code>([^<]+)</code>", site))):
        path = token.strip().rstrip("/")
        if not re.fullmatch(r"[A-Za-z0-9_.@/-]+", path):
            continue
        if not re.match(r"(core|webui|pyengine|gui|tui|scripts|docs|skills|workbench)/", path):
            continue
        if not os.path.exists(os.path.join(ROOT, path.replace("/", os.sep))):
            bad.append(f"死路径：官网引用了 {path}，仓库里没有")

    # 未开源期间，官网不得链向**源码仓库**。链向 Releases（安装包）或模型权重仓库是刻意的，
    # 所以判据只能取 `git remote get-url origin` 那条 —— 用「任何 github 链接」当判据
    # 会把 5 条正当的权重下载全报成缺陷（假阳性）。
    origin = subprocess.run(
        ["git", "config", "--get", "remote.origin.url"], cwd=ROOT, capture_output=True
    )
    origin_repo = origin.stdout.decode("utf-8", "replace").strip().removesuffix(".git")
    if not origin_repo:
        raise RuntimeError("读不到 remote.origin.url —— 无法判断哪条是源码仓库外链")
    slug = origin_repo.split("github.com/")[-1].removesuffix("/").lower()
    for href in re.findall(r'href="(https://github\.com/[^"]+)"', site):
        parts = href.removeprefix("https://github.com/").split("/")
        # 只比对 owner/repo 这两段，否则 Marguerite-Releases 会被 Marguerite 前缀误伤
        if "/".join(parts[:2]).lower() == slug:
            bad.append(f"源码未开放期间不应外链到源码仓库：{href}")

    if bad:
        print("[FAIL] 官网硬事实对账 %d 处漂移：" % len(bad))
        for line in bad:
            print("   - " + line)
        return 1

    print(
        "[PASS] 官网硬事实对账 —— Rust {rust_lines:,} 行/{rust_files} 文件 · "
        "前端 {fe_lines:,} 行/{fe_files} 文件 · Python {py_lines:,} 行/{py_files} 文件 · "
        "技能文档 {skills_md:,} · 工具模块 {tool_modules} · 视图 {views} · 内核模块 {core_dirs} · "
        "RPC {rpc_methods} · CLI 路由 {cli_routes} · 守卫 {guards} 道 · 语言 {locales} · "
        "版本同步 {version_sync} 处 · 门禁 {gate_checks} 项 · 基线 core {core_tests:,}/webui {webui_tests:,} · "
        "{bpm} BPM · CJK 棘轮 {cjk_lines} 行".format(**measured)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
