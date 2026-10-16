#!/usr/bin/env python3
"""重新生成 README 用的 Web 控制台演示截图。

## 为什么需要这个脚本
`docs/assets/console-*.png` 是 README 的演示资产。截图会随 UI 变更而过期，
而「手工打开浏览器截图」既不可复现、也没人愿意做。这个脚本把它变成一条命令。

## 前置
1. 网关在跑：`marguerite-core gateway`（默认 127.0.0.1:18790）
2. 模型可用。**不需要真实 API Key**——仓库内置了一个本地 mock 模型服务：
   ```bash
   python scripts/demo/mock_llm.py 18999          # 终端 A
   MARGUERITE_API_KEY=mock MARGUERITE_BASE_URL=http://127.0.0.1:18999/v1 \
     MARGUERITE_MODEL=mock-model marguerite-core gateway   # 终端 B
   ```
   （用真实模型也可以，只要网关能正常回话。）
3. `pip install playwright && playwright install chromium`

## 用法
```bash
python scripts/demo/capture_console.py            # 写 docs/assets/
python scripts/demo/capture_console.py --out /tmp/shots

# 双语 README：同一套图出两个语言版本
python scripts/demo/capture_console.py --locale en
python scripts/demo/capture_console.py --locale zh-Hans --suffix -zh-Hans
```

⚠️ `--locale` 只切换**控制台界面**语言（写入 `localStorage['marguerite-locale']`）。
引擎生成的内容（计划步骤、`# 计划：…` 标题）目前仍是中文，所以英文界面下计划卡会中英混排。

## 退出码
0 = 全部截图成功；1 = 有页面打不开或找不到输入框。
"""
import argparse
import json
import os
import sys
from pathlib import Path

BASE_URL = "http://127.0.0.1:18790"
VIEWPORT = {"width": 1440, "height": 900}
SCALE = 2  # 2x，Retina 清晰度
# 演示用输入：中文、有明确结构，能触发计划拆解
DEMO_PROMPT = "帮我把这个目录按文件类型整理，并给出可回滚的方案"

COMPOSER_SELECTORS = [
    "textarea",
    "[contenteditable='true']",
    "input[type='text']",
]
# 启动提示条/引导层，截图上不好看，能关就关掉
DISMISS_SELECTORS = ["text=Got it", "text=Dismiss", "text=知道了", "text=明白了"]


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def dismiss_overlays(page) -> None:
    for sel in DISMISS_SELECTORS:
        try:
            el = page.locator(sel).first
            if el.is_visible(timeout=600):
                el.click()
                page.wait_for_timeout(300)
        except Exception:
            pass


def find_composer(page):
    for sel in COMPOSER_SELECTORS:
        try:
            loc = page.locator(sel).first
            if loc.is_visible(timeout=2500):
                return loc, sel
        except Exception:
            continue
    return None, None


def settle(page, max_seconds: int = 90) -> int:
    """等流式回复稳定：正文长度连续 4 秒不变即认为结束。"""
    last, stable = 0, 0
    for _ in range(max_seconds):
        page.wait_for_timeout(1000)
        try:
            n = len(page.inner_text("body"))
        except Exception:
            break
        if n == last:
            stable += 1
            if stable >= 4:
                break
        else:
            stable, last = 0, n
    return last


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base-url", default=BASE_URL)
    ap.add_argument("--out", default=str(repo_root() / "docs" / "assets"))
    ap.add_argument("--prompt", default=DEMO_PROMPT)
    ap.add_argument(
        "--locale",
        default=None,
        help="控制台语言（en / zh-Hans / ja / fr / de / es）；不传则用控制台已持久化的设置",
    )
    ap.add_argument("--suffix", default="", help="输出文件名后缀，便于同时保存多语言版本")
    args = ap.parse_args()

    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        print("FAIL: 需要 playwright —— pip install playwright && playwright install chromium")
        return 1

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    saved, failed = [], []

    with sync_playwright() as p:
        browser = p.chromium.launch()
        ctx = browser.new_context(viewport=VIEWPORT, device_scale_factor=SCALE)
        page = ctx.new_page()
        if args.locale:
            # 必须在应用代码读取之前写入，否则会被已持久化的旧语言覆盖
            page.add_init_script(
                "try{localStorage.setItem('marguerite-locale',"
                f"{json.dumps(args.locale)}"
                ")}catch(e){}"
            )
        errors = []
        page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)

        def shoot(name: str) -> None:
            stem, dot, ext = name.rpartition(".")
            name = f"{stem}{args.suffix}.{ext}" if dot else f"{name}{args.suffix}"
            path = out / name
            page.screenshot(path=str(path))
            saved.append((name, path.stat().st_size))

        # 1) 首屏空态：新用户打开控制台看到的第一眼
        print("[1/3] first-run empty state ...")
        page.goto(f"{args.base_url}/control/", wait_until="networkidle", timeout=45000)
        page.wait_for_timeout(4000)
        dismiss_overlays(page)
        # 指针移开，避免悬停态 tooltip 入镜
        page.mouse.move(0, 0)
        page.wait_for_timeout(400)
        shoot("console-first-run.png")

        # 2) 输入框已填写
        print("[2/3] composer filled ...")
        composer, sel = find_composer(page)
        if composer is None:
            print("FAIL: 找不到输入框（textarea / contenteditable / text input 都不可见）")
            browser.close()
            return 1
        print(f"      composer via {sel!r}")
        composer.click()
        composer.fill(args.prompt)
        page.wait_for_timeout(500)
        page.mouse.move(0, 0)
        shoot("console-composer-filled.png")

        # 3) 真实对话：经网关 -> 引擎 -> 模型，渲染出计划卡与执行态
        print("[3/3] drive a real conversation and wait for it to settle ...")
        composer.press("Enter")
        n = settle(page)
        print(f"      body settled at {n} chars")
        page.mouse.move(0, 0)
        page.wait_for_timeout(500)
        shoot("console-conversation.png")

        browser.close()

    print("\n=== saved ===")
    for name, size in saved:
        print(f"  {name:32} {size/1024:>8,.0f} KB   -> {out}")
    for name in failed:
        print(f"  FAILED {name}")
    if errors:
        print("\n=== browser console errors ===")
        for e in errors[:8]:
            print("  -", e[:200])

    return 0 if saved else 1


if __name__ == "__main__":
    raise SystemExit(main())
