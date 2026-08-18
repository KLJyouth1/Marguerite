"""v14 冒烟：kb.* RPC + config.get 离线兜底 + sessions.list 台账合并。"""
import asyncio, json

URL = "ws://127.0.0.1:18790/ws"

async def call(ws, mid, method, params):
    await ws.send(json.dumps({"type": "req", "id": mid, "method": method, "params": params}))
    r = json.loads(await asyncio.wait_for(ws.recv(), 10))
    return r

async def probe(method, params):
    import websockets
    try:
        async with websockets.connect(URL, open_timeout=5) as ws:
            await ws.recv()
            await call(ws, "p0", "connect", {"auth": {"guestSessionKey": "probe"}})
            r = await call(ws, "p1", method, params)
            if r.get("ok") is True:
                p = r.get("payload")
                print(f"[OK  ] {method} -> {json.dumps(p, ensure_ascii=False)[:220]}")
            else:
                err = r.get("error") or {}
                print(f"[FAIL] {method} -> {err.get('code')} {err.get('message')}")
    except asyncio.TimeoutError:
        print(f"[TIMEOUT] {method}")
    except Exception as e:
        print(f"[ERR] {method} -> {type(e).__name__}: {e}")

async def main():
    await probe("kb.status", {})
    await asyncio.sleep(0.5)
    await probe("kb.search", {"query": "配置 模型 密钥"})
    await asyncio.sleep(0.5)
    await probe("kb.ingest", {"text": "冒烟测试文档：验证 kb.ingest 灌入链路。", "source": "smoke-v14"})
    await asyncio.sleep(0.5)
    await probe("config.get", {})
    await asyncio.sleep(0.5)
    await probe("sessions.list", {})
    print("DONE")

asyncio.run(asyncio.wait_for(main(), 120))
