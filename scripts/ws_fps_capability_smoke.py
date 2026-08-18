"""端到端冒烟：验证 plans.capabilities 含宝塔/Computer Use/FPS，skills.list 含 fps。"""
import asyncio, json, os

URL = "ws://127.0.0.1:18790/ws"

async def call(ws, rid, method, params):
    await ws.send(json.dumps({"type": "req", "id": rid, "method": method, "params": params}))
    return json.loads(await asyncio.wait_for(ws.recv(), 10))

async def main():
    import websockets
    async with websockets.connect(URL) as ws:
        await ws.recv()  # connect.challenge
        hello = await call(ws, "c1", "connect", {"auth": {"guestSessionKey": "smoke-fps"}})
        assert hello.get("ok") is True, f"握手失败: {hello}"

        caps = await call(ws, "c2", "plans.capabilities", {})
        assert caps.get("ok") is True, f"plans.capabilities 失败: {caps}"
        payload = caps.get("payload", caps)
        items = payload.get("list", []) or []
        ids = [c.get("id") for c in items] if isinstance(items, list) else []
        for need in ["tool.baota", "tool.desktop_control", "tool.fps_assistant"]:
            assert need in ids, f"缺少能力 {need}; 现有: {ids}"
            print(f"OK  capability  {need}")

        skills = await call(ws, "c3", "skills.list", {})
        assert skills.get("ok") is True, f"skills.list 失败: {skills}"
        payload = skills.get("payload", skills)
        names = [s.get("name") for s in (payload.get("skills") or [])]
        assert "fps" in names, f"缺少 fps 技能; 现有: {names}"
        print("OK  skill       fps")

        tools = await call(ws, "c4", "tools.catalog", {})
        assert tools.get("ok") is True, f"tools.catalog 失败: {tools}"
        tp = tools.get("payload", tools)
        tool_names = [t.get("name") for t in (tp.get("items") or [])]
        assert "system_control" in tool_names, f"缺少 system_control 工具"
        assert any(n.startswith("baota.") for n in tool_names), f"缺少 baota.* 工具"
        print("OK  tools       system_control + baota.*")

        cfg = await call(ws, "c5", "config.get", {})
        assert cfg.get("ok") is True, f"config.get 失败: {cfg}"
        cp = cfg.get("payload", cfg)
        mcp_servers = (cp.get("mcp") or {}).get("servers") or []
        print(f"OK  mcp         config.mcp.servers={len(mcp_servers)} 个")

        await ws.send(json.dumps({"type": "ping"}))
        pong = json.loads(await asyncio.wait_for(ws.recv(), 5))
        print("OK  ping ->", pong.get("type"))

        # sandbox.path.pick 路由探测：Rust 原生对话框会等待用户选择（无头会话挂起）。
        # 默认跳过，避免自动化冒烟被对话框阻塞；桌面验证时设 SMOKE_PICK_PROBE=1。
        if os.environ.get("SMOKE_PICK_PROBE") == "1":
            try:
                await ws.send(json.dumps({"type": "req", "id": "c6", "method": "sandbox.path.pick",
                                          "params": {"sessionKey": "smoke", "kind": "workspace"}}))
                pick = json.loads(await asyncio.wait_for(ws.recv(), 12))
                print(f"INFO sandbox.path.pick -> {json.dumps(pick)[:200]}")
            except asyncio.TimeoutError:
                print("INFO sandbox.path.pick -> 原生对话框等待中（桌面端将正常弹出）")
        else:
            print("INFO sandbox.path.pick -> 已跳过（设 SMOKE_PICK_PROBE=1 探测路由）")

        print("SMOKE ALL OK")

asyncio.run(asyncio.wait_for(main(), 60))
