"""item 10 运行时实测：直连 Marguerite 网关 /ws WebSocket JSON-RPC。

能力链：models.config_path → artifacts.open_in_system {path}。
只打真实网关 RPC（不走浏览器代理），逐帧打印请求/响应作为运行时证据。
"""
import asyncio
import json
import os
import sys

import websockets

URL = os.environ.get("ITEM10_WS_URL", "ws://127.0.0.1:18790/ws")
TOKEN_FILE = os.environ.get(
    "ITEM10_TOKEN_FILE",
    os.path.join(os.environ.get("MARGUERITE_DATA_DIR", ""), "gateway-token"),
)


async def recv_json(ws, timeout=15):
    """收一帧并解析 JSON（跳过无法解析的文本帧）。"""
    raw = await asyncio.wait_for(ws.recv(), timeout)
    print(f"  ← 响应帧: {raw}")
    return json.loads(raw)


async def call(ws, rid, method, params):
    """发一个 req 帧并返回同 id 的 res 帧（跳过中间的 event/tick 帧）。"""
    frame = {"type": "req", "id": rid, "method": method, "params": params}
    raw = json.dumps(frame, ensure_ascii=False)
    print(f"  → 请求帧: {raw}")
    await ws.send(raw)
    while True:
        resp = await recv_json(ws)
        if resp.get("type") == "res" and resp.get("id") == rid:
            return resp


async def main():
    token = ""
    if TOKEN_FILE and os.path.isfile(TOKEN_FILE):
        with open(TOKEN_FILE, "r", encoding="utf-8") as f:
            token = f.read().strip()
    print(f"[连接] {URL}；token 文件: {TOKEN_FILE or '(未设置)'}；token 长度: {len(token)}")

    async with websockets.connect(URL) as ws:
        # 握手第一步：服务端推 connect.challenge
        ch = await recv_json(ws)
        assert ch.get("type") == "event" and ch.get("event") == "connect.challenge", "challenge 帧格式不符"

        # 握手第二步：connect req（带 token）
        hello = await call(ws, "c1", "connect", {
            "minProtocol": 3,
            "maxProtocol": 3,
            "auth": {"token": token, "guestSessionKey": "item10-runtime-probe"},
        })
        assert hello.get("ok") is True, f"握手失败: {hello}"
        methods = (hello.get("features") or {}).get("methods") or []
        print(f"[握手成功] protocol={hello.get('protocol')}；SUPPORTED_METHODS={len(methods)} 条")
        print(f"  models.config_path 在清单: {'models.config_path' in methods}；"
              f"artifacts.open_in_system 在清单: {'artifacts.open_in_system' in methods}")

        # ① models.config_path
        print("\n[调用 1] models.config_path params={}")
        r1 = await call(ws, "c2", "models.config_path", {})
        p1 = r1.get("payload") or {}
        print(f"[结果 1] ok={r1.get('ok')} payload={json.dumps(p1, ensure_ascii=False)}")
        cfg_path = p1.get("path") or ""
        assert r1.get("ok") is True and cfg_path, f"models.config_path 失败: {r1}"
        # 路径必须落在隔离配置目录内（存在性字段如实回传，不强求 exists=true）
        expected_dir = os.environ.get("MARGUERITE_CONFIG_DIR", "").rstrip("\\/")
        assert cfg_path.replace("/", "\\").lower().startswith(expected_dir.replace("/", "\\").lower()), \
            f"config_path 不在隔离配置目录内: {cfg_path}（期望前缀 {expected_dir}）"
        print(f"  ✓ 路径校验：位于 {expected_dir} 内；exists={p1.get('exists')}（如实回传）")
        if not os.path.isfile(cfg_path):
            # models.json 是懒写文件：干净环境首查不存在（exists=false 如实）。
            # 用 models.profiles.add 落盘一张 __item10_probe__ 档案，真实驱动一次写路径。
            print("\n[准备] models.json 尚不存在，调用 models.profiles.add 触发真实落盘")
            rp = await call(ws, "c2b", "models.profiles.add", {
                "id": "__item10_probe__",
                "label": "Item10 Probe",
                "provider": "custom",
                "model": "item10-probe",
                "base_url": "http://127.0.0.1:9/item10",
            })
            pp = rp.get("payload") or {}
            print(f"[准备结果] ok={rp.get('ok')} payload={json.dumps(pp, ensure_ascii=False)[:300]}")
            assert rp.get("ok") is True and (pp.get("ok") is True) and os.path.isfile(cfg_path), \
                f"profiles.add 后 models.json 仍未落盘: {rp}"
            # 复查 models.config_path：此时应如实回报 exists=true
            r1b = await call(ws, "c2c", "models.config_path", {})
            p1b = r1b.get("payload") or {}
            print(f"[复查 1] ok={r1b.get('ok')} payload={json.dumps(p1b, ensure_ascii=False)}")
            assert p1b.get("exists") is True, f"落盘后 exists 应为 true: {r1b}"
            # 清理探针档案（id 形如 __*__ 的探针条目由脚本负责删除）
            rd = await call(ws, "c2d", "models.profiles.remove", {"id": "__item10_probe__"})
            print(f"[清理] models.profiles.remove ok={rd.get('ok')}")
            assert rd.get("ok") is True, f"探针档案清理失败: {rd}"
            assert os.path.isfile(cfg_path), "清理后 models.json 应保留（删除档案 ≠ 删除文件）"
        print(f"  ✓ models.json 真实存在: {cfg_path}")

        # ② artifacts.open_in_system {path}（path 取自上一步真实返回）
        print(f"\n[调用 2] artifacts.open_in_system params={{\"path\": {json.dumps(cfg_path)}}}")
        r2 = await call(ws, "c3", "artifacts.open_in_system", {"path": cfg_path})
        p2 = r2.get("payload") or {}
        print(f"[结果 2] ok={r2.get('ok')} payload={json.dumps(p2, ensure_ascii=False)}")
        assert r2.get("ok") is True, f"artifacts.open_in_system 失败: {r2}"
        # 方法返回两种信封：artifacts 系用 {code,message,data}，成功 code=0
        assert p2.get("code", 0) == 0, f"open_in_system 业务信封非 code=0: {r2}"
        assert (p2.get("data") or p2).get("path") == cfg_path, f"open_in_system 回显路径不符: {r2}"
        print("  ✓ 系统默认程序打开成功（code=0，回显 path 与请求一致）")

        # ③ 路径校验反例：不存在的路径必须被拒绝（1404）
        print("\n[调用 3] artifacts.open_in_system 反例（不存在路径）")
        r3 = await call(ws, "c4", "artifacts.open_in_system", {"path": "E:\\temp\\__item10_not_exist__.txt"})
        p3 = r3.get("payload") or {}
        print(f"[结果 3] ok={r3.get('ok')} payload={json.dumps(p3, ensure_ascii=False)}")
        assert p3.get("code") == 1404, f"不存在路径应拒 1404: {r3}"

        print("\nITEM10 WS RPC PROBE: ALL OK")
        return {
            "hello": hello,
            "config_path": r1,
            "open_in_system": r2,
            "open_in_system_bad_path": r3,
        }


result = asyncio.run(main())
