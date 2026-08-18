#!/usr/bin/env python3
"""
btpanel-mcp stdio 冒烟自测：无需真实面板凭据即可运行。

流程：以 subprocess 以 stdio 方式启动 server，完成 initialize + notifications/initialized
      + tools/list，断言工具名集合（>=16 个），并验证无凭据调用工具返回友好错误。
运行：py -3.14 d:\\Downloads\\Marguerite\\marguerite\\btpanel-mcp\\smoke_stdio.py
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

PKG_ROOT = Path(__file__).resolve().parent

EXPECTED_TOOLS: set[str] = {
    "baota_health",
    "baota_sites_list",
    "baota_site_create",
    "baota_site_status",
    "baota_databases_list",
    "baota_database_create",
    "baota_ftps_list",
    "baota_ftp_create",
    "baota_files_list",
    "baota_file_read",
    "baota_file_write",
    "baota_file_delete",
    "baota_file_upload",
    "baota_system_info",
    "baota_network_info",
    "baota_exec",
    "baota_login",
    "baota_credentials",
    "baota_logout",
}
MIN_TOOLS = 19


def send(proc: subprocess.Popen, obj: dict) -> dict:
    """写一行 JSON-RPC 请求，读取并返回匹配 id 的响应行。"""
    proc.stdin.write((json.dumps(obj, ensure_ascii=False) + "\n").encode("utf-8"))
    proc.stdin.flush()
    while True:
        line = proc.stdout.readline()
        if not line:
            raise RuntimeError("server 提前退出，未收到响应")
        try:
            resp = json.loads(line.decode("utf-8"))
        except json.JSONDecodeError:
            continue
        if resp.get("id") == obj.get("id"):
            return resp


def main() -> int:
    # 干净环境：不注入任何面板凭据，验证 tools/list 在无凭据时也能返回
    env = dict(os.environ)
    for key in ("BAOTA_BASE_URL", "BAOTA_SESSION_FILE", "BAOTA_REQUEST_TOKEN"):
        env.pop(key, None)
    env["BAOTA_DRY_RUN"] = "true"
    env["PYTHONPATH"] = str(PKG_ROOT) + os.pathsep + env.get("PYTHONPATH", "")

    proc = subprocess.Popen(
        [sys.executable, "-m", "btpanel_mcp.cli", "stdio"],
        cwd=str(PKG_ROOT),
        env=env,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        # 1) initialize 握手
        init = send(proc, {
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                       "clientInfo": {"name": "smoke_stdio", "version": "1.0"}},
        })
        result = init.get("result", {})
        assert result.get("protocolVersion") == "2024-11-05", f"协议版本不符: {init}"
        assert result.get("serverInfo", {}).get("name") == "btpanel-mcp"
        assert result.get("capabilities", {}).get("tools") is not None
        print("[OK] initialize")

        # 2) notifications/initialized（通知，无需响应）
        proc.stdin.write(
            (json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n").encode("utf-8")
        )
        proc.stdin.flush()

        # 3) tools/list：无凭据也应返回全部工具
        lst = send(proc, {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        tools = lst.get("result", {}).get("tools", [])
        names = {t.get("name") for t in tools}
        assert len(tools) >= MIN_TOOLS, f"工具数不足: {len(tools)} < {MIN_TOOLS}"
        missing = EXPECTED_TOOLS - names
        assert not missing, f"缺少工具: {sorted(missing)}"
        for t in tools:
            assert "inputSchema" in t, f"工具缺少 inputSchema: {t.get('name')}"
        print(f"[OK] tools/list 返回 {len(tools)} 个工具，16 个预期工具齐全且均带 Schema")

        # 4) 无凭据调用工具应返回友好错误（isError=true）而非崩溃
        call = send(proc, {"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                           "params": {"name": "baota_sites_list", "arguments": {}}})
        call_result = call.get("result", {})
        assert call_result.get("isError") is True, "无凭据时应返回 isError=true"
        text = call_result.get("content", [{}])[0].get("text", "")
        assert "BAOTA_BASE_URL" in text, f"错误信息应提示配置 BAOTA_BASE_URL: {text}"
        print("[OK] 无凭据工具调用返回友好错误（提示配置）")

        # 5) baota_health 无凭据时返回配置状态而非报错
        health = send(proc, {"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                             "params": {"name": "baota_health", "arguments": {}}})
        health_result = health.get("result", {})
        assert health_result.get("isError") is False, "baota_health 无凭据不应报错"
        print("[OK] baota_health 无凭据返回配置状态")

        # 6) 未知工具应返回 JSON-RPC 错误
        bad = send(proc, {"jsonrpc": "2.0", "id": 5, "method": "tools/call",
                          "params": {"name": "no_such_tool", "arguments": {}}})
        assert "error" in bad, f"未知工具应返回 JSON-RPC 错误: {bad}"
        print("[OK] 未知工具返回 JSON-RPC 错误")

        # 7) ping
        pong = send(proc, {"jsonrpc": "2.0", "id": 6, "method": "ping", "params": {}})
        assert pong.get("result") == {}, f"ping 应返回空 result: {pong}"
        print("[OK] ping")
    finally:
        try:
            proc.stdin.close()
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()

    print("=" * 52)
    print("PASS: smoke_stdio 全部通过（initialize / tools/list / 错误处理 / ping）")
    print("=" * 52)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:
        print(f"FAIL: {e}")
        sys.exit(1)
