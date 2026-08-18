"""
药品说明书与用药信息检索 - 调用脚本（依赖 sinohealth_skills_sdk）

- SKILLS_BIZ_TYPE：与 `SkillsClient` 的 `biz_type` 由脚本对齐并写入当前进程环境
- bizToken：仅从系统环境变量 SKILLS_BIZ_TOKEN 读取，不落盘

用法:
    export SKILLS_BIZ_TOKEN="<JWT>"
    python drug_instructions.py "<用户的问题>"
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from typing import Any

from skills_sdk import SkillsClient

# bizType / SKILLS_BIZ_TYPE 使用同一取值（对应环境变量名 SKILLS_BIZ_TYPE）
_BIZ_TYPE = "workbuddy"
_ENV_TYPE = "SKILLS_BIZ_TYPE"
_ENV_TOKEN = "SKILLS_BIZ_TOKEN"

# 网关 /skill/{url_suffix} 路径，与线上一致时无需修改
_SKILL_URL = "/dataset/drug"


def _format_output(data: Any) -> str:
    if isinstance(data, dict) and "choices" in data:
        choices = data["choices"]
        if choices and isinstance(choices[0], dict):
            content = choices[0].get("message", {}).get("content", "")
            if content:
                try:
                    inner = json.loads(content)
                    if isinstance(inner, dict):
                        t = inner.get("text")
                        if isinstance(t, str) and t.strip():
                            return t
                except (json.JSONDecodeError, TypeError):
                    pass
                return content
    if isinstance(data, dict):
        text = data.get("text")
        if isinstance(text, str) and text.strip():
            return text
    if isinstance(data, str) and data.strip():
        return data
    return json.dumps(data, ensure_ascii=False, indent=2)


def main() -> None:
    try:
        from pathlib import Path as _PathSkillRoot
        _skill_root = _PathSkillRoot(__file__).resolve().parent.parent.parent
        if str(_skill_root) not in sys.path:
            sys.path.insert(0, str(_skill_root))
        from skill_stdio_utf8 import ensure_utf8_stdio
        ensure_utf8_stdio()
    except ImportError:
        pass

    parser = argparse.ArgumentParser(description="药品说明书与用药信息检索")
    parser.add_argument("prompt", help="用户的问题")
    args = parser.parse_args()
    prompt = (args.prompt or "").strip()
    if not prompt:
        print("错误: 问题不能为空", file=sys.stderr)
        sys.exit(2)

    # 与 SkillsClient(biz_type=...) 对齐
    os.environ[_ENV_TYPE] = _BIZ_TYPE

    token = (os.getenv(_ENV_TOKEN) or "").strip()
    if not token:
        print(
            f"错误: 未设置环境变量 {_ENV_TOKEN}。\n"
            f"{_ENV_TYPE} 由脚本自动处理，无需手动配置。",
            file=sys.stderr,
        )
        sys.exit(1)

    client = SkillsClient(biz_type=_BIZ_TYPE, biz_token=token, stream=False)
    try:
        response = client.call_skill(
            _SKILL_URL,
            {
                "prompt": prompt,
                "msgId": "debug",
                "medBase": "药品",
            }
        )
        if response.success:
            print(_format_output(response.data))
        else:
            err = response.error or "调用失败"
            print(err, file=sys.stderr)
            low = str(err).lower()
            if any(x in low for x in ("401", "403", "认证", "鉴权", "token", "unauthorized")):
                print(
                    f"提示: 若 token 已过期，请更新环境变量 {_ENV_TOKEN}。",
                    file=sys.stderr,
                )
            sys.exit(1)
    finally:
        client.close()


if __name__ == "__main__":
    main()
