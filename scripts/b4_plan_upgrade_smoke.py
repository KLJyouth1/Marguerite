"""B4 规划升级冒烟：执行前清单 / 动态重规划 / 并行子任务（Rust 网关 18790）。"""
import asyncio, json, sys

URL = "ws://127.0.0.1:18790/ws"

async def rpc(ws, rid, method, params):
    await ws.send(json.dumps({
        "type": "req", "id": rid, "method": method, "params": params,
    }))
    res = json.loads(await asyncio.wait_for(ws.recv(), 20))
    ok = res.get("ok") is True
    return ok, res.get("payload", {}), res.get("error", {})

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
                print(f"[OK ] {name:<32} {json.dumps(detail, ensure_ascii=False)[:140]}")
            else:
                failed += 1
                print(f"[FAIL] {name:<32} {json.dumps(detail, ensure_ascii=False)[:140]}")

        # 1) 创建修订（长难任务自动拆解）
        ok, payload, err = await rpc(ws, "p1", "plans.create", {
            "prompt": "实现登录模块、编写单元测试并部署到测试服务器",
        })
        revision = payload.get("revision", {})
        check("plans.create", ok and revision.get("revisionId", "").startswith("rev-"), revision)
        revision_id = revision.get("revisionId", "")
        steps_count = len(revision.get("steps", []))

        # 2) 启动运行（前置生成执行前清单，初始全未勾选）
        ok, payload, err = await rpc(ws, "p2", "plans.run", {"revisionId": revision_id})
        run = payload.get("run", {})
        checklist = run.get("checklist", [])
        check("plans.run+checklist", ok and len(checklist) == steps_count and all(not c["checked"] for c in checklist), checklist)
        run_id = run.get("runId", "")

        # 3) 读取清单
        ok, payload, err = await rpc(ws, "p3", "plans.checklist", {"runId": run_id})
        check("plans.checklist", ok and len(payload.get("run", {}).get("checklist", [])) == steps_count, payload)

        # 4) 勾选清单项 + 推进下一步
        first_step = checklist[0]["stepId"]
        ok, payload, err = await rpc(ws, "p4", "plans.run.advance", {
            "runId": run_id, "stepId": first_step, "checked": True,
        })
        check("plans.run.advance(勾选)", ok and payload["run"]["checklist"][0]["checked"] is True, payload)
        ok, payload, err = await rpc(ws, "p5", "plans.run.advance", {"runId": run_id, "advance": True})
        check("plans.run.advance(推进)", ok and payload["run"]["steps"][0]["status"] == "completed", payload["run"].get("currentStepId"))

        # 5) 动态重规划：偏离观测 → 生成同 plan 新修订 + run.replan diff
        ok, payload, err = await rpc(ws, "p6", "plans.replan", {
            "runId": run_id, "observation": "遭遇网络超时，切换代理方案，并调整部署顺序",
        })
        replan = payload
        check("plans.replan(偏离)", ok and replan.get("deviated") is True, replan)
        new_revision = replan.get("revision", {})
        check("plans.replan(谱系)", new_revision.get("planId") == revision.get("planId") and new_revision.get("parentRevisionId") == revision_id, new_revision)
        run_replan = replan.get("run", {}).get("replan", {})
        check("plans.replan(diff)", run_replan.get("previousRevisionId") == revision_id and "similarity" in run_replan, run_replan)

        # 6) 并行子任务：确定性扇出聚合
        ok, payload, err = await rpc(ws, "p7", "plans.parallel.run", {"runId": run_id})
        batch = payload.get("run", {}).get("parallelBatch", {})
        check("plans.parallel.run", ok and batch.get("status") == "completed" and batch.get("successful") == batch.get("total", -1), batch)
        # 指定失败子项
        ids = [s["stepId"] for s in run.get("steps", [])]
        fail_target = ids[0] if ids else "step-1"
        ok, payload, err = await rpc(ws, "p8", "plans.parallel.run", {
            "runId": run_id, "stepIds": ids, "failStepIds": [fail_target],
        })
        batch = payload.get("run", {}).get("parallelBatch", {})
        check("plans.parallel.run(含失败)", ok and batch.get("status") == "failed" and batch.get("failed") == 1, batch)

        # 7) 列表确认修订谱系
        ok, payload, err = await rpc(ws, "p9", "plans.list", {})
        check("plans.list", ok and any(r["revisionId"] == new_revision.get("revisionId", "zzz") for r in payload.get("plans", [])), len(payload.get("plans", [])))

        print(f"\n结果: {passed} passed / {failed} failed / {passed + failed} total")
        sys.exit(0 if failed == 0 else 1)

asyncio.run(main())
