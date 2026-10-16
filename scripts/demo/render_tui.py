#!/usr/bin/env python3
"""Render a real Marguerite TUI frame to the PNG used in the README.

Why this exists
---------------
`docs/assets/tui-workbench.png` must be reproducible and must show the *real*
workbench, not a mock-up. A terminal screenshot normally needs a pty, which Windows
does not give you -- but the workbench already knows how to render itself into
ratatui's `TestBackend` (that is how `core/src/tui/**` tests verify panels), so the
frame can be produced headlessly:

    cargo run -p marguerite-core --example tui_frame   ->  ANSI on stdout
    render_tui.py                                      ->  ANSI to PNG

Two details make the image faithful rather than approximate:

* **Column placement is computed, not entrusted to the font.** Every glyph is
  absolutely positioned at `column * cell_width`, with the column advancing by the
  glyph's East-Asian display width -- so a CJK glyph lands on its true column even
  if the font's advance is not exactly twice the Latin advance.
* **Every row is verified** against the terminal width the Rust side rendered, which
  cross-validates the two independent width tables (Rust `unicode-width` vs Python
  `unicodedata.east_asian_width`) against each other.

Prerequisites: a Rust toolchain, and `pip install playwright && playwright install chromium`.

Usage
-----
    python scripts/demo/render_tui.py                 # writes docs/assets/
    python scripts/demo/render_tui.py --no-run        # reuse the last capture
    python scripts/demo/render_tui.py --out /tmp/x.png

Exit codes: 0 = rendered; 1 = the example failed to run or a row width disagreed.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile
import unicodedata
from collections import Counter
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SGR = re.compile(r"\x1b\[([0-9;]*)m")

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")


def sniff(path: Path) -> str:
    """Decode a capture whose encoding depends on the shell that wrote it.

    PowerShell's `>` writes UTF-16LE with a BOM while a bash redirect writes UTF-8,
    so sniff instead of guessing (AGENTS.md section 4).
    """
    data = path.read_bytes()
    if data[:2] == b"\xff\xfe":
        return data.decode("utf-16-le")
    if data[:2] == b"\xfe\xff":
        return data.decode("utf-16-be")
    return data.decode("utf-8", errors="replace")


def cell_width(ch: str) -> int:
    if unicodedata.combining(ch):
        return 0
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


def apply_sgr(params: str, state: dict) -> None:
    """Apply one SGR parameter list; fg and bg may arrive combined in one sequence."""
    codes = [int(c) for c in params.split(";") if c != ""]
    i = 0
    while i < len(codes):
        c = codes[i]
        if c == 0:
            state.update(fg="reset", bg="reset", bold=False)
            i += 1
        elif c == 1:
            state["bold"] = True
            i += 1
        elif c == 22:
            state["bold"] = False
            i += 1
        elif c == 38 and codes[i + 1: i + 2] == [2]:
            r, g, b = codes[i + 2: i + 5]
            state["fg"] = f"#{r:02x}{g:02x}{b:02x}"
            i += 5
        elif c == 48 and codes[i + 1: i + 2] == [2]:
            r, g, b = codes[i + 2: i + 5]
            state["bg"] = f"#{r:02x}{g:02x}{b:02x}"
            i += 5
        elif c == 39:
            state["fg"] = "reset"
            i += 1
        elif c == 49:
            state["bg"] = "reset"
            i += 1
        else:
            i += 1


def parse(text: str):
    """Yield (row, column, char, fg, bg, bold) for every glyph in the frame."""
    state = {"fg": "reset", "bg": "reset", "bold": False}
    for row, raw in enumerate(text.splitlines()):
        col = 0
        pos = 0
        for m in SGR.finditer(raw):
            for ch in raw[pos: m.start()]:
                yield row, col, ch, state["fg"], state["bg"], state["bold"]
                col += max(cell_width(ch), 1)
            pos = m.end()
            apply_sgr(m.group(1), state)
        for ch in raw[pos:]:
            yield row, col, ch, state["fg"], state["bg"], state["bold"]
            col += max(cell_width(ch), 1)


def capture_frame(dest: Path) -> None:
    env = {k: v for k, v in os.environ.items() if k != "NO_COLOR"}
    with dest.open("wb") as fh:
        proc = subprocess.run(
            ["cargo", "run", "-q", "-p", "marguerite-core", "--example", "tui_frame"],
            cwd=REPO_ROOT, stdout=fh, stderr=subprocess.PIPE, env=env,
        )
    if proc.returncode != 0:
        print("[FAIL] `cargo run --example tui_frame` failed:", file=sys.stderr)
        print(proc.stderr.decode("utf-8", "replace")[-1500:], file=sys.stderr)
        raise SystemExit(1)


def luminance(colour: str) -> float:
    r, g, b = (int(colour[i: i + 2], 16) / 255 for i in (1, 3, 5))
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def build_html(glyphs, cols: int, rows: int, cell: float, line: float) -> str:
    """`reset` means "whatever the terminal's default is", so it needs a stand-in.

    The frame paints some cells explicitly and leaves the rest `Reset`. Mapping those
    to the most *frequent* explicit colour would paint body text in the dim border
    colour, so pick by luminance instead: the theme's light text colour for the
    foreground, its dominant background for the background.
    """
    fg_hist = Counter(g[3] for g in glyphs if g[3] != "reset")
    bg_hist = Counter(g[4] for g in glyphs if g[4] != "reset")
    lights = [c for c in fg_hist if luminance(c) > 0.5]
    default_fg = max(lights, key=lambda c: fg_hist[c]) if lights else "#d4d4d4"
    default_bg = bg_hist.most_common(1)[0][0] if bg_hist else "#101828"
    print(f"  reset stand-ins: fg={default_fg} bg={default_bg}")

    spans = []
    for row, col, ch, fg, bg, bold in glyphs:
        if ch == " " and bg == "reset":
            continue  # nothing to paint
        fg = default_fg if fg == "reset" else fg
        bg = default_bg if bg == "reset" else bg
        bg_css = "" if bg == default_bg else f"background:{bg};"
        weight = "700" if bold else "400"
        text = ch.replace("&", "&amp;").replace("<", "&lt;")
        spans.append(
            f'<i style="left:{col * cell:.2f}px;top:{row * line:.2f}px;'
            f'color:{fg};{bg_css}font-weight:{weight}">{text}</i>'
        )
    w, h = cols * cell, rows * line
    return f"""<!doctype html><meta charset="utf-8"><style>
  html,body{{margin:0;padding:0;background:{default_bg};}}
  #t{{position:relative;width:{w}px;height:{h}px;
      font-family:"Cascadia Mono","Consolas","DejaVu Sans Mono",monospace;
      font-size:14px;line-height:{line}px;white-space:pre;}}
  #t i{{position:absolute;font-style:normal;white-space:pre;}}
</style><div id="t">{''.join(spans)}</div>"""


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(REPO_ROOT / "docs" / "assets" / "tui-workbench.png"))
    ap.add_argument("--cols", type=int, default=120, help="must match the example's WIDTH")
    ap.add_argument("--rows", type=int, default=38, help="must match the example's HEIGHT")
    ap.add_argument("--cell", type=float, default=8.4)
    ap.add_argument("--line", type=float, default=18.0)
    ap.add_argument("--scale", type=float, default=2.0)
    ap.add_argument("--no-run", action="store_true", help="reuse the cached capture")
    args = ap.parse_args()

    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        print("[FAIL] needs playwright -- pip install playwright && playwright install chromium",
              file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory(prefix="mgt-tui-") as tmp:
        tmpdir = Path(tmp)
        raw = tmpdir / "frame.ansi"
        if not args.no_run:
            print("[1/2] rendering a real frame via the tui_frame example ...")
            capture_frame(raw)
        if not raw.exists():
            print("[FAIL] no capture available; re-run without --no-run", file=sys.stderr)
            return 1

        glyphs = list(parse(sniff(raw)))
        if not glyphs:
            print("[FAIL] the capture contained no glyphs", file=sys.stderr)
            return 1
        rows = max(g[0] for g in glyphs) + 1

        per_row: dict[int, int] = {}
        for row, col, ch, _f, _b, _bo in glyphs:
            per_row[row] = max(per_row.get(row, 0), col + max(cell_width(ch), 1))
        widths = sorted(set(per_row.values()))
        print(f"[2/2] parsed {rows} rows; widths {widths} (expected {args.cols})")
        if widths != [args.cols]:
            offenders = {r: w for r, w in per_row.items() if w != args.cols}
            print(f"[FAIL] row widths disagree with {args.cols}: {offenders}", file=sys.stderr)
            return 1
        print("      every row fills the terminal exactly -- Rust and Python width tables agree")

        html_path = tmpdir / "frame.html"
        html_path.write_text(build_html(glyphs, args.cols, rows, args.cell, args.line),
                             encoding="utf-8")
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        with sync_playwright() as p:
            browser = p.chromium.launch()
            page = browser.new_page(
                viewport={"width": int(args.cols * args.cell),
                          "height": int(rows * args.line)},
                device_scale_factor=args.scale,
            )
            page.goto(html_path.as_uri())
            page.wait_for_timeout(400)
            page.locator("#t").screenshot(path=str(out))
            browser.close()

    print(f"saved {out} ({out.stat().st_size / 1024:,.0f} KB)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
