#!/usr/bin/env python3
"""NeoData 金融数据查询客户端（通过代理 API）

Usage:
    python query.py --query "腾讯最新财报"
    python query.py --query "贵州茅台股价" --data-type api
    python query.py --save-token "<JWT>"

鉴权优先级: --token 参数 > ~/.workbuddy/.neodata_token 文件

环境变量 (可选):
    NEODATA_ENDPOINT - 代理 URL (可选，默认 https://copilot.tencent.com/agenttool/v1/neodata)
"""

import argparse
import json
import os
import stat
import sys
from pathlib import Path
from typing import Optional

try:
    import requests
except ImportError:
    print("需要安装 requests: pip install requests", file=sys.stderr)
    sys.exit(1)

DEFAULT_ENDPOINT = "https://copilot.tencent.com/agenttool/v1/neodata"
TOKEN_FILE = Path.home() / ".workbuddy" / ".neodata_token"


def _read_token_file() -> Optional[str]:
    """从 ~/.workbuddy/.neodata_token 读取 token"""
    try:
        token = TOKEN_FILE.read_text().strip()
        return token if token else None
    except (FileNotFoundError, PermissionError):
        return None


def _save_token_file(token: str) -> None:
    """将 token 写入 ~/.workbuddy/.neodata_token（权限 600）"""
    TOKEN_FILE.parent.mkdir(parents=True, exist_ok=True)
    TOKEN_FILE.write_text(token.strip())
    TOKEN_FILE.chmod(stat.S_IRUSR | stat.S_IWUSR)


def query_neodata(
    query: str,
    data_type: str = "all",
    token: Optional[str] = None,
    endpoint: Optional[str] = None,
) -> dict:
    url = endpoint or os.getenv("NEODATA_ENDPOINT", DEFAULT_ENDPOINT)
    jwt_token = token or _read_token_file()
    if not jwt_token:
        print("错误: 未找到 token。请先运行 --save-token 保存，或使用 --token 参数传入", file=sys.stderr)
        sys.exit(1)

    headers = {
        "Content-Type": "application/json",
        "Authorization": f"Bearer {jwt_token}",
    }

    # channel 和 sub_channel 为固定字段，必须显式传入
    payload: dict = {
        "query": query,
        "channel": "neodata",
        "sub_channel": "workbuddy",
    }
    if data_type != "all":
        payload["data_type"] = data_type

    resp = requests.post(url, headers=headers, json=payload, timeout=30)
    resp.raise_for_status()
    return resp.json()


def main():
    parser = argparse.ArgumentParser(description="NeoData 金融数据查询")
    parser.add_argument("--query", "-q", default=None, help="自然语言查询")
    parser.add_argument("--token", "-t", default=None, help="JWT token（优先级高于文件缓存）")
    parser.add_argument("--data-type", "-d", default="all", choices=["all", "api", "doc"], help="数据类型 (默认: all)")
    parser.add_argument("--save-token", default=None, metavar="TOKEN", help="将 token 保存到 ~/.workbuddy/.neodata_token")

    args = parser.parse_args()

    # --save-token 模式：保存后退出
    if args.save_token:
        _save_token_file(args.save_token)
        print(f"Token 已保存到 {TOKEN_FILE}")
        return

    if not args.query:
        parser.error("--query 或 --save-token 必须提供其一")

    try:
        result = query_neodata(
            query=args.query,
            data_type=args.data_type,
            token=args.token,
        )
    except requests.RequestException as e:
        print(f"请求失败: {e}", file=sys.stderr)
        sys.exit(1)

    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
