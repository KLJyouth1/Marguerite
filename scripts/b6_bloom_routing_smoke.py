"""B6 模型路由 Bloom 融合路由冒烟（pyengine 网关 18793）。

链路：connect 握手 → models.routing.get（初始模式）→ set bloom（快照断言）→ get（持久化断言）→ set 恢复初始模式（无副作用）。
"""
import asyncio, json, sys

URL = "ws://127.0.0.1:18793/ws"

async def rpc(ws, rid, method, params):
    await ws.send(json.dumps({
        "type": "req", "id": rid, "method": method, "params": params,
    }))
    while True:
        res = json.loads(await asyncio.wait_for(ws.recv(), 20))
        if res.get("type") == "res" and res.get("id") == rid:
            ok = res.get("ok") is True
            return ok, res.get("payload", {}), res.get("error", {})
        # 事件帧（models.routing.changed 等）插队：跳过，等待匹配的 res

async def main():
    import websockets
    async with websockets.connect(URL) as ws:
        ch = json.loads(await ws.recv())
        assert ch.get("event") == "connect.challenge", f"challenge {ch}"
        await ws.send(json.dumps({
            "type": "req", "id": "c1", "method": "connect",
            "params": {"auth": {}},
        }))
        hello = json.loads(await asyncio.wait_for(ws.recv(), 5))
        assert hello.get("type") == "hello-ok" or hello.get("ok") is True, f"握手失败 {hello}"

        passed, failed = 0, 0
        def check(name, ok, detail=""):
            nonlocal passed, failed
            if ok:
                passed += 1
                print(f"[OK ] {name:<36} {json.dumps(detail, ensure_ascii=False)[:150]}")
            else:
                failed += 1
                print(f"[FAIL] {name:<36} {json.dumps(detail, ensure_ascii=False)[:150]}")

        # 0) 记录初始模式，冒烟结束后恢复（无副作用）
        ok, initial, err = await rpc(ws, "g0", "models.routing.get", {})
        initial_mode = initial.get("mode", "direct")
        check("models.routing.get 初始", ok and isinstance(initial_mode, str), initial)

        # 1) set bloom：快照应为 bloom + 编排 auto + 融合/Router 双开
        ok, snap, err = await rpc(ws, "b1", "models.routing.set", {"mode": "bloom"})
        check(
            "set bloom 快照",
            ok
            and snap.get("mode") == "bloom"
            and snap.get("orchestration_mode") == "auto"
            and snap.get("ensemble_enabled") is True
            and snap.get("router_enabled") is True,
            snap,
        )

        # 2) get 持久化确认（跨请求）
        ok, snap2, err = await rpc(ws, "b2", "models.routing.get", {})
        check("get 持久化 bloom", ok and snap2.get("mode") == "bloom" and snap2.get("orchestration_mode") == "auto", snap2)

        # 3) set ensemble：编排应回落 off（bloom 与普通融合路由互斥）
        ok3, snap3, err = await rpc(ws, "b3", "models.routing.set", {"mode": "ensemble"})
        check(
            "set ensemble 回落",
            ok3 and snap3.get("mode") == "ensemble" and snap3.get("orchestration_mode") == "off",
            snap3,
        )

        # 4) 恢复初始模式
        ok, restored, err = await rpc(ws, "b4", "models.routing.set", {"mode": initial_mode})
        check("恢复初始模式", ok and restored.get("mode") == initial_mode, restored)

        print(f"\nBloom 融合路由冒烟: {passed} passed, {failed} failed")
        sys.exit(1 if failed else 0)

if __name__ == "__main__":
    asyncio.run(main())
