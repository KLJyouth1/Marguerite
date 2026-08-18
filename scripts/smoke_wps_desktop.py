#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Marguerite 桌面自动化 · 本机冒烟测试（WPS 一周计划 PPT）
=========================================================

用途
----
直接打回环 WebSocket JSON-RPC（ws://127.0.0.1:18790/ws），绕过前端，
端到端验证 Epic C 桌面自动化接线是否真实可用：
  desktop.list_apps → desktop.launch(wps_ppt) → desktop.action(generate_presentation)
  → artifacts.list 返回 .pptx → 监听 artifact.updated / desktop.action_done 事件

⚠️ 关键前提
----------
1. 应用已在运行且网关监听 127.0.0.1:18790（cargo tauri dev 或已安装 .msi 启动）。
2. 本机已安装 WPS，且 WPS 的 COM 已注册：
   以管理员打开 CMD 执行一次  `wps /regserver`  （否则 CoCreateInstance 报 2505）。
3. Python 3.10+ 且已装 websockets：  pip install websockets
4. 本脚本验证的是「RPC 层」真实调用链路。注意：聊天里让智能体自动操控 WPS 的
   UX 路径**后端 RPC 全通、但 LLM 侧没有把 desktop.launch 暴露成工具**（全仓无
   tool.desktop_control 之外的桌面工具注册），所以「对话触发」目前走不通，只能直接打 RPC。
   这正是本冒烟脚本存在的意义——用 RPC 直连证明引擎可用。

运行
----
  python scripts/smoke_wps_desktop.py
"""

import asyncio
import json
import os
import sys
import tempfile
import uuid

try:
    import websockets
except ImportError:
    sys.exit("请先安装依赖: pip install websockets")

PORT = 18790
WS_URL = f"ws://127.0.0.1:{PORT}/ws"

# 收集服务端主动推送的事件（artifact.updated / desktop.action_done / tick ...）
RECEIVED_EVENTS: list[dict] = []


def load_token() -> str:
    """token 落盘于 Config::data_dir()/gateway-token。
    Windows → %LOCALAPPDATA%/Marguerite/gateway-token
    其它   → ~/.local/share/Marguerite/gateway-token
    """
    base = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~/.local/share")
    path = os.path.join(base, "Marguerite", "gateway-token")
    if not os.path.isfile(path):
        sys.exit(
            f"找不到网关 token 文件: {path}\n"
            "请先启动 Marguerite（桌面应用 / cargo tauri dev），token 会在首次启动时生成。"
        )
    with open(path, encoding="utf-8") as f:
        return f.read().strip()


async def rpc(ws, method: str, params: dict, req_id: str) -> dict:
    """发送一个 req 并等到对应 id 的 res 返回；期间捕获的事件收入 RECEIVED_EVENTS。"""
    await ws.send(json.dumps({"type": "req", "id": req_id, "method": method, "params": params}))
    while True:
        raw = await ws.recv()
        msg = json.loads(raw)
        if msg.get("type") == "event":
            RECEIVED_EVENTS.append(msg)
            continue
        if msg.get("type") == "res" and msg.get("id") == req_id:
            return msg
        # 其它 res（心跳 pong 等）忽略
        print(f"  [忽略非预期帧] {msg}")


def check(cond: bool, label: str):
    mark = "✅ PASS" if cond else "❌ FAIL"
    print(f"{mark}  {label}")
    return cond


async def main():
    token = load_token()
    out_dir = os.path.join(tempfile.gettempdir(), "marguerite-smoke")
    os.makedirs(out_dir, exist_ok=True)
    out_path = os.path.join(out_dir, "weekly-plan.pptx")

    print(f"→ 连接 {WS_URL}")
    async with websockets.connect(WS_URL) as ws:
        # 1) 握手：接收 challenge，回 connect
        challenge = json.loads(await ws.recv())
        if challenge.get("event") != "connect.challenge":
            sys.exit(f"首帧不是 connect.challenge: {challenge}")
        await ws.send(json.dumps({
            "type": "req",
            "id": "1",
            "method": "connect",
            "params": {"minProtocol": 3, "maxProtocol": 3, "auth": {"token": token}},
        }))
        hello = json.loads(await ws.recv())
        if not (hello.get("type") == "res" and hello.get("ok") is True):
            sys.exit(f"握手失败（token 可能不匹配）: {hello}")
        print("✅ PASS  握手成功（protocol=%s）" % hello.get("protocol"))

        all_ok = True

        # 2) desktop.list_apps —— 确认 wps_ppt 已静态注册
        r = await rpc(ws, "desktop.list_apps", {}, "2")
        apps = (r.get("payload") or {}).get("data", {}).get("apps", []) if r.get("ok") else []
        # 回信封结构为 {code,message,data}；兼容直接 payload
        if not apps and isinstance(r.get("payload"), dict):
            apps = r["payload"].get("apps", [])
        wps_registered = any(a.get("kind") == "wps_ppt" for a in apps)
        all_ok &= check(wps_registered, f"desktop.list_apps 含 wps_ppt 注册（共 {len(apps)} 个应用）")

        # 3) desktop.launch —— 启动 WPS
        r = await rpc(ws, "desktop.launch", {"app_kind": "wps_ppt"}, "3")
        if not r.get("ok"):
            print(f"  launch 失败: {r}")
            all_ok &= check(False, "desktop.launch 返回 handle")
        else:
            pld = r.get("payload") or {}
            data = pld.get("data", pld)
            handle = data.get("handle") or pld.get("handle")
            all_ok &= check(bool(handle), f"desktop.launch 返回 handle={handle}")
            if not handle:
                sys.exit("无 handle，无法继续 action 测试")

        # 4) desktop.action generate_presentation —— 生成一周计划 PPT
        outline = {
            "slides": [
                {"title": "一周工作计划", "bullets": ["目标：本周交付 MVP 验收", " owner：我"]},
                {"title": "周一", "bullets": ["需求澄清", "技术选型"]},
                {"title": "周二", "bullets": ["前端框架搭建", "后端 API 骨架"]},
                {"title": "周三", "bullets": ["桌面自动化 COM 驱动", "产物台账"]},
                {"title": "周四", "bullets": ["Bloom 路由可视化", "联调"]},
                {"title": "周五", "bullets": ["集成冒烟", "交付"]},
            ]
        }
        action_params = {
            "handle": handle,
            "action": {
                "kind": "generate_presentation",
                "params": {"outline": outline, "output_path": out_path},
            },
            "approved": True,
            "session_key": "smoke-test",
        }
        r = await rpc(ws, "desktop.action", action_params, "4")
        if not r.get("ok"):
            print(f"  action 失败: {r}")
            all_ok &= check(False, "desktop.action(generate_presentation) 成功")
        else:
            all_ok &= check(True, "desktop.action(generate_presentation) 成功")
            # 验证文件确实写出
            all_ok &= check(
                os.path.isfile(out_path) and os.path.getsize(out_path) > 0,
                f"PPTX 已落盘: {out_path} ({os.path.getsize(out_path)} bytes)",
            )

        # 5) artifacts.list —— 确认 PPT 进入产物台账
        r = await rpc(ws, "artifacts.list", {"session_key": "smoke-test", "limit": 50}, "5")
        arts = []
        if r.get("ok"):
            pld = r.get("payload") or {}
            data = pld.get("data", pld)
            arts = data.get("artifacts", []) or pld.get("artifacts", [])
        found = [a for a in arts if str(a.get("path", "")).endswith(".pptx") or "pptx" in str(a.get("kind", "")).lower()]
        all_ok &= check(bool(found), f"artifacts.list 返回 .pptx 产物（命中 {len(found)} 个）")

        # 6) 事件验证 —— 运行期间应收到 artifact.updated / desktop.action_done
        ev_names = [e.get("event") for e in RECEIVED_EVENTS]
        all_ok &= check(
            "artifact.updated" in ev_names,
            f"收到 artifact.updated 事件（共捕获 {len(RECEIVED_EVENTS)} 个事件: {sorted(set(ev_names))}）",
        )
        all_ok &= check(
            "desktop.action_done" in ev_names,
            "收到 desktop.action_done 事件",
        )

    print("\n" + ("=" * 56))
    print("冒烟结论:", "✅ 全部通过 — Epic C 桌面自动化真实可用" if all_ok else "❌ 有失败项，见上")
    print("=" * 56)
    sys.exit(0 if all_ok else 1)


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("\n已中止")
    except ConnectionRefusedError:
        sys.exit(f"连接被拒：确认 Marguerite 网关已在 127.0.0.1:{PORT} 监听（应用是否已启动？）")
