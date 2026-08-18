"""Marguerite 双引擎全能力冒烟：验证 webui 依赖的方法组均可经 Rust 网关→Python 侧车调用。"""
import asyncio, json, sys

URL = "ws://127.0.0.1:18790/ws"

# 方法名 -> (params, 说明)  覆盖 webui 各页面
CHECKS = [
    ("sessions.list", {"limit": 5}, "会话列表(Rust)"),
    ("channels.status", {}, "渠道状态(Py)"),
    ("skills.list", {}, "技能列表(Py)"),
    ("skills.status", {}, "技能状态(Py)"),
    ("tools.catalog", {}, "工具目录(Rust)"),
    ("tools.effective", {}, "生效工具(Py)"),
    ("memory.list", {}, "记忆列表(Py)"),
    ("memory.search", {"query": "test", "limit": 5}, "记忆检索(Rust)"),
    ("agents.list", {}, "代理列表(Rust占位)"),
    ("agent.identity.get", {}, "Agent身份(Py)"),
    ("sandbox.status", {}, "沙箱状态(Rust)"),
    ("sandbox.run_context.get", {"sessionKey": "smoke-test"}, "沙箱运行上下文(Py)"),
    ("search.status", {}, "搜索状态(Py)"),
    ("router.selflearning.status", {}, "自学习状态(Py)"),
    ("router.decisions.list", {"limit": 5}, "路由决策(Py)"),
    ("usage.status", {}, "用量状态(Rust)"),
    ("usage.query", {}, "用量查询(Py)"),
    ("onboarding.status", {}, "引导状态(Py)"),
    ("cron.list", {}, "定时任务(Rust占位)"),
    ("providers.status", {}, "模型提供方(Py)"),
    ("models.list", {}, "模型列表(Py)"),
    ("doctor.status", {}, "诊断(Rust)"),
    ("config.get", {}, "配置(Rust)"),
    ("logs.status", {}, "日志状态(Py)"),
]

async def main():
    import websockets
    async with websockets.connect(URL) as ws:
        ch = json.loads(await ws.recv())
        assert ch.get("event") == "connect.challenge"
        await ws.send(json.dumps({
            "type": "req", "id": "c1", "method": "connect",
            "params": {"auth": {}},
        }))
        hello = json.loads(await asyncio.wait_for(ws.recv(), 5))
        assert hello.get("type") == "hello-ok" or hello.get("ok") is True, f"握手失败 {hello}"

        passed, failed = 0, 0
        for i, (method, params, note) in enumerate(CHECKS):
            await ws.send(json.dumps({
                "type": "req", "id": f"m{i}", "method": method, "params": params,
            }))
            res = json.loads(await asyncio.wait_for(ws.recv(), 20))
            ok = res.get("ok") is True
            if ok:
                passed += 1
                detail = json.dumps(res.get("payload", {}))[:120]
            else:
                failed += 1
                err = res.get("error", {})
                detail = f"{err.get('code')}: {err.get('message')}" if isinstance(err, dict) else str(res)[:120]
            print(f"[{'OK ' if ok else 'FAIL'}] {method:<28} {note:<18} {detail}")

        print(f"\n结果: {passed} passed / {failed} failed / {len(CHECKS)} total")
        sys.exit(0 if failed == 0 else 1)

asyncio.run(main())
