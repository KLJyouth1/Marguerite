#!/usr/bin/env python3
"""把 `website/assets/**/*.png` 生成 WebP 旁路文件（**不删 PNG**），并产出社交分享卡片。

为什么要有这个脚本：
1. 官网首页 9 张 PNG 共 **6.0 MB**，其中 hero 的 `ui/chat.png` 单张 **1.16 MB**（2880×1800）
   且带 `loading="eager"` —— 它就是首屏 LCP，是全站单点收益最大的一处。
2. 这些图是"渐变 + 界面截图"的照片型内容，PNG 是最差的格式选择；WebP 通常省 60–80%。
3. PNG **保留**作为 fallback（`<picture>` 的 `<img>` 分支），老浏览器/截图引用不会断。

用法：
    C:/Python314/python.exe website/scripts/gen-webp.py            # 生成/覆盖 WebP + og 卡片
    C:/Python314/python.exe website/scripts/gen-webp.py --dry-run  # 只报数字

约束（有意为之）：
- 只写 `assets/**/*.webp` 与 `assets/og-card.jpg`，**不改动任何 .png**；
- 幂等：目标已存在且比源新则跳过（加 `--force` 强制重生成）；
- 尺寸不变（保持 2880×1800），因为 `<img>` 上写了 width/height，缩放交由 CSS。
"""

import argparse
import os
import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFont
except ImportError:  # pragma: no cover
    sys.exit("需要 Pillow：C:/Python314/python.exe -m pip install pillow")

HERE = Path(__file__).resolve().parent
SITE = HERE.parent
ASSETS = SITE / "assets"

# 只转 ≥ 300 KB 的图：小图（logo 256×256）转 WebP 的收益还不够多一种格式的维护成本。
MIN_BYTES = 300 * 1024
WEBP_QUALITY = 80

# 社交分享卡片：1200×630 是 Open Graph 的通用尺寸（微信/X/Slack 都会按它裁）。
OG_SIZE = (1200, 630)
OG_OUT = ASSETS / "og-card.jpg"


def human(n: int) -> str:
    return f"{n / 1048576:.2f} MB" if n >= 1048576 else f"{n / 1024:.0f} KB"


def find_font(size: int):
    """找一个能写中文的字体；找不到就退到 Pillow 默认位图字体（英文仍可读）。"""
    for cand in (
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyhbd.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
    ):
        if os.path.exists(cand):
            try:
                return ImageFont.truetype(cand, size)
            except OSError:
                continue
    return ImageFont.load_default()


def gen_webp(dry: bool, force: bool) -> tuple[int, int]:
    saved_before = saved_after = 0
    targets = sorted(p for p in ASSETS.rglob("*.png") if p.stat().st_size >= MIN_BYTES)
    for src in targets:
        out = src.with_suffix(".webp")
        before = src.stat().st_size
        saved_before += before
        if out.exists() and not force and out.stat().st_mtime >= src.stat().st_mtime:
            after = out.stat().st_size
            saved_after += after
            print(f"skip  {out.relative_to(SITE)}  (已是最新, {human(after)})")
            continue
        if dry:
            print(f"would {src.relative_to(SITE)}  {human(before)} → webp")
            saved_after += before
            continue
        with Image.open(src) as im:
            im = im.convert("RGB") if im.mode in ("RGBA", "P") else im
            # method=6：最慢但压缩率最高；官网图是一次性产物，值得多花几秒。
            im.save(out, "WEBP", quality=WEBP_QUALITY, method=6)
        after = out.stat().st_size
        saved_after += after
        print(f"webp  {out.relative_to(SITE)}  {human(before)} → {human(after)}  (-{100 - after * 100 // before}%)")
    return saved_before, saved_after


def gen_og_card(dry: bool, force: bool) -> None:
    src = ASSETS / "ui" / "chat.png"
    if not src.exists():
        print("warn  找不到 assets/ui/chat.png，跳过 og 卡片")
        return
    if OG_OUT.exists() and not force and OG_OUT.stat().st_mtime >= src.stat().st_mtime:
        print(f"skip  {OG_OUT.relative_to(SITE)}  (已是最新, {human(OG_OUT.stat().st_size)})")
        return
    if dry:
        print(f"would {OG_OUT.relative_to(SITE)}  {OG_SIZE[0]}x{OG_SIZE[1]}")
        return

    card = Image.new("RGB", OG_SIZE, (13, 13, 15))  # 与站点 theme-color #0d0d0f 一致
    # 右侧放真实界面截图。约束：**宽度不超过卡片的 58%**。
    # 第一版按"高度 92%"缩放，结果截图横跨到左半边、把标题压住了（缩略图里字全糊）——
    # 社交卡片的第一职责是"缩略图能读"，不是"截图放得大"。
    with Image.open(src) as shot:
        max_w = int(OG_SIZE[0] * 0.58)
        scale = max_w / shot.width
        shot = shot.convert("RGB").resize((max_w, int(shot.height * scale)), Image.LANCZOS)
        x = OG_SIZE[0] - shot.width - 28
        y = (OG_SIZE[1] - shot.height) // 2
        card.paste(shot, (x, y))
    # 接缝压一层从左到右的渐变（暗→透明），保证截图边缘不与正文抢视线。
    grad = Image.new("L", (OG_SIZE[0], 1))
    for i in range(OG_SIZE[0]):
        grad.putpixel((i, 0), max(0, 235 - int(i * 235 / (OG_SIZE[0] * 0.72))))
    shade = Image.new("RGB", OG_SIZE, (13, 13, 15))
    card = Image.composite(shade, card, grad.resize((OG_SIZE[0], OG_SIZE[1])))

    d = ImageDraw.Draw(card)
    accent = (94, 234, 212)          # 站点强调色（青）
    muted = (150, 150, 160)
    d.text((56, 150), "MARGUERITE", font=find_font(46), fill=accent)
    d.text((56, 216), "本地优先的桌面", font=find_font(52), fill=(240, 240, 245))
    d.text((56, 282), "AI Agent 工作台", font=find_font(52), fill=(240, 240, 245))
    d.text((56, 380), "Rust 核心 · 四入口 · 数据不出机", font=find_font(26), fill=muted)
    # 底栏用一条强调色短线代替口号，避免小字在缩略图里糊成一团。
    d.rectangle([56, 440, 136, 446], fill=accent)

    OG_OUT.parent.mkdir(parents=True, exist_ok=True)
    card.save(OG_OUT, "JPEG", quality=86, optimize=True)
    print(f"og    {OG_OUT.relative_to(SITE)}  {OG_SIZE[0]}x{OG_SIZE[1]}  {human(OG_OUT.stat().st_size)}")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--force", action="store_true")
    a = ap.parse_args()
    before, after = gen_webp(a.dry_run, a.force)
    gen_og_card(a.dry_run, a.force)
    if before:
        print(f"\n合计：{human(before)} → {human(after)}（省 {100 - after * 100 // before}%）")


if __name__ == "__main__":
    main()
