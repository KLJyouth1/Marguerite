"""B4 规划升级冒烟：执行前清单 / 前瞻式动态重规划 / 并行子任务（Rust 网关）。

默认打回环 18790；本机若以非默认端口起网关（如 `gateway --port 18801`），
用 `MARGUERITE_WS=ws://127.0.0.1:18801/ws python scripts/dev/b4_plan_upgrade_smoke.py`。
"""
import asyncio, json, os, sys

URL = os.environ.get("MARGUERITE_WS", "ws://127.0.0.1:18790/ws")

async def rpc(ws, rid, method, params):
    await ws.send(json.dumps({
        "type": "req", "id": rid, "method": method, "params": params,
    }))
    # 网关会与响应交错推送事件帧（tick / plan_run / plan_revision …），
    # 只读一帧会把事件当响应 → 必须按 id 配对到本次请求的 res 帧。
    while True:
        res = json.loads(await asyncio.wait_for(ws.recv(), 20))
        if res.get("type") == "res" and res.get("id") == rid:
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
        steps = revision.get("steps", [])
        steps_count = len(steps)

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
        check("plans.replan(判定模式)", ok and replan.get("mode") == "rebuild" and replan.get("applied") is True, replan.get("mode"))
        check("plans.replan(代价模型)", ok and "remainingSteps" in replan.get("costBefore", {}) and "estimatedRemainingMs" in replan.get("costAfter", {}), replan.get("costBefore"))
        new_revision = replan.get("revision", {})
        check("plans.replan(谱系)", new_revision.get("planId") == revision.get("planId") and new_revision.get("parentRevisionId") == revision_id, new_revision)
        run_replan = replan.get("run", {}).get("replan", {})
        check("plans.replan(diff)", run_replan.get("previousRevisionId") == revision_id and "similarity" in run_replan, run_replan)

        # 5b) 前瞻式重规划：观测命中**后续**步骤 → 判定已推进（跳步）
        #     旧实现只比对当前步骤，这种情形与「真偏离」同为低相似度、无法区分。
        #     先在原修订上验证「依赖安全闸」：step-2 依赖未完成的 step-1 → 只回报不落地。
        ok, payload, err = await rpc(ws, "p6a", "plans.run", {"revisionId": revision_id})
        dep_run_id = payload.get("run", {}).get("runId", "")
        ok, payload, err = await rpc(ws, "p6b", "plans.replan", {
            "runId": dep_run_id, "observation": steps[-1]["title"], "apply": True,
        })
        dep = payload
        check("plans.replan(前瞻模式)", ok and dep.get("mode") == "advance", dep.get("mode"))
        check("plans.replan(命中后续步)", ok and dep.get("bestMatch", {}).get("distance", 0) >= 1, dep.get("bestMatch"))
        check("plans.replan(前瞻窗口)", ok and len(dep.get("lookahead", [])) >= 2 and dep["lookahead"][0].get("distance") == 0, dep.get("lookahead"))
        check("plans.replan(依赖安全闸)", ok and dep.get("applied") is False and len(dep.get("blockedByDeps", [])) > 0, dep.get("blockedByDeps"))

        # 追加一个**无依赖**步骤 → 跳步应真落地（跳过其间未完成步骤）
        ok, payload, err = await rpc(ws, "p6c", "plans.step.add", {
            "planRevisionId": revision_id, "stepTitle": "验证与收尾", "deps": [],
        })
        adv_rev = payload.get("planRevision", {})
        check("plans.step.add", ok and len(adv_rev.get("steps", [])) == steps_count + 1, len(adv_rev.get("steps", [])))
        ok, payload, err = await rpc(ws, "p6d", "plans.run", {"revisionId": adv_rev.get("revisionId", "")})
        adv_run_id = payload.get("run", {}).get("runId", "")
        ok, payload, err = await rpc(ws, "p6e", "plans.replan", {
            "runId": adv_run_id, "observation": "验证与收尾", "apply": True,
        })
        adv = payload
        adv_run = adv.get("run", {})
        check("plans.replan(跳步落地)", ok and adv.get("applied") is True and adv.get("skippedSteps", 0) >= 1, {"applied": adv.get("applied"), "skippedSteps": adv.get("skippedSteps")})
        check("plans.replan(当前步=命中步)", ok and adv_run.get("currentStepId") == adv.get("bestMatch", {}).get("stepId"), adv_run.get("currentStepId"))
        check("plans.replan(跳步 diff)", ok and adv_run.get("replan", {}).get("mode") == "advance" and adv_run["replan"].get("skippedStepIds"), adv_run.get("replan", {}).get("skippedStepIds"))

        # 未显式 apply → 只回报建议，不改运行
        ok, payload, err = await rpc(ws, "p6f", "plans.run", {"revisionId": adv_rev.get("revisionId", "")})
        dry_run_id = payload.get("run", {}).get("runId", "")
        ok, payload, err = await rpc(ws, "p6g", "plans.replan", {
            "runId": dry_run_id, "observation": "验证与收尾",
        })
        check("plans.replan(默认不落地)", ok and payload.get("applied") is False and len(payload.get("proposedSkippedStepIds", [])) >= 1, payload.get("proposedSkippedStepIds"))

        # 6) 并行子任务：有 LLM Key 时后台真执行（立即返回、终态经事件/快照回流），
        #    离线才走确定性扇出标记。两种形态分别断言，不把「立即返回的空批」当失败。
        ok, payload, err = await rpc(ws, "p7", "plans.parallel.run", {"runId": run_id})
        if payload.get("simulated") is True:
            batch = payload.get("run", {}).get("parallelBatch") or {}
            check("plans.parallel.run(离线扇出)", ok and batch.get("status") == "completed" and batch.get("successful") == batch.get("total", -1), batch)
            # 指定失败子项（仅离线扇出支持注入失败）
            ids = [s["stepId"] for s in run.get("steps", [])]
            ok, payload, err = await rpc(ws, "p8", "plans.parallel.run", {
                "runId": run_id, "stepIds": ids, "failStepIds": [ids[0] if ids else "step-1"],
            })
            batch = payload.get("run", {}).get("parallelBatch") or {}
            check("plans.parallel.run(含失败)", ok and batch.get("status") == "failed" and batch.get("failed") == 1, batch)
        else:
            check("plans.parallel.run(已受理)", ok and payload.get("executed") is True, {"executed": payload.get("executed")})
            batch = {}
            for _ in range(90):
                _, snap, _ = await rpc(ws, "p8", "plans.checklist", {"runId": run_id})
                batch = (snap.get("run", {}) or {}).get("parallelBatch") or {}
                if batch:
                    break
                await asyncio.sleep(1)
            # 断言「批已真执行且如实报告」，不强求全部成功：治理门/宪法是**确定性裁决**
            # （如含部署等不可逆动作会被 C1 拦下、不重试），此时失败才是正确行为。
            results = batch.get("results", [])
            check("plans.parallel.run(真执行收口)", bool(batch) and batch.get("executed") is True and len(results) == batch.get("total", -1) and all(r.get("status") in ("completed", "failed") for r in results), {"status": batch.get("status"), "successful": batch.get("successful"), "total": batch.get("total")})
            check("plans.parallel.run(失败须有据)", bool(batch) and all(r.get("status") != "failed" or r.get("summary") for r in results), (results or [{}])[0].get("summary", "")[:120])
            check("plans.parallel.run(真实耗时)", bool(batch) and all("elapsedMs" in r for r in results), (results or [{}])[0])

        # 7) 列表确认修订谱系
        ok, payload, err = await rpc(ws, "p9", "plans.list", {})
        check("plans.list", ok and any(r["revisionId"] == new_revision.get("revisionId", "zzz") for r in payload.get("plans", [])), len(payload.get("plans", [])))

        print(f"\n结果: {passed} passed / {failed} failed / {passed + failed} total")
        sys.exit(0 if failed == 0 else 1)

asyncio.run(main())
