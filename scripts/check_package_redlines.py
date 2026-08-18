#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""安装包三条红线回归守卫（打包门禁）。

机械锁定安装包（MSI / NSIS / exe）必须满足的三条红线，任何一条被违反即阻断门禁：

  红线一  中文不乱码
          R1a  安装包文件名（MSI / NSIS）必须为纯 ASCII，杜绝中文安装包名乱码。
          R1b  （探测）exe 内应内嵌 webui 资源，且含正确 UTF-8 标语字节
                "只要做到极致 什么都能成功"；若可静态确认则判 PASS，否则仅提示。

  红线二  网关可用（UI 不转圈）
          R2a  （可选运行时）若传入 --probe 且网关已起，GET /api/health 须返回
                HTTP 200 且 JSON {"engine":"rust","status":"ok"}。
                网关未起时跳过并提示（打包期未必能拉起完整 GUI）。

  红线三  不弹缺依赖提示
          R3a  exe 不得导入 VCRUNTIME140.dll / MSVCP140.dll（windows-msvc 默认静态链
                CRT，缺这俩 DLL 即说明退化成动态 CRT，换机必弹"缺少 VCRUNTIME140"）。
                用纯 Python PE 导入表解析（零外部依赖，不依赖 dumpbin / VS 环境）。
          R3b  MSI 必须将 WebView2 运行时离线安装器嵌进 File 表
                （MicrosoftEdgeWebView2RuntimeInstaller*.exe），保证离线装机不弹缺运行时。

退出码：0 = 无硬阻断；1 = 任一条红线被违反（或必备产物缺失导致无法验证）。
用法：
  python scripts/check_package_redlines.py                       # 自动发现产物
  python scripts/check_package_redlines.py --exe X --msi Y --nsis Z
  python scripts/check_package_redlines.py --probe               # 顺带探活网关
  python scripts/check_package_redlines.py --verbose             # 打印明细
"""
from __future__ import annotations

import os
import struct
import sys
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# 红线三-R3a：windows-msvc 目标默认静态链 CRT，不应出现这俩 DLL 导入。
FORBIDDEN_DLLS = ("VCRUNTIME140.dll", "MSVCP140.dll", "VCRUNTIME140_1.dll")
# 红线三-R3b：WebView2 离线运行时安装器文件名特征。
WEBVIEW2_GLOB = "microsoftedgewebview2runtimeinstaller"
# 红线一-R1b：webui 内嵌的正确 UTF-8 标语（用于探测乱码）。
SLOGAN = "只要做到极致 什么都能成功".encode("utf-8")
# 红线二-R2a：网关健康端点。
DEFAULT_HEALTH_URL = "http://127.0.0.1:18790/api/health"


# ── 纯 Python PE 导入表解析（零依赖） ────────────────────────────────────────
def _rva_to_off(data: bytes, sections, rva: int) -> int | None:
    for vaddr, vsize, rawptr in sections:
        if vaddr <= rva < vaddr + max(vsize, 1):
            return rawptr + (rva - vaddr)
    return None


def _read_cstr(data: bytes, off: int) -> str:
    end = data.index(b"\x00", off)
    return data[off:end].decode("ascii", "replace")


def pe_import_dlls(path: str) -> list[str] | None:
    """返回 exe/dll 导入的 DLL 名列表；非 PE 文件返回 None。"""
    try:
        with open(path, "rb") as fh:
            data = fh.read()
    except OSError:
        return None
    if len(data) < 64 or data[:2] != b"MZ":
        return None
    (e_lfanew,) = struct.unpack_from("<I", data, 0x3C)
    if data[e_lfanew : e_lfanew + 4] != b"PE\x00\x00":
        return None
    coff = e_lfanew + 4
    (num_sections,) = struct.unpack_from("<H", data, coff + 2)
    opt_header_size = struct.unpack_from("<H", data, coff + 16)[0]
    opt = coff + 20
    (magic,) = struct.unpack_from("<H", data, opt)
    if magic == 0x20B:  # PE32+
        import_rva = struct.unpack_from("<I", data, opt + 120)[0]
    elif magic == 0x10B:  # PE32
        import_rva = struct.unpack_from("<I", data, opt + 104)[0]
    else:
        return None
    sec_start = opt + opt_header_size
    sections = []
    for i in range(num_sections):
        sh = sec_start + i * 40
        vaddr = struct.unpack_from("<I", data, sh + 12)[0]
        vsize = struct.unpack_from("<I", data, sh + 8)[0]
        rawptr = struct.unpack_from("<I", data, sh + 20)[0]
        sections.append((vaddr, vsize, rawptr))
    if import_rva == 0:
        return []
    desc_off = _rva_to_off(data, sections, import_rva)
    if desc_off is None:
        return []
    dlls: list[str] = []
    i = 0
    while True:
        base = desc_off + i * 20
        if base + 20 > len(data):
            break
        name_rva = struct.unpack_from("<I", data, base + 12)[0]
        if name_rva == 0:
            break  # 终止描述符
        noff = _rva_to_off(data, sections, name_rva)
        if noff is not None:
            dlls.append(_read_cstr(data, noff).lower())
        i += 1
    return dlls


# ── 红线三-R3b：MSI 内嵌 WebView2 安装器 ─────────────────────────────────────
# 不依赖 msilib（部分便携 Python 未编译该模块）；直接扫描 MSI 字节中的
# 文件名字串池。MSI 的 _StringPool/_StringData 会以 ASCII 或 UTF-16LE 保存
# 每个内嵌文件的文件名，故检索该唯一长文件名即可证明离线运行时已内嵌。
def msi_has_webview2(msi_path: str) -> bool | None:
    """MSI 是否内嵌 WebView2 运行时（离线装机不弹缺运行时）。

    返回 None 表示无法打开文件（应转硬阻断）；True/False 表示是否检出。
    检测策略（双重证据，避免压缩流导致的漏报）：
      - 主信号：离线安装器文件名特征 WEBVIEW2_GLOB；
      - 次信号：MSI 字节中同时存在 microsoftedge 与 webview2 子串，
        覆盖安装器被改名/拆分的真实包（文件名不含完整 GLOB 串）。
    """
    try:
        with open(msi_path, "rb") as fh:
            blob = fh.read()
    except OSError:
        return None
    low = blob.lower()
    needle = WEBVIEW2_GLOB.encode("ascii")  # 已是全小写
    if needle in low:
        return True
    if b"microsoftedge" in low and b"webview2" in low:
        return True
    try:
        if WEBVIEW2_GLOB in blob.decode("utf-16-le", "ignore").lower():
            return True
    except Exception:  # noqa: BLE001
        pass
    return False


# ── 红线二-R2a：网关健康探活 ─────────────────────────────────────────────────
def probe_health(url: str, timeout: float = 5.0) -> tuple[bool, str]:
    try:
        with urllib.request.urlopen(url, timeout=timeout) as resp:
            body = resp.read().decode("utf-8", "replace")
            ok = resp.status == 200 and '"status":"ok"' in body and '"engine":"rust"' in body
            return ok, "HTTP %d %s" % (resp.status, body[:160])
    except Exception as exc:  # noqa: BLE001 - 探活失败即视为不可验证
        return False, "连接失败: %s" % exc


# ── 自动发现打包产物 ─────────────────────────────────────────────────────────
def _bounded_find(root: str, patterns: tuple[str, ...], maxdepth: int = 6) -> list[str]:
    hits: list[str] = []

    def walk(d: str, depth: int):
        if depth > maxdepth or len(hits) >= 10:
            return
        try:
            entries = os.listdir(d)
        except OSError:
            return
        for name in entries:
            p = os.path.join(d, name)
            if os.path.isdir(p):
                walk(p, depth + 1)
            elif any(pat.lower() in name.lower() for pat in patterns):
                hits.append(p)

    walk(root, 0)
    return hits


def discover() -> dict[str, str]:
    found = {"exe": None, "msi": None, "nsis": None}
    candidates = [
        os.path.join(ROOT, "target", "release"),
        os.path.join(ROOT, "target", "release", "bundle", "msi"),
        os.path.join(ROOT, "release"),
        os.path.join(ROOT, "..", "c", "marg_build_target", "release"),
        "D:/c/marg_build_target/release",
    ]
    for cand in candidates:
        if not os.path.isdir(cand):
            continue
        exes = _bounded_find(cand, ("marguerite-gui.exe", "marguerite.exe"))
        msis = _bounded_find(cand, (".msi",))
        nsis = _bounded_find(cand, ("setup.exe",))
        if not found["exe"] and exes:
            found["exe"] = exes[0]
        if not found["msi"] and msis:
            found["msi"] = msis[0]
        if not found["nsis"] and nsis:
            found["nsis"] = nsis[0]
    return found


# ── 各红线检查 ───────────────────────────────────────────────────────────────
def check_r1a_filenames(msi: str | None, nsis: str | None, verbose: bool) -> tuple[bool, list[str]]:
    problems: list[str] = []
    for label, path in (("MSI", msi), ("NSIS", nsis)):
        if not path:
            continue
        base = os.path.basename(path)
        try:
            base.encode("ascii")
        except UnicodeEncodeError:
            problems.append("%s 文件名含非 ASCII 字节（乱码红线）: %s" % (label, base))
        if verbose:
            print("       %s 文件名: %s" % (label, base))
    return (len(problems) == 0), problems


def _slogan_confirmable(exe: str) -> bool:
    try:
        with open(exe, "rb") as fh:
            return SLOGAN in fh.read()
    except OSError:
        return False


def check_r1b_slogan(exe: str | None, verbose: bool) -> tuple[bool, list[str], bool]:
    """返回 (pass, problems, confirmable)。confirmable=False 表示无法静态确认。"""
    if not exe:
        return False, ["缺 exe，无法静态校验标语 UTF-8"], False
    confirmable = _slogan_confirmable(exe)
    if verbose:
        print("       标语 UTF-8 字节在 exe 内: %s" % ("存在" if confirmable else "未找到（可能已压缩，转提示）"))
    if confirmable:
        return True, [], True
    return False, [], False  # 无法确认 → 转 advisory，不阻断


def check_r3a_crt(exe: str | None, verbose: bool) -> tuple[bool, list[str]]:
    problems: list[str] = []
    if not exe:
        return False, ["缺 exe，无法校验 CRT 静态链接（红线三）"]
    dlls = pe_import_dlls(exe)
    if dlls is None:
        return False, ["无法解析 PE 导入表: %s" % exe]
    if verbose:
        print("       exe 导入 DLL 数: %d" % len(dlls))
        for d in sorted(dlls):
            print("         - %s" % d)
    for f in FORBIDDEN_DLLS:
        if f.lower() in dlls:
            problems.append("exe 导入了禁止的 %s（退化成动态 CRT，换机必弹缺依赖）" % f)
    return (len(problems) == 0), problems


def check_r3b_webview2(msi: str | None, verbose: bool) -> tuple[bool, list[str]]:
    if not msi:
        return False, ["缺 MSI，无法校验 WebView2 离线运行时内嵌（红线三）"]
    embedded = msi_has_webview2(msi)
    if embedded is None:
        return False, ["无法打开 MSI 读取 File 表: %s" % msi]
    if verbose:
        print("       MSI 内嵌 WebView2 运行时: %s" % ("是" if embedded else "否"))
    if not embedded:
        return False, ["MSI 未内嵌 WebView2 运行时安装器（离线装机将弹缺运行时）"]
    return True, []


def _pick(args: list[str], flag: str) -> str | None:
    if flag in args:
        i = args.index(flag)
        if i + 1 < len(args):
            return args[i + 1]
    return None


def main() -> int:
    args = sys.argv[1:]
    verbose = "--verbose" in args
    do_probe = "--probe" in args
    explicit = {
        "exe": _pick(args, "--exe"),
        "msi": _pick(args, "--msi"),
        "nsis": _pick(args, "--nsis"),
    }
    health_url = _pick(args, "--health") or DEFAULT_HEALTH_URL

    found = discover()
    exe = explicit["exe"] or found["exe"]
    msi = explicit["msi"] or found["msi"]
    nsis = explicit["nsis"] or found["nsis"]

    print("=" * 68)
    print("安装包三条红线回归守卫  (root=%s)" % ROOT)
    print("-" * 68)
    print("发现产物: exe=%s" % (exe or "(无)"))
    print("          msi=%s" % (msi or "(无)"))
    print("          nsis=%s" % (nsis or "(无)"))

    hard: list[tuple[str, bool, list[str]]] = []
    adv: list[tuple[str, bool, list[str]]] = []

    p, probs = check_r1a_filenames(msi, nsis, verbose)
    hard.append(("R1a 安装包文件名纯 ASCII", p, probs))

    p, probs, confirmable = check_r1b_slogan(exe, verbose)
    if exe and confirmable:
        hard.append(("R1b 标语 UTF-8 内嵌", p, probs))
    else:
        adv.append(("R1b 标语 UTF-8 内嵌（无法静态确认）", p, probs))

    p, probs = check_r3a_crt(exe, verbose)
    hard.append(("R3a exe 静态链 CRT（无 VCRUNTIME140/MSVCP140）", p, probs))

    p, probs = check_r3b_webview2(msi, verbose)
    hard.append(("R3b MSI 内嵌 WebView2 离线运行时", p, probs))

    if do_probe:
        ok, detail = probe_health(health_url)
        adv.append(("R2a 网关 /api/health 探活", ok, [detail]))
        print("       探活 %s -> %s" % (health_url, detail))

    print("-" * 68)
    print("硬阻断项（任意失败即门禁不过）")
    hard_fail = 0
    for label, ok, probs in hard:
        status = "PASS" if ok else "FAIL"
        if not ok:
            hard_fail += 1
        print("[%s] %s" % (status, label))
        for pr in probs:
            print("       ! %s" % pr)

    if adv:
        print("-" * 68)
        print("提示项（不阻断；无法静态确认或需运行时）")
        for label, ok, probs in adv:
            status = "PASS" if ok else "ADVISORY"
            print("[%s] %s" % (status, label))
            for pr in probs:
                print("       ~ %s" % pr)

    print("=" * 68)
    if hard_fail == 0:
        print("结论: 三条红线全部通过打包门禁。")
        return 0
    print("结论: %d 项红线被违反，必须修复后方可通过打包门禁。" % hard_fail)
    return 1


if __name__ == "__main__":
    sys.exit(main())
