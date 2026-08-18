#!/usr/bin/env bash
# NeoData 金融数据查询 - curl 封装（通过代理 API）
#
# Usage:
#   bash query.sh "腾讯最新财报"
#   bash query.sh --token "<JWT>" "贵州茅台股价"
#   bash query.sh --save-token "<JWT>"
#
# 鉴权优先级: --token 参数 > ~/.workbuddy/.neodata_token 文件
#
# 环境变量 (可选):
#   NEODATA_ENDPOINT  - 代理 URL (可选，默认 https://copilot.tencent.com/agenttool/v1/neodata)
#   NEODATA_DATA_TYPE - 数据类型 all/api/doc (可选，默认不传由代理填充)

set -euo pipefail

DEFAULT_ENDPOINT="https://copilot.tencent.com/agenttool/v1/neodata"
TOKEN_FILE="$HOME/.workbuddy/.neodata_token"

# 解析参数
CLI_TOKEN=""
SAVE_TOKEN=""
QUERY=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --token)
            CLI_TOKEN="$2"
            shift 2
            ;;
        --save-token)
            SAVE_TOKEN="$2"
            shift 2
            ;;
        *)
            QUERY="$1"
            shift
            ;;
    esac
done

# --save-token 模式：保存后退出
if [[ -n "$SAVE_TOKEN" ]]; then
    mkdir -p "$(dirname "$TOKEN_FILE")"
    printf '%s' "$SAVE_TOKEN" > "$TOKEN_FILE"
    chmod 600 "$TOKEN_FILE"
    echo "Token 已保存到 $TOKEN_FILE"
    exit 0
fi

if [[ -z "$QUERY" ]]; then
    echo "用法: bash query.sh [--token <JWT>] <query>" >&2
    echo "      bash query.sh --save-token <JWT>" >&2
    exit 1
fi

ENDPOINT="${NEODATA_ENDPOINT:-$DEFAULT_ENDPOINT}"
DATA_TYPE="${NEODATA_DATA_TYPE:-}"

# Token 优先级: --token 参数 > 文件
TOKEN="$CLI_TOKEN"
if [[ -z "$TOKEN" && -f "$TOKEN_FILE" ]]; then
    TOKEN="$(cat "$TOKEN_FILE")"
fi

if [[ -z "$TOKEN" ]]; then
    echo "错误: 未找到 token。请先运行 --save-token 保存，或使用 --token 参数传入" >&2
    exit 1
fi

# 构建请求体，channel 和 sub_channel 为固定字段
if [[ -n "$DATA_TYPE" ]]; then
    BODY=$(printf '{"query": "%s", "channel": "neodata", "sub_channel": "workbuddy", "data_type": "%s"}' "$QUERY" "$DATA_TYPE")
else
    BODY=$(printf '{"query": "%s", "channel": "neodata", "sub_channel": "workbuddy"}' "$QUERY")
fi

RESPONSE=$(curl --silent --show-error --location --max-time 30 --connect-timeout 10 \
    --write-out "\n%{http_code}" \
    "${ENDPOINT}" \
    --header "Content-Type: application/json" \
    --header "Authorization: Bearer ${TOKEN}" \
    --data "$BODY")

HTTP_CODE=$(echo "$RESPONSE" | tail -1)
BODY_RESP=$(echo "$RESPONSE" | sed '$d')

if [[ "$HTTP_CODE" -ne 200 ]]; then
    echo "请求失败: HTTP ${HTTP_CODE}" >&2
    [[ -n "$BODY_RESP" ]] && echo "$BODY_RESP" >&2
    exit 1
fi

echo "$BODY_RESP" | python3 -m json.tool 2>/dev/null || echo "$BODY_RESP"
