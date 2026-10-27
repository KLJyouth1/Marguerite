#!/usr/bin/env python3
"""gui 壳（Tauri 2）架构不变量守卫。

# 为什么需要它（2026-10-21 的真实事故）

那次审计做的第一件事是"生效链四层核查"，读到 `tauri.conf.json` 才发现：

    windows[0].url   = "http://localhost:18790/control/"   → 加载 webui（Vue）
    frontendDist     = "webui/dist"
    bundle.resources = [...]                                → 不含 gui/dist

也就是说 `gui/src`（React，22 个面板 / 5597 行 / 70 个测试）**从未进入安装包，
主窗口也从不加载它**。在此之前，整整一轮"面板审计"修的全是用户看不见的代码。

这个事实在仓库里没有任何自动化守卫，只靠 AGENTS.md 的一段注释提醒。
注释会被忽略，守卫不会。

# 本脚本断言的四条不变量

1. 主窗口 `url` 指向网关 `/control/`，或指向 `splash.html` 且 `lib.rs` 具备
   「就绪后 navigate 到 /control/」的完整链路（2026-10-23 起的真实启动流程）；
2. `frontendDist` 是 `webui/dist`（唯一前端产物来源）；
3. `bundle.resources` 只声明 webui 的资源，**不声明** `gui/dist`；
4. `beforeDevCommand` 不再启动一个已不存在的前端 dev server；
5. `gui/src` 目录不存在（前端已归档，见 docs/prd/2026-10-21-gui-react-retire.md）。

退出码：0 = 全部成立；1 = 有不变量被破坏（逐条打印原因）。
`--self-check`：用合成夹具验证判定函数本身（不依赖当前工作区），0 = 自检通过。
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONF = ROOT / "gui" / "src-tauri" / "tauri.conf.json"
LIB_RS = ROOT / "gui" / "src-tauri" / "src" / "lib.rs"

# 网关托管前端的位置。桌面端是"壳 + 网关托管"架构，不是"壳 + 本地 dist"。
GATEWAY_CONTROL_PATH = "/control/"
SPLASH_URL = "splash.html"


def check_main_window_url(url: str, lib_src: str) -> list[str]:
    """判定主窗口 URL 是否属于两种合法启动模式之一。

    纯函数：不读磁盘，`lib_src` 由调用方注入，因而可用合成夹具自检。
    """
    if GATEWAY_CONTROL_PATH in url:
        return []
    if url == SPLASH_URL:
        failures: list[str] = []
        if "fn control_url()" not in lib_src:
            failures.append("splash 启动模式要求 lib.rs 定义 control_url()")
        if "win.navigate(" not in lib_src:
            failures.append("splash 启动模式要求 lib.rs 在就绪后调用 win.navigate(...)")
        if GATEWAY_CONTROL_PATH not in lib_src:
            failures.append("lib.rs 的 control_url() 必须指向 /control/")
        if "MIN_SPLASH_MS" not in lib_src:
            failures.append("splash 启动模式要求声明 MIN_SPLASH_MS（首帧动画最短展示时长）")
        return failures
    if ":1420" in url or "localhost:1420" in url:
        return [
            f"主窗口 url 指向已废止的 Vite dev 端口 1420（{url!r}）；gui/src 已归档"
        ]
    return [
        f"主窗口 url 必须是 {GATEWAY_CONTROL_PATH!r} 或 {SPLASH_URL!r}，实际 {url!r}"
        "\n     → 桌面端是「壳 + 网关托管 webui」；改成加载本地 dist 会让运行时拿不到网关能力"
    ]


def main() -> int:
    failures: list[str] = []

    if not CONF.exists():
        print(f"[FAIL] 找不到 {CONF.relative_to(ROOT)}")
        return 1

    try:
        conf = json.loads(CONF.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        print(f"[FAIL] tauri.conf.json 不是合法 JSON：{exc}")
        return 1

    windows = conf.get("app", {}).get("windows", [])
    if not windows:
        failures.append("app.windows 为空 —— 桌面端没有主窗口")
    else:
        url = str(windows[0].get("url", ""))
        lib_src = LIB_RS.read_text(encoding="utf-8") if LIB_RS.exists() else ""
        if not LIB_RS.exists():
            failures.append(f"找不到 {LIB_RS.relative_to(ROOT)}（splash 模式无法校验导航链路）")
        failures.extend(check_main_window_url(url, lib_src))

    build = conf.get("build", {})
    # 3. 前端产物只有一个来源
    dist = str(build.get("frontendDist", ""))
    if dist.replace("\\", "/").rstrip("/") != "webui/dist":
        failures.append(
            f"build.frontendDist 应为 'webui/dist'，实际为 {dist!r}"
            "\n     → 前端只在 webui/（Vue）维护；指向别处等于又开一条前端产线"
        )

    # 4. beforeDevCommand 不得启动已不存在的前端
    before_dev = str(build.get("beforeDevCommand", "") or "")
    if "npm run dev" in before_dev and not (ROOT / "gui" / "src").exists():
        failures.append(
            "build.beforeDevCommand 仍在执行 'npm run dev'，但 gui/src 已归档 —— "
            "dev 模式会因缺少前端源码直接失败"
        )

    # 5. bundle 不得把这个不存在的前端产物列进去
    resources = conf.get("bundle", {}).get("resources", []) or []
    if isinstance(resources, dict):
        resource_keys = list(resources.keys())
    else:
        resource_keys = [str(r) for r in resources]
    for res in resource_keys:
        normalized = res.replace("\\", "/").rstrip("/")
        if normalized == "gui/dist":
            failures.append(
                "bundle.resources 声明了 'gui/dist' —— 该前端已归档，打包会失败或缺资源"
            )

    # 6. 前端源码不得重现
    if (ROOT / "gui" / "src").exists():
        failures.append(
            "gui/src 存在 —— 双前端已废止（见 docs/prd/2026-10-21-gui-react-retire.md）；"
            "\n     → 前端只在 webui/（Vue）维护，源码归档在 docs/legacy/gui-react-2026-10-21/"
        )

    print("=== gui 壳架构不变量 ===")
    print(f"  主窗口 url      : {windows[0].get('url') if windows else '(缺失)'}")
    print(f"  frontendDist    : {build.get('frontendDist')}")
    print(f"  bundle.resources: {len(resource_keys)} 项")
    print()

    if failures:
        for f in failures:
            print(f"[FAIL] {f}")
        print()
        print(f"未通过：{len(failures)} 条不变量被破坏。")
        return 1

    print("[PASS] 主窗口经网关加载 webui，frontendDist = webui/dist，gui/src 不存在。")
    print("闸门通过：单一前端架构成立。")
    return 0


def self_check() -> int:
    """用合成夹具验证判定函数（不读当前工作区，防止守卫与实现一起腐烂）。"""
    good_lib = """
fn control_url() -> String { format!("http://localhost:18790/control/") }
const MIN_SPLASH_MS: u64 = 6000;
fn go(win: &Window) { let _ = win.navigate(control_url()); }
"""
    cases: list[tuple[str, str, str, int]] = [
        ("控制台直连模式", "http://localhost:18790/control/", "", 0),
        ("splash 完整链路", SPLASH_URL, good_lib, 0),
        ("splash 缺 control_url", SPLASH_URL, good_lib.replace("fn control_url()", "fn other()"), 1),
        ("splash 缺 navigate", SPLASH_URL, good_lib.replace("win.navigate(", "win.doSomething("), 1),
        ("splash 缺 /control/", SPLASH_URL, good_lib.replace("/control/", "/app/"), 1),
        ("splash 缺 MIN_SPLASH_MS", SPLASH_URL, good_lib.replace("MIN_SPLASH_MS", "MIN_MS"), 1),
        ("本地 dist 模式", "index.html", good_lib, 1),
        ("Vite dev 端口", "http://localhost:1420", good_lib, 1),
    ]
    failed = 0
    for name, url, lib_src, expected in cases:
        got = len(check_main_window_url(url, lib_src))
        ok = (got == 0) if expected == 0 else (got > 0)
        print(f"[{'PASS' if ok else 'FAIL'}] self-check: {name}（预期{'通过' if expected == 0 else '拒绝'}，实际失败项 {got}）")
        if not ok:
            failed += 1
    if failed:
        print(f"\n自检失败：{failed}/{len(cases)} 条夹具不符合预期。")
        return 1
    print(f"\n自检通过：{len(cases)}/{len(cases)} 条夹具均符合预期。")
    return 0


if __name__ == "__main__":
    if "--self-check" in sys.argv[1:]:
        sys.exit(self_check())
    sys.exit(main())
