#!/usr/bin/env bash
# 供应链安全审查 v2（bundled 技能 audit-third-party，基于 skill-vetter 的 RED FLAGS 协议）
#
# v2 改进：
#  - 排除测试/文档/示例目录与文件（消除「安全扫描器测试数据」「文档提及」产生的
#    误报）：*.test.*  *.spec.*  /tests  /test  /docs  /doc  /examples
#    node_modules  .git
#  - 收紧正则：eval( 排除 evaluate(/.exec() 方法；exec( 排除 .exec() 方法调用；
#    ssh/aws 仅匹配真实路径引用而非扫描器正则定义。
#  - 仅做只读静态分析，不执行任何仓库代码。
#
# 用法: bash audit-third-party.sh [<third_party 目录>]   （默认 ./third_party，相对当前目录）
set +e
# 这里曾写死一个作者本机的绝对路径。third_party/ 本身是被 .gitignore 整个忽略的
# （不随仓库分发），所以任何写死的绝对路径都只对作者成立。
TP="${1:-third_party}"
if [ ! -d "$TP/harness" ] && [ ! -d "$TP/computer_use" ]; then
  echo "[FAIL] 找不到 $TP/{harness,computer_use} —— 请显式传入 third_party 目录。" >&2
  echo "       用法: bash audit-third-party.sh <third_party 目录>" >&2
  exit 2
fi
OUT="$TP/security_audit_report.md"
: > "$OUT"

# ---- 实时进度看板状态输出（供 pipeline_dashboard.html 读取）----
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
STATUS_JSON="$TP/audit_status.json"
TOTAL=$(find "$TP/harness" "$TP/computer_use" -maxdepth 1 -mindepth 1 -type d 2>/dev/null | grep -v '/_garbage' | wc -l)
DONE=0
emit_status() {
  cat > "$STATUS_JSON" <<JSON
{
  "skill": "audit-third-party",
  "started_at": "$STARTED_AT",
  "updated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "total": $TOTAL,
  "done": $1,
  "current": "$2",
  "finished": $3,
  "report": "$OUT"
}
JSON
}
emit_status 0 "" false

cat >> "$OUT" <<'EOF'
# 第三方仓库供应链安全审查报告 v2（skill-vetter 协议适配）

> 适配自 skill-vetter 的 RED FLAGS 协议，对克隆落地的社区仓库做静态安全审查。
> **v2 已排除测试/文档/示例文件以消除误报**（初版把「安全扫描器测试数据」「文档提及」
> 误判为危险信号）。仅做只读静态分析，不执行任何仓库代码。
> 风险分级：LOW（基础审查即可）/ MEDIUM（需完整代码审查）/ HIGH（需人工批准）/ EXTREME（禁止集成）。

EOF
echo "生成时间: $(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$OUT"
echo "" >> "$OUT"

# 排除项（测试/文档/示例/依赖/元）
EX_DIR="--exclude-dir=node_modules --exclude-dir=.git --exclude-dir=tests --exclude-dir=test --exclude-dir=docs --exclude-dir=doc --exclude-dir=examples --exclude-dir=dist --exclude-dir=build"
EX_FILE="--exclude=*.test.* --exclude=*.spec.* --exclude=*.min.js --exclude=*.min.css --exclude-dir=.next --exclude-dir=__pycache__"

# 危险信号模式（已收紧）： tag|regex|severity
PATTERNS=(
  "PIPE-TO-SHELL|curl[[:space:]]+[^'\" ]*\|[[:space:]]*(sh|bash)|EXTREME"
  "PIPE-TO-SHELL|wget[[:space:]]+[^'\" ]*\|[[:space:]]*(sh|bash)|EXTREME"
  "PIPE-TO-SHELL|curl[[:space:]]+[^'\" ]*\|[[:space:]]*(python|perl)|HIGH"
  "EVAL-DIRECT|(^|[^.[:alnum:]])eval[[:space:]]*\(|HIGH"
  "EVAL-BASE64|eval[[:space:]]*\([^)]*base64|EXTREME"
  "OS-SYSTEM|os\.system[[:space:]]*\(|HIGH"
  "SUBPROCESS-SHELL|subprocess[^;]*shell[[:space:]]*=[[:space:]]*True|HIGH"
  "EXEC-EXTERNAL|(^|[^.[:alnum:]])exec[[:space:]]*\(|HIGH"
  "BASE64-DECODE|base64[^;]*b64decode|MEDIUM"
  "PICKLE-RCE|pickle\.load|HIGH"
  "YAML-UNSAFE|yaml\.load[[:space:]]*\(|MEDIUM"
  "IMPORT-DYN|__import__[[:space:]]*\(|MEDIUM"
  "DYNAMIC-CODE|new[[:space:]]+Function[[:space:]]*\(|HIGH"
  "CRED-DIR|(^|[^[:alnum:]\\])\.ssh/|(^|[^[:alnum:]\\])\.aws/|HIGH"
  "NET-RAW|http://[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}|HIGH"
  "SUDO|(^|[^[:alnum:]])sudo[[:space:]]|HIGH"
  "SYS-MODIFY|/etc/passwd|/System/Library|HKLM\\\\|HIGH"
  "INSTALL-UNLISTED|npm[[:space:]]+install[[:space:]]+[^ ]*(git\+|https?://)|MEDIUM"
  "INSTALL-UNLISTED|pip[[:space:]]+install[[:space:]]+[^ ]*(git\+|https?://)|MEDIUM"
)

scan_repo() {
  local d="$1" name="$2"
  local worst="LOW"
  echo "" >> "$OUT"
  echo "## $name" >> "$OUT"
  echo "" >> "$OUT"
  lic="(未声明)"
  for l in LICENSE LICENSE.md LICENSE.txt COPYING; do [ -f "$d/$l" ] && { lic=$(head -c 80 "$d/$l" | tr '\n' ' '); break; }; done
  echo "- 许可证: $lic" >> "$OUT"
  echo "- 危险信号命中（已排除测试/文档，按严重度）:" >> "$OUT"
  local any=0
  for p in "${PATTERNS[@]}"; do
    tag="${p%%|*}"; rest="${p#*|*}"; rx="${rest%|*}"; sev="${rest##*|}"
    hits=$(grep -rIE $EX_DIR $EX_FILE --include="*.py" --include="*.js" --include="*.ts" --include="*.tsx" --include="*.jsx" --include="*.rs" --include="*.go" --include="*.sh" -e "$rx" "$d" 2>/dev/null | head -4)
    cnt=$(printf '%s\n' "$hits" | grep -c .)
    if [ "$cnt" -gt 0 ]; then
      any=1
      case "$sev" in
        EXTREME) worst="EXTREME";;
        HIGH) [ "$worst" != "EXTREME" ] && worst="HIGH";;
        MEDIUM) [ "$worst" = "LOW" ] && worst="MEDIUM";;
      esac
      echo "  - [$sev] $tag ($cnt 处):" >> "$OUT"
      printf '%s\n' "$hits" | sed 's#^#      #' | head -4 >> "$OUT"
    fi
  done
  if [ "$any" -eq 0 ]; then
    echo "  - 无非测试/文档源码危险信号" >> "$OUT"
  fi
  echo "- **风险定级: $worst**" >> "$OUT"
  case "$worst" in
    EXTREME) echo "- 处置: **禁止集成**，需人工复核源码" >> "$OUT";;
    HIGH) echo "- 处置: 集成前需人工批准 + 完整代码审查" >> "$OUT";;
    MEDIUM) echo "- 处置: 完整代码审查后集成" >> "$OUT";;
    LOW) echo "- 处置: 基础审查通过，可集成" >> "$OUT";;
  esac
}

for cat in harness computer_use; do
  for d in "$TP/$cat"/*/; do
    [ -d "$d" ] || continue
    name=$(basename "$d")
    case "$name" in _*) continue;; esac
    scan_repo "$d" "$cat/$name"
    DONE=$((DONE+1)); emit_status $DONE "$cat/$name" false
  done
done

echo "" >> "$OUT"
echo "## 审查说明" >> "$OUT"
echo "- 本审查为**静态模式匹配**，v2 已排除测试/文档/示例以消除「安全扫描器测试数据」「文档提及」类误报。" >> "$OUT"
echo "- 仍存在的命中需人工复核确认是否真为危险逻辑（而非扫描器定义/合法 SDK 调用）。" >> "$OUT"
echo "- 许可证字段用于合规判断；缺少 LICENSE 的仓库集成前需确认授权。" >> "$OUT"
echo "审查完成: $OUT"
emit_status $TOTAL "all" true
