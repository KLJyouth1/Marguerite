"""精准复测：每个方法用独立连接，串行，避免网关串行排队伪影。"""
import asyncio, json

URL = "ws://127.0.0.1:18790/ws"
METHODS = [
    ("sandbox.setup.ensure", {}),
    ("sandbox.run_context.set", {"runMode": "full"}),
    ("sandbox.terminal.list", {}),
    ("sandbox.terminal.read", {}),
    ("sessions.abort", {"key": "__probe__"}),
    ("sessions.delete", {"keys": []}),
    ("sandbox.run_mode.preference.get", {}),
    ("sandbox.setup.status", {}),
    ("sandbox.path.pick", {}),
]

async def probe(method, params):
    import websockets
    try:
        async with websockets.connect(URL, open_timeout=5) as ws:
            await ws.recv()
            await ws.send(json.dumps({"type": "req", "id": "p0", "method": "connect", "params": {"auth": {"guestSessionKey": "probe"}}}))
            await ws.recv()
            await ws.send(json.dumps({"type": "req", "id": "p1", "method": method, "params": params}))
            r = json.loads(await asyncio.wait_for(ws.recv(), 8))
            if r.get("ok") is True:
                print(f"[OK  ] {method} -> {json.dumps(r.get('payload'), ensure_ascii=False)[:160]}")
            else:
                err = r.get("error") or {}
                print(f"[FAIL] {method} -> {err.get('code')} {err.get('message')}")
    except asyncio.TimeoutError:
        print(f"[TIMEOUT] {method}（8s 无响应）")
    except Exception as e:
        print(f"[ERR] {method} -> {type(e).__name__}: {e}")

async def main():
    for m, p in METHODS:
        await probe(m, p)
        await asyncio.sleep(1)
    print("DONE")

asyncio.run(asyncio.wait_for(main(), 120))
