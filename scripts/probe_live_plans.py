"""Probe the running gateway: sessions.list + plans.list to see the live plan/card data."""
import asyncio
import json

import websockets

URL = "ws://127.0.0.1:18790/ws"


async def call(ws, rid, method, params=None):
    await ws.send(json.dumps({"type": "req", "id": rid, "method": method, "params": params or {}}))
    return json.loads(await asyncio.wait_for(ws.recv(), 12))


async def main():
    async with websockets.connect(URL) as ws:
        await asyncio.wait_for(ws.recv(), 5)  # connect.challenge
        rid = 0

        def next_rid():
            nonlocal rid
            rid += 1
            return f"p{rid}"

        for method, params in [
            ("connect", {}),
            ("plans.list", {}),
            ("sessions.list", {}),
        ]:
            try:
                resp = await call(ws, next_rid(), method, params)
                print("=" * 60)
                print(method, "->", json.dumps(resp, ensure_ascii=False)[:2500])
            except Exception as exc:
                print("=" * 60)
                print(method, "ERR", repr(exc))


asyncio.run(asyncio.wait_for(main(), 30))
