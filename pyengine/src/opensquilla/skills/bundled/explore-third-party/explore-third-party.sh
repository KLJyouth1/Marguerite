#!/usr/bin/env bash
# 统一勘探脚本（bundled 技能 explore-third-party）：克隆完成后对 <target>/{harness,computer_use}
# 逐仓分析，产出可吸收能力清单（语言/构建系统/入口/核心模式/供应链风险）。
# 仅做只读侦察，不修改任何上游代码。
#
# 用法: bash explore-third-party.sh [<third_party 目录>]   （默认 /d/Downloads/Marguerite/third_party）
set +e
TP="${1:-/d/Downloads/Marguerite/third_party}"
OUT="$TP/exploration_report.md"
: > "$OUT"

# ---- 实时进度看板状态输出（供 pipeline_dashboard.html 读取）----
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
STATUS_JSON="$TP/explore_status.json"
TOTAL=$(find "$TP/harness" "$TP/computer_use" -maxdepth 1 -mindepth 1 -type d 2>/dev/null | grep -v '/_garbage' | wc -l)
DONE=0
emit_status() {
  cat > "$STATUS_JSON" <<JSON
{
  "skill": "explore-third-party",
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
# Third-Party 统一勘探报告

> 自动生成于克隆完成后。逐仓侦察语言、构建系统、入口、可吸收能力模式与供应链风险。
> 本脚本只读探查，不修改上游代码。

EOF

echo "生成时间: $(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$OUT"
echo "" >> "$OUT"

# ---------- 工具函数 ----------
count_ext() { find "$1" -type f -name "$2" 2>/dev/null | wc -l; }

detect_lang() {
  local d="$1" py=0 ts=0 js=0 rs=0 go=0 cpp=0 java=0 sh=0 rust=0
  py=$(find "$d" -type f \( -name "*.py" -o -name "*.pyi" \) 2>/dev/null | wc -l)
  ts=$(find "$d" -type f \( -name "*.ts" -o -name "*.tsx" -o -name "*.js" -o -name "*.jsx" \) 2>/dev/null | wc -l)
  rs=$(find "$d" -type f -name "*.rs" 2>/dev/null | wc -l)
  go=$(find "$d" -type f -name "*.go" 2>/dev/null | wc -l)
  cpp=$(find "$d" -type f \( -name "*.cpp" -o -name "*.cc" -o -name "*.h" -o -name "*.hpp" \) 2>/dev/null | wc -l)
  java=$(find "$d" -type f -name "*.java" 2>/dev/null | wc -l)
  sh=$(find "$d" -type f \( -name "*.sh" -o -name "*.bash" \) 2>/dev/null | wc -l)
  local best="unknown" bestn=0
  [ "$py"  -gt "$bestn" ] && { best="Python";  bestn=$py; }
  [ "$ts"  -gt "$bestn" ] && { best="TS/JS";   bestn=$ts; }
  [ "$rs"  -gt "$bestn" ] && { best="Rust";    bestn=$rs; }
  [ "$go"  -gt "$bestn" ] && { best="Go";      bestn=$go; }
  [ "$cpp" -gt "$bestn" ] && { best="C/C++";   bestn=$cpp; }
  [ "$java" -gt "$bestn" ] && { best="Java";  bestn=$java; }
  printf "%s(%d)" "$best" "$bestn"
}

detect_build() {
  local d="$1" b=""
  [ -f "$d/Cargo.toml" ]   && b="${b}Cargo "
  [ -f "$d/package.json" ] && b="${b}npm/pnpm "
  [ -f "$d/pyproject.toml" ] && b="${b}poetry/uv "
  [ -f "$d/setup.py" ]     && b="${b}setuptools "
  [ -f "$d/requirements.txt" ] && b="${b}pip "
  [ -f "$d/go.mod" ]       && b="${b}go-mod "
  [ -f "$d/pom.xml" ]      && b="${b}maven "
  [ -f "$d/build.gradle" ] && b="${b}gradle "
  [ -f "$d/Makefile" ]     && b="${b}make "
  [ -f "$d/CMakeLists.txt" ] && b="${b}cmake "
  [ -z "$b" ] && b="(none)"
  printf "%s" "$b"
}

# 在某目录树下统计关键词命中（用于模式识别）
kw() { local d="$1" pat="$2"; grep -rIl --include="*.py" --include="*.ts" --include="*.tsx" --include="*.js" --include="*.rs" --include="*.go" -e "$pat" "$d" 2>/dev/null | wc -l; }

total_files() { find "$1" -type f 2>/dev/null | wc -l; }

# ---------- 主流程 ----------
for cat in harness computer_use; do
  echo "" >> "$OUT"
  echo "## 分类：$cat" >> "$OUT"
  echo "" >> "$OUT"
  echo "| 仓库 | 主语言 | 构建系统 | 文件数 | 顶层结构 |" >> "$OUT"
  echo "|------|--------|----------|--------|----------|" >> "$OUT"
  for d in "$TP/$cat"/*/; do
    [ -d "$d" ] || continue
    name=$(basename "$d")
    [ "$name" = "_garbage_"* ] && continue
    lang=$(detect_lang "$d")
    build=$(detect_build "$d")
    nf=$(total_files "$d")
    top=$(ls -1 "$d" 2>/dev/null | grep -v '^\.git$' | head -12 | paste -sd' ' -)
    printf "| %s | %s | %s | %s | %s |\n" "$name" "$lang" "$build" "$nf" "$top"  >> "$OUT"
  done
done

# ---------- 模式深掘 ----------
echo "" >> "$OUT"
echo "## 可吸收能力模式（按分类）" >> "$OUT"
echo "" >> "$OUT"

echo "### Harness 类（agent / subagent / tool / plugin / 路由 / 检查点 / 评测）" >> "$OUT"
echo "" >> "$OUT"
for d in "$TP/harness"/*/; do
  [ -d "$d" ] || continue
  name=$(basename "$d")
  [ "$name" = "_garbage_"* ] && continue
  echo "#### $name" >> "$OUT"
  echo "- subagent/agent 框架: $(kw "$d" 'subagent\|class Agent\|agent loop\|AgentExecutor') 文件" >> "$OUT"
  echo "- 生命周期钩子: $(kw "$d" 'lifecycle\|on_start\|hook\|pre_step\|post_step') 文件" >> "$OUT"
  echo "- 动态路由: $(kw "$d" 'router\|route(\|dispatch\|dynamic.*rout') 文件" >> "$OUT"
  echo "- git 检查点: $(kw "$d" 'git checkpoint\|git stash\|snapshot\|checkpoint') 文件" >> "$OUT"
  echo "- 工具/插件系统: $(kw "$d" 'register_tool\|@tool\|plugin\|toolbox\|function_call') 文件" >> "$OUT"
  echo "- 提示词/上下文管理: $(kw "$d" 'system_prompt\|context_window\|token.*budget\|prompt template') 文件" >> "$OUT"
  echo "- 评测/打分: $(kw "$d" 'eval\|grade\|benchmark\|score') 文件" >> "$OUT"
  # 许可证
  lic="(未声明)"
  for l in LICENSE LICENSE.md LICENSE.txt COPYING; do [ -f "$d/$l" ] && { lic=$(head -c 120 "$d/$l" | tr '\n' ' '); break; }; done
  echo "- 许可证: $lic" >> "$OUT"
  echo "" >> "$OUT"
  DONE=$((DONE+1)); emit_status $DONE "$name" false
done

echo "### Computer Use 类（屏幕感知 / 键鼠控制 / GUI 定位 / 视觉 grounding）" >> "$OUT"
echo "" >> "$OUT"
for d in "$TP/computer_use"/*/; do
  [ -d "$d" ] || continue
  name=$(basename "$d")
  [ "$name" = "_garbage_"* ] && continue
  echo "#### $name" >> "$OUT"
  echo "- 屏幕捕获: $(kw "$d" 'screenshot\|screen_capture\|mss\|pyautogui.screenshot\|capture_screen') 文件" >> "$OUT"
  echo "- 键鼠控制: $(kw "$d" 'pyautogui\|mouse.click\|keyboard.press\|pynput\|SendInput\|xdotool') 文件" >> "$OUT"
  echo "- GUI 元素定位: $(kw "$d" 'accessibility\|UIA\|element locate\|dom\|selector\|bounding box') 文件" >> "$OUT"
  echo "- 视觉 grounding: $(kw "$d" 'grounding\|yolo\|detect\|ocr\|opencv\|segmentation\|bbox') 文件" >> "$OUT"
  echo "- 视觉模型集成: $(kw "$d" 'gpt-4v\|qwen-vl\|clip\|vit\|llava\|vision model\|multimodal') 文件" >> "$OUT"
  lic="(未声明)"
  for l in LICENSE LICENSE.md LICENSE.txt COPYING; do [ -f "$d/$l" ] && { lic=$(head -c 120 "$d/$l" | tr '\n' ' '); break; }; done
  echo "- 许可证: $lic" >> "$OUT"
  echo "" >> "$OUT"
  DONE=$((DONE+1)); emit_status $DONE "$name" false
done

echo "## 后续动作" >> "$OUT"
echo "- 依据本报告的「可吸收能力模式」列，将 harness 模式映射到 core/src/harness 与 agent 运行时。" >> "$OUT"
echo "- 依据 computer_use 模式，重构 tools/computer_use 的屏幕感知 / 键鼠控制 / GUI 定位三层。" >> "$OUT"
echo "- 对命中外部 API/重依赖的仓库做 supply-chain 风险复核（audit-third-party 技能）。" >> "$OUT"

echo "勘探报告已写入: $OUT"
emit_status $TOTAL "all" true
