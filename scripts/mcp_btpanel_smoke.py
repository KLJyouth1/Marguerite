"""宝塔面板 MCP 接入冒烟：验证 opensquilla discovery 能把 btpanel-mcp 注册为 `mcp_baota_*` 工具。

链路：MCPServerConfig(stdio) → discover_and_register → ToolRegistry 注册 mcp_* 工具。
tools/list 无需真实面板凭据；工具调用缺凭据时返回友好错误（不触网）。
"""
import asyncio
import json
import sys

from opensquilla.mcp.discovery import discover_and_register
from opensquilla.mcp.types import MCPServerConfig
from opensquilla.tools.registry import ToolRegistry

EXPECTED_TOOLS = {
    "mcp_baota_health",
    "mcp_baota_sites_list",
    "mcp_baota_site_create",
    "mcp_baota_site_status",
    "mcp_baota_databases_list",
    "mcp_baota_database_create",
    "mcp_baota_ftps_list",
    "mcp_baota_ftp_create",
    "mcp_baota_files_list",
    "mcp_baota_file_read",
    "mcp_baota_file_write",
    "mcp_baota_file_delete",
    "mcp_baota_file_upload",
    "mcp_baota_system_info",
    "mcp_baota_network_info",
    "mcp_baota_exec",
}


async def main() -> int:
    cfg = MCPServerConfig(
        name="btpanel",
        transport="stdio",
        command="py",
        args=["-3.14", "-m", "btpanel_mcp.cli", "stdio"],
        tool_timeout_seconds=30,
        env={"PYTHONPATH": "d:/Downloads/Marguerite/marguerite/btpanel-mcp"},
    )
    registry = ToolRegistry()
    registered = await discover_and_register(cfg, registry, owner="smoke")
    names = set(registered)
    missing = EXPECTED_TOOLS - names
    if missing:
        print(f"[FAIL] 缺失工具: {sorted(missing)}")
        return 1
    if len(names) != len(EXPECTED_TOOLS):
        print(f"[WARN] 注册 {len(names)} 个工具，预期 {len(EXPECTED_TOOLS)}")
    print(f"[OK ] discovery 注册 {len(names)} 个 mcp_baota_* 工具")

    # 缺凭据调用：应返回友好错误文本而非崩溃/触网
    spec = registry.get("mcp_baota_sites_list")
    if spec is None:
        print("[FAIL] mcp_baota_sites_list 未注册")
        return 1
    try:
        out = await spec.handler(page=1, limit=15)
        print(f"[OK ] 无凭据调用返回友好结果: {str(out)[:120]}")
    except Exception as e:  # SafeToolError 是预期错误形态之一
        msg = str(e)
        if "BAOTA" in msg or "配置" in msg or "未配置" in msg or "token" in msg.lower():
            print(f"[OK ] 无凭据调用返回预期错误: {msg[:120]}")
        else:
            print(f"[FAIL] 无凭据调用错误信息不友好: {msg[:120]}")
            return 1

    from opensquilla.mcp.discovery import close_active_clients

    await close_active_clients("smoke")
    print("PASS: 宝塔面板 MCP 接入冒烟全部通过")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
