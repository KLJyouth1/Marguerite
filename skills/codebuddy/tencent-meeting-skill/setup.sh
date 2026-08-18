#!/bin/bash
# Setup script for 腾讯会议 MCP Skill

set -e

# 版本号定义
SKILL_VERSION="v1.0.5"

echo "🚀 设置腾讯会议 MCP Skill..."
echo "📦 当前版本: $SKILL_VERSION"
echo ""

# 检查 mcporter
if ! command -v mcporter &> /dev/null; then
    echo "⚠️  未找到 mcporter，正在安装..."
    npm install -g mcporter
    echo "✅ mcporter 安装完成"
fi

# 新增：检查 TENCENT_MEETING_TOKEN 环境变量
echo "🔍 检查腾讯会议 Token 环境变量..."
if [ -z "$TENCENT_MEETING_TOKEN" ]; then
    echo "❌ 错误：未检测到 TENCENT_MEETING_TOKEN 环境变量！"
    echo "请先执行以下命令设置环境变量（替换为真实 Token）："
    echo "  export TENCENT_MEETING_TOKEN=\"your_actual_token_here\""
    echo "或在执行脚本时直接传入："
    echo "  TENCENT_MEETING_TOKEN=\"your_actual_token_here\" bash this_script.sh"
    exit 1  # 退出脚本，避免后续无效操作
else
    echo "✅ TENCENT_MEETING_TOKEN 环境变量已配置"
fi
echo ""

# 添加 MCP 配置
echo "🔧 配置 mcporter..."

# 从环境变量中读取用户填写的 Token
mcporter config add tencent-meeting-mcp https://mcp.meeting.tencent.com/mcp/wemeet-open/v1 \
    --header "X-Tencent-Meeting-Token=$TENCENT_MEETING_TOKEN" \
    --header "X-Skill-Version=$SKILL_VERSION" \
    --transport http \
    --scope project

# 检查 mcporter config add 命令的返回值
if [ $? -ne 0 ]; then
    echo "❌ mcporter 配置添加失败！"
    echo "可能的原因："
    echo "  - 网络连接问题"
    echo "  - mcporter 版本不兼容"
    echo "  - 传入的配置参数有误"
    echo ""
    echo "请检查上述问题后重试。"
    exit 1 # 退出脚本
else
    echo "✅ 配置完成！"
fi
echo ""

# 验证配置
echo "🧪 验证配置..."
LIST_OUTPUT=$(mcporter list 2>&1)

# 检查条件：必须包含"tencent-meeting-mcp"，并且不能包含"auth required"
if echo "$LIST_OUTPUT" | grep -q "tencent-meeting-mcp" && ! echo "$LIST_OUTPUT" | grep -q "0 healthy"; then
    echo "✅ 配置验证成功！"
    echo ""
    echo "$LIST_OUTPUT" | grep -A 1 "tencent-meeting-mcp" || true
else
    echo "⚠️  配置验证失败！"
    echo "$LIST_OUTPUT"
    echo "请检查以下可能的原因："
    echo "  1. 网络无法访问 MCP 服务 URL"
    echo "  2. Token 无效或已过期"
    echo "  3. mcporter 安装不完整"
    echo ""
    echo "调试命令："
    echo "  mcporter list | grep tencent-meeting-mcp"
    echo "预期展示：tencent-meeting-mcp (xx tools, 0.3s)"
    exit 1
fi

echo ""
echo "─────────────────────────────────────"
echo "🎉 设置完成！"