"""验证 Marguerite 网关 WS 握手（无 token 直连）+ sessions.list 调用。"""
import asyncio, json, sys

URL = "ws://127.0.0.1:18790/ws"

async def main():
    import websockets
    async with websockets.connect(URL) as ws:
        # 1) challenge
        ch = json.loads(await ws.recv())
        print("challenge:", ch)
        assert ch.get("type") == "event" and ch.get("event") == "connect.challenge", "challenge 格式不匹配"

        # 2) connect req（无 token，模拟浏览器首次访问）
        await ws.send(json.dumps({
            "type": "req", "id": "c1", "method": "connect",
            "params": {"auth": {"guestSessionKey": "test-guest"}},
        }))
        hello = json.loads(await asyncio.wait_for(ws.recv(), 5))
        print("hello:", json.dumps(hello)[:300])
        assert hello.get("ok") is True, f"握手失败: {hello}"

        # 3) sessions.list
        await ws.send(json.dumps({
            "type": "req", "id": "c2", "method": "sessions.list",
            "params": {"limit": 5},
        }))
        res = json.loads(await asyncio.wait_for(ws.recv(), 5))
        print("sessions.list:", json.dumps(res)[:400])
        assert res.get("ok") is True, f"sessions.list 失败: {res}"

        # 4) channels.status（Python 方法，经 Rust 网关代理到侧车）
        await ws.send(json.dumps({
            "type": "req", "id": "c3", "method": "channels.status",
            "params": {},
        }))
        ch_res = json.loads(await asyncio.wait_for(ws.recv(), 10))
        print("channels.status:", json.dumps(ch_res)[:400])
        assert ch_res.get("ok") is True, f"channels.status 失败: {ch_res}"

        # 5) 心跳 ping（无空格格式）
        await ws.send(json.dumps({"type": "ping"}))
        pong = json.loads(await asyncio.wait_for(ws.recv(), 5))
        print("ping ->", pong.get("type"))
        print("ALL OK")

asyncio.run(main())
