"""全量体检：对 18790 网关逐一探测关键 RPC，输出每个方法的 ok/错误，不阻塞。"""
import asyncio, json

URL = "ws://127.0.0.1:18790/ws"
METHODS = [
    "connect",
    "plans.capabilities",
    "skills.list",
    "tools.catalog",
    "config.get",
    "config.effective",
    "sandbox.capability.status",
    "sandbox.setup.status",
    "sandbox.policy.list",
    "sandbox.run_mode.get",
    "capability.status",
    "mcp.list",
]

async def call(ws, rid, method, params=None):
    await ws.send(json.dumps({"type": "req", "id": rid, "method": method, "params": params or {}}))
    return json.loads(await asyncio.wait_for(ws.recv(), 12))

async def main():
    import websockets
    async with websockets.connect(URL) as ws:
        await ws.recv()  # connect.challenge
        rid = 0
        for method in METHODS:
            rid += 1
            try:
                resp = await call(ws, f"h{rid}", method)
                ok = resp.get("ok")
                payload = resp.get("payload", resp)
                if method == "connect":
                    ok = resp.get("ok") is True
                    print(f"[{'OK ' if ok else 'FAIL'}] {method} -> {json.dumps(resp)[:120]}")
                    continue
                if not ok:
                    err = resp.get("error") or resp
                    print(f"[FAIL] {method} -> {json.dumps(err)[:200]}")
                    continue
                # 摘要化输出
                if method == "plans.capabilities":
                    items = payload.get("list", []) or []
                    print(f"[OK  ] {method} -> {len(items)} 项能力: {[c.get('id') for c in items]}")
                elif method == "skills.list":
                    names = [s.get("name") for s in (payload.get("skills") or [])]
                    print(f"[OK  ] {method} -> {names}")
                elif method == "tools.catalog":
                    tools = payload.get("items") or []
                    names = sorted(t.get("name") for t in tools)
                    baota = [n for n in names if n.startswith("baota.")]
                    print(f"[OK  ] {method} -> {len(tools)} 个工具; baota.*={baota}")
                    print(f"      其余: {[n for n in names if not n.startswith('baota.')]}")
                elif method == "config.get":
                    mcp = (payload.get("mcp") or {}).get("servers") or []
                    print(f"[OK  ] {method} -> mcp.servers={mcp}")
                    print(f"      model={payload.get('model')}")
                elif method in ("sandbox.capability.status", "sandbox.setup.status", "sandbox.run_mode.get"):
                    print(f"[OK  ] {method} -> {json.dumps(payload, ensure_ascii=False)[:180]}")
                else:
                    print(f"[OK  ] {method} -> {json.dumps(payload, ensure_ascii=False)[:180]}")
            except asyncio.TimeoutError:
                print(f"[TIMEOUT] {method}（12s 无响应）")
            except Exception as exc:
                print(f"[ERR] {method} -> {exc}")
        print("HEALTH CHECK DONE")

asyncio.run(asyncio.wait_for(main(), 60))
