"""
btpanel-mcp 命令行入口。

用法:
    btpanel-mcp stdio                          # stdio 传输（MCP 客户端默认）
    btpanel-mcp sse --host 127.0.0.1 --port 8765   # HTTP+SSE 传输

环境变量:
    BAOTA_BASE_URL       面板地址，如 http://127.0.0.1:8888/安全路径（必填才能调工具）
    BAOTA_SESSION_FILE   会话 cookie JSON 文件路径（浏览器开发者工具导出形态）
    BAOTA_REQUEST_TOKEN  面板 token（Windows 面板风格，与 SESSION_FILE 二选一）
    BAOTA_DRY_RUN        默认 true：写操作只模拟不执行
    BAOTA_CONFIRM_EXEC   默认 true：exec 工具需该开关且命令在白名单内才执行
"""
from __future__ import annotations

import argparse
import os
import sys

from .bt_api import BaotaConfig
from .mcp_server import BtMCP


def _env_bool(name: str, default: bool) -> bool:
    raw = os.getenv(name, "true" if default else "false").strip().lower()
    return raw in ("1", "true", "yes", "on")


def load_config() -> BaotaConfig:
    """从环境变量加载配置（凭据只来自环境变量/会话文件，严禁硬编码）。"""
    return BaotaConfig(
        base_url=os.getenv("BAOTA_BASE_URL", "").strip(),
        session_file=os.getenv("BAOTA_SESSION_FILE", "").strip() or None,
        request_token=os.getenv("BAOTA_REQUEST_TOKEN", "").strip() or None,
        dry_run=_env_bool("BAOTA_DRY_RUN", True),
        confirm_exec=_env_bool("BAOTA_CONFIRM_EXEC", True),
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="btpanel-mcp", description="宝塔面板(BT Panel) MCP Server")
    sub = parser.add_subparsers(dest="transport", required=True, metavar="{stdio,sse}")

    sub.add_parser("stdio", help="stdio 传输（换行分隔 JSON-RPC）")

    p_sse = sub.add_parser("sse", help="HTTP+SSE 传输")
    p_sse.add_argument("--host", default="127.0.0.1", help="监听地址（默认 127.0.0.1，请勿对外开放）")
    p_sse.add_argument("--port", type=int, default=8765, help="监听端口（默认 8765）")

    args = parser.parse_args(argv)
    config = load_config()
    server = BtMCP(config)

    if args.transport == "stdio":
        server.run_stdio()
    else:
        server.run_sse(host=args.host, port=args.port)
    return 0


if __name__ == "__main__":
    sys.exit(main())
