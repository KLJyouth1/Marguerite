#!/bin/bash
# Setup script for sino-drug-instructions-search Skill（依赖 sinohealth_skills_sdk）
# bizType 与 SKILLS_BIZ_TYPE 由脚本与 SkillsClient 对齐；bizToken 仅来自环境变量 SKILLS_BIZ_TOKEN（不写文件）

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKILL_VERSION="v1.0.0"

echo "🚀 设置药品说明书检索 Skill..."
echo "📦 当前版本: $SKILL_VERSION"
echo "📌 SKILLS_BIZ_TYPE 由脚本自动设置（与 SkillsClient 的 biz_type 一致，无需手动 export）"
echo "📌 SKILLS_BIZ_TOKEN 须由环境注入（禁止写入本地 token 文件）"
echo ""

if ! command -v python3 &> /dev/null; then
    echo "❌ 错误：未找到 python3，请先安装 Python 3.8+"
    exit 1
fi

if command -v pip3 &> /dev/null; then
    PIP="pip3"
else
    PIP="python3 -m pip"
fi

echo "📦 安装/升级 PyPI 包 sinohealth_skills_sdk 至最新版..."
$PIP install -U -q sinohealth_skills_sdk

if ! python3 -c "from sinohealth_skills_sdk import SkillsClient" 2>/dev/null; then
    echo "❌ sinohealth_skills_sdk 导入失败，请手动执行:"
    echo "  pip3 install -U sinohealth_skills_sdk"
    exit 1
fi
echo "✅ sinohealth_skills_sdk 模块可用"
echo ""

echo "🔍 检查环境变量 SKILLS_BIZ_TOKEN..."
if [ -z "$SKILLS_BIZ_TOKEN" ]; then
    echo "❌ 错误：未设置 SKILLS_BIZ_TOKEN。"
    echo "   请由宿主或 Agent 注入 JWT（例如 connect_cloud_service 返回的 token），例如："
    echo "   export SKILLS_BIZ_TOKEN=\"<JWT>\""
    echo "   不要将 token 写入磁盘文件。详见 SKILL.md「鉴权」。"
    exit 1
fi
echo "✅ SKILLS_BIZ_TOKEN 已设置"
echo ""

echo "🧪 验证 SkillsClient（SKILLS_BIZ_TYPE 与 biz_type 与脚本一致）..."
if ! python3 -c "
import os
from sinohealth_skills_sdk import SkillsClient
os.environ['SKILLS_BIZ_TYPE'] = 'workbuddy'
tok = (os.getenv('SKILLS_BIZ_TOKEN') or '').strip()
if not tok:
    raise SystemExit(1)
c = SkillsClient(biz_type='workbuddy', biz_token=tok, stream=False)
c.close()
" 2>/dev/null; then
    echo "❌ SkillsClient 初始化失败，请检查 SKILLS_BIZ_TOKEN 是否有效"
    exit 1
fi
echo "✅ SkillsClient 初始化成功"
echo ""

echo "🧪 验证检索脚本可加载..."
if ! python3 "$SCRIPT_DIR/scripts/drug_instructions.py" --help &>/dev/null; then
    echo "⚠️  脚本 --help 不可用，请检查 scripts/drug_instructions.py"
else
    echo "✅ scripts/drug_instructions.py 可执行"
fi

echo ""
echo "─────────────────────────────────────"
echo "🎉 设置完成！"
echo ""
echo "执行查询前请确保当前环境已 export SKILLS_BIZ_TOKEN，例如:"
echo "  python3 \"$SCRIPT_DIR/scripts/drug_instructions.py\" \"阿司匹林肠溶片禁忌症\""
