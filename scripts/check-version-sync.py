#!/usr/bin/env python3
"""版本号一致性门禁 —— 把所有前端/清单的版本钉到 core/Cargo.toml。

## 为什么需要它
`marguerite-core --version` 打印的是 `core/Cargo.toml` 的 `version`（经
`env!("CARGO_PKG_VERSION")`，见 `core/src/cli/program.rs` 的 `CLI_VERSION`）。
但仓库里另有若干处独立版本号，历史上长期漂移（实测 0.2.6 / 0.1.6 / 0.2.0 / 0.2.1 / 0.1.5）：

    core/Cargo.toml                    单一事实源
    tui/Cargo.toml
    gui/src-tauri/Cargo.toml
    gui/src-tauri/tauri.conf.json
    webui/package.json     + webui/package-lock.json（顶层包版本须一致）
    gui/package.json       + gui/package-lock.json（同上）
    README.md / README.zh-Hans.md      正文里的「引擎版本」声明（见下）

四端版本不一致会直接造成发布事故：安装包文件名、更新通道比对、Windows 产品版本资源
三者互相对不上。本脚本把「对账」固化成可重复的机械门禁。

## README 正文版本声明（P0-3）
此前本门禁**只**看前端/清单，不看 README 正文——于是 README 正文漂移到 `0.2.6`
而门禁 26/26 全绿也没抓到。现在把正文里的引擎版本声明一并纳入：命中不到即视为
「无法校验」（exit 2），避免模式失效后门禁退化成「假绿」。

## 用法
    python scripts/check-version-sync.py            # 门禁：不一致则 exit 1
    python scripts/check-version-sync.py --fix      # 统一到 core 的版本
    python scripts/check-version-sync.py --print    # 只打印对照表

## 退出码
    0 = 一致（或 --fix 后已一致）
    1 = 存在漂移
    2 = 单一事实源/目标文件缺失或解析失败
"""
import io
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CANON = "core/Cargo.toml"

# kind: "toml"  -> `version = "x"`（TOML 顶层键）
#       "json"  -> `"version": "x"`（JSON，取文件中出现的第 `nth` 处，1-based）
#       "json0" -> 同上但取第一处
TARGETS = [
    ("tui/Cargo.toml", "toml", None),
    ("gui/src-tauri/Cargo.toml", "toml", None),
    ("gui/src-tauri/tauri.conf.json", "json", 1),
    ("webui/package.json", "json", 1),
    ("gui/package.json", "json", 1),
    # npm lockfile 里顶层包自身的 version 在 files[0]，即文件中的第一处 "version":
    ("webui/package-lock.json", "json", 1),
    ("gui/package-lock.json", "json", 1),
]

# npm lockfile（lockfileVersion 3）里同一个版本号出现**两次**：
#   * 顶层 `"version"`
#   * `"packages": { "": { "version": ... } }` —— 包自身的条目
# 只改前者会留下后者漂移（实测过：webui 残留 0.2.1、gui 残留 0.1.5），
# 而 `npm ci` 会校验这份一致性。因此这两个文件额外用 JSON 解析做校验与修复。
NPM_LOCKFILES = [
    ("webui/package-lock.json", "webui/package.json"),
    ("gui/package-lock.json", "gui/package.json"),
]

# README 正文里的「引擎版本」声明（每条只含一个捕获组）。
# 注意：不能用宽泛的 `当前 \`x.y.z\`` —— 同一行上下文里 workbench 也写「当前 `1.1.0`」，
# 会误报。这里把模式锚定到各自的措辞/引擎名上，保证只命中引擎版本声明本身。
README_CLAIMS = [
    ("README.md", re.compile(r"`marguerite-core --version`[^\n]*?currently `(\d+\.\d+\.\d+)`"), "currently"),
    ("README.md", re.compile(r"engine's `(\d+\.\d+\.\d+)`"), "engine's"),
    ("README.zh-Hans.md", re.compile(r"`marguerite-core --version`[^\n]*?当前 `(\d+\.\d+\.\d+)`"), "当前"),
    ("README.zh-Hans.md", re.compile(r"引擎的 `(\d+\.\d+\.\d+)`"), "引擎的"),
]

RE_TOML = re.compile(r'(?m)^(version\s*=\s*")([^"]+)(")')
RE_JSON = re.compile(r'(?m)^(\s*"version":\s*")([^"]+)(")')


WEBSITE_FILES = ["website/index.html", "website/index.en.html"]


def read_canonical() -> str:
    path = os.path.join(ROOT, CANON)
    if not os.path.isfile(path):
        print(f"[check-version-sync] FAIL: 找不到单一事实源 {CANON}", file=sys.stderr)
        raise SystemExit(2)
    text = io.open(path, encoding="utf-8").read()
    m = RE_TOML.search(text)
    if not m:
        print(f"[check-version-sync] FAIL: {CANON} 里解析不到 version", file=sys.stderr)
        raise SystemExit(2)
    return m.group(2)


def locate(text: str, kind: str, nth):
    """返回 (当前版本字面量, 命中次数)；找不到返回 (None, 0)。"""
    rx = RE_TOML if kind == "toml" else RE_JSON
    hits = list(rx.finditer(text))
    if not hits:
        return None, 0
    idx = 0 if nth is None else min(nth - 1, len(hits) - 1)
    return hits[idx].group(2), len(hits)


def rewrite(text: str, kind: str, nth, new_ver: str):
    """只替换第 nth 处版本字面量；返回 (新文本, 替换次数)。"""
    rx = RE_TOML if kind == "toml" else RE_JSON
    hits = list(rx.finditer(text))
    if not hits:
        return text, 0
    idx = 0 if nth is None else min(nth - 1, len(hits) - 1)
    h = hits[idx]
    return text[: h.start(2)] + new_ver + text[h.end(2) :], 1


def rewrite_claim(text: str, rx, new_ver: str):
    """把 README 声明正则的捕获组（组 1）替换为 new_ver；返回 (新文本, 替换次数)。"""
    out, last, n = [], 0, 0
    for m in rx.finditer(text):
        out.append(text[last : m.start(1)])
        out.append(new_ver)
        last = m.end(1)
        n += 1
    out.append(text[last:])
    return "".join(out), n


def check_readme_claims():
    """返回 (rows, drifted, broken)，与主表同构；README 声明缺失即 broken（避免假绿）。"""
    rows, drifted, broken = [], [], []
    for rel, rx, label in README_CLAIMS:
        full = os.path.join(ROOT, rel)
        if not os.path.isfile(full):
            broken.append((rel, "文件不存在"))
            continue
        txt = io.open(full, encoding="utf-8").read()
        found = rx.findall(txt)
        if not found:
            broken.append((f"{rel} [{label}]", "解析不到引擎版本声明（模式失效或文案改写）"))
            continue
        for idx, ver in enumerate(found, 1):
            row_label = f"{rel} [{label}#{idx}]"
            rows.append((row_label, ver, 1))
            if ver != read_canonical():
                drifted.append(row_label)
    return rows, drifted, broken


def main() -> int:
    do_fix = "--fix" in sys.argv
    do_print = "--print" in sys.argv or do_fix

    canon = read_canonical()
    rows, drifted, broken = [], [], []

    for path, kind, nth in TARGETS:
        full = os.path.join(ROOT, path)
        if not os.path.isfile(full):
            broken.append((path, "文件不存在"))
            continue
        txt = io.open(full, encoding="utf-8").read()
        current, nhits = locate(txt, kind, nth)
        if current is None:
            broken.append((path, "解析不到 version"))
            continue
        rows.append((path, current, nhits))
        if current != canon:
            drifted.append(path)

    # npm lockfile 的「包自身」条目（packages[""].version）单独校验：
    # 正则只看得见第一处 "version"，漏掉了这一处（实测曾残留旧值）。
    for lock_rel, pkg_rel in NPM_LOCKFILES:
        full = os.path.join(ROOT, lock_rel)
        if not os.path.isfile(full):
            broken.append((lock_rel, "文件不存在"))
            continue
        try:
            data = json.loads(io.open(full, encoding="utf-8").read())
        except json.JSONDecodeError as exc:
            broken.append((lock_rel, f"JSON 解析失败: {exc}"))
            continue
        self_entry = data.get("packages", {}).get("", {})
        if "version" not in self_entry:
            broken.append((lock_rel, 'packages[""].version 缺失'))
            continue
        label = f'{lock_rel}  packages[""]'
        rows.append((label, self_entry["version"], 1))
        if self_entry["version"] != canon:
            drifted.append(label)

    # 官网版本位（2026-10-29 补）：`website/index.html` 有一批**硬编码兜底值**，
    # 运行时会被 GitHub release tag 覆盖，但"页面源码里写几"这件事没人管 ——
    # 当天真实事故：全仓已 0.2.19，页面还写着 0.2.18，而这条门禁当时完全不看 website/，
    # 于是所有闸门全绿、页面在撒谎。
    # 口径：所有带 `data-ver` 的版本展示位必须都是 canon（它们是 API 失败时的兜底）。
    for site_rel in WEBSITE_FILES:
        full = os.path.join(ROOT, site_rel)
        if not os.path.isfile(full):
            broken.append((site_rel, "文件不存在"))
            continue
        html = io.open(full, encoding="utf-8").read()
        found = re.findall(r'data-ver[^>]*?>([^<]*)<', html)
        # 只有属性、没有文本节点的（如 <b data-ver>Marguerite 0.2.19</b>）也要能抓到版本串
        found += [m for m in re.findall(r'data-ver[^>]*>([^<]{0,80})', html) if m not in found]
        vers = sorted({v for v in re.findall(r'\d+\.\d+\.\d+', " ".join(found))})
        if not vers:
            broken.append((site_rel, "解析不到 data-ver 版本位（标记被删了？）"))
            continue
        label = f"{site_rel} data-ver"
        rows.append((label, ",".join(vers), len(vers)))
        if vers != [canon]:
            drifted.append(label)

    # README 正文版本声明（P0-3）：同一把尺子对齐到 canon。
    readme_rows, readme_drifted, readme_broken = check_readme_claims()
    rows.extend(readme_rows)
    drifted.extend(readme_drifted)
    broken.extend(readme_broken)

    if do_print:
        print(f"单一事实源 {CANON} = {canon}")
        for path, current, _nhits in rows:
            flag = "ok   " if current == canon else "DRIFT"
            print(f"  {flag} {path:46} {current}")

    if broken:
        print("[check-version-sync] FAIL: 下列文件无法校验:", file=sys.stderr)
        for path, why in broken:
            print(f"  - {path}: {why}", file=sys.stderr)
        return 2

    if do_fix and drifted:
        fixed = 0
        for path, kind, nth in TARGETS:
            if path not in drifted:
                continue
            full = os.path.join(ROOT, path)
            txt = io.open(full, encoding="utf-8").read()
            new, n = rewrite(txt, kind, nth, canon)
            if n:
                io.open(full, "w", encoding="utf-8", newline="").write(new)
                fixed += 1
        # npm lockfile 的包自身条目：用 JSON 写回，保证 packages[""] 也被改到
        for lock_rel, _pkg_rel in NPM_LOCKFILES:
            if not any(d.startswith(lock_rel) for d in drifted):
                continue
            full = os.path.join(ROOT, lock_rel)
            raw = io.open(full, encoding="utf-8", newline="").read()
            data = json.loads(raw)
            data["version"] = canon
            if "" in data.get("packages", {}):
                data["packages"][""]["version"] = canon
            out = json.dumps(data, ensure_ascii=False, indent=2) + "\n"
            if "\r\n" in raw:
                out = out.replace("\n", "\r\n")
            io.open(full, "w", encoding="utf-8", newline="").write(out)
            fixed += 1
        # README 正文声明：按声明正则改写捕获组。
        for rel, rx, _label in README_CLAIMS:
            if not any(d.startswith(rel) for d in drifted):
                continue
            full = os.path.join(ROOT, rel)
            txt = io.open(full, encoding="utf-8", newline="").read()
            new, n = rewrite_claim(txt, rx, canon)
            if n:
                io.open(full, "w", encoding="utf-8", newline="").write(new)
                fixed += 1
        # 复验（含 lockfile 自身条目 + README 声明）
        still = []
        for path, kind, nth in TARGETS:
            txt = io.open(os.path.join(ROOT, path), encoding="utf-8").read()
            current, _ = locate(txt, kind, nth)
            if current != canon:
                still.append(path)
        for lock_rel, _pkg_rel in NPM_LOCKFILES:
            data = json.loads(io.open(os.path.join(ROOT, lock_rel), encoding="utf-8").read())
            if data.get("packages", {}).get("", {}).get("version") != canon:
                still.append(f'{lock_rel}  packages[""]')
        _rrows, rstill, rbroken = check_readme_claims()
        still.extend(rstill)
        if rbroken:
            still.extend(p for p, _ in rbroken)
        if still:
            print(f"[check-version-sync] FAIL: --fix 后仍漂移: {still}", file=sys.stderr)
            return 1
        print(f"[check-version-sync] OK — 已统一 {fixed} 个文件到 {canon}")
        return 0

    if drifted:
        print(
            f"[check-version-sync] FAIL: {len(drifted)} 处版本号与 {CANON} 的 {canon} 不一致:",
            file=sys.stderr,
        )
        for path in drifted:
            print(f"  - {path}", file=sys.stderr)
        print("  修复：python scripts/check-version-sync.py --fix", file=sys.stderr)
        return 1

    print(
        f"[check-version-sync] OK — {len(rows)} 处版本号与 {CANON} 一致（{canon}），"
        "含 2 个 npm lockfile 的包自身条目与 README 正文声明"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
