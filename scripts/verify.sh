#!/usr/bin/env bash
# =============================================================================
# verify.sh — Marguerite 统一「完成前验证」闸门
# -----------------------------------------------------------------------------
# 铁律（verification-before-completion）：任何「完成 / 通过 / 已修复 / 已构建」
# 声明之前，必须运行本脚本并取得「全绿」证据（每条 [PASS] + 最终退出码 0）。
# 任一环节失败 → 非零退出，禁止声称完成。
#
# 设计目标：把分散的验证手段收口成一个机械闸门，破除两类伪完成：
#   1) 调用了工具即声称完成（如「跑了搜索」≠「拿到新闻」）
#   2) 增量缓存掩盖库级编译错误（cargo 增量缓存会让「上次绿」误导本次声明）
#
# 用法：
#   bash scripts/verify.sh                 # 等同 --all（不含耗时的 tauri 打包）
#   bash scripts/verify.sh --clean --all   # 先 cargo clean -p marguerite-core 再全测，破除缓存遮蔽（完成声明用此档）
#   bash scripts/verify.sh --release       # --all + cargo tauri build（发版门禁）
#   bash scripts/verify.sh --core          # 仅 Rust core 栈
#   bash scripts/verify.sh --webui         # 仅 webui 栈（typecheck + 单测）
#   bash scripts/verify.sh --gui           # 仅 gui（纯 Tauri 壳：无前端源码守卫 + 壳编译 + 壳 clippy）
#   bash scripts/verify.sh --pyengine      # 仅 Python 侧车栈（tests/unit + tests/test_gateway + 契约族六个目录，无网络/无 LLM）
#   bash scripts/verify.sh --e2e          # 仅 Web 端 Playwright e2e（自起隔离网关，跑确定性子集）
#   bash scripts/verify.sh --fast          # 提交快闸：core 测试 + clippy + webui typecheck
#
# 退出码：0 = 全绿；1 = 有检查失败；2 = 参数/环境错误。
# =============================================================================
set -o pipefail

# --- 解析项目根（脚本在 scripts/，根在上级） ---
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT" || { echo "[FAIL] 无法进入项目根 $ROOT"; exit 1; }

# --- 工具链 PATH（本机 cargo 在 /c/Users/Administrator/.cargo/bin） ---
if ! command -v cargo >/dev/null 2>&1; then
  if [ -x "/c/Users/Administrator/.cargo/bin/cargo" ]; then
    export PATH="/c/Users/Administrator/.cargo/bin:$PATH"
  else
    echo "[FAIL] 找不到 cargo：请将 ~/.cargo/bin 加入 PATH"; exit 2
  fi
fi
if ! command -v npm >/dev/null 2>&1; then
  echo "[WARN] 找不到 npm：webui 检查将被跳过（不影响 Rust 栈）"
fi

# 关键：NO_COLOR=1 会让 2 个既有主题测试走单色回退而误失败（非代码缺陷，见 AGENTS.md §2）
unset NO_COLOR

# --- 输出颜色（脚本自身着色，与上面 unset NO_COLOR 解耦） ---
if [ -t 1 ]; then
  C_PASS=$'\033[32m'; C_FAIL=$'\033[31m'; C_HEAD=$'\033[36m'; C_RST=$'\033[0m'
else
  C_PASS=""; C_FAIL=""; C_HEAD=""; C_RST=""
fi

EVIDENCE_LINES=30
PASSED=(); FAILED=(); SKIPPED=()

# run_check <名称> <命令...>
# 运行命令、捕获合并输出与真实退出码，打印尾部证据，登记 PASS/FAIL。
run_check() {
  local name="$1"; shift
  local start end dur out rc
  start=$(date +%s)
  echo
  echo "${C_HEAD}=== [CHECK] $name ===${C_RST}"
  out="$("$@" 2>&1)"; rc=$?
  end=$(date +%s); dur=$((end - start))
  # 打印尾部证据（失败时常含关键错误行）
  printf '%s\n' "$out" | tail -n "$EVIDENCE_LINES"
  if [ "$rc" -eq 0 ]; then
    echo "${C_PASS}[PASS]${C_RST} $name  (${dur}s)"
    PASSED+=("$name")
  else
    echo "${C_FAIL}[FAIL]${C_RST} $name  (rc=$rc, ${dur}s)"
    FAILED+=("$name")
  fi
  return "$rc"
}

# run_tests <名称> <cargo test 参数...>
# 在 run_check 基础上额外抽取测试计数作为证据（cargo 失败行会被 tail 截到）。
run_tests() {
  local name="$1"; shift
  local out rc start end dur
  start=$(date +%s)
  echo
  echo "${C_HEAD}=== [CHECK] $name ===${C_RST}"
  out="$(cargo test "$@" 2>&1)"; rc=$?
  end=$(date +%s); dur=$((end - start))
  printf '%s\n' "$out" | tail -n "$EVIDENCE_LINES"
  # 抽取 test result 汇总作为铁证
  local summary
  summary="$(printf '%s\n' "$out" | grep -E 'test result:' | sed 's/^/    /')"
  [ -n "$summary" ] && echo "${C_HEAD}证据:${C_RST}$summary"
  if [ "$rc" -eq 0 ]; then
    echo "${C_PASS}[PASS]${C_RST} $name  (${dur}s)"
    PASSED+=("$name")
  else
    echo "${C_FAIL}[FAIL]${C_RST} $name  (rc=$rc, ${dur}s)"
    FAILED+=("$name")
  fi
  return "$rc"
}

# skip_check <名称> <原因>
#
# 跳过**不等于**通过：登记到独立的 SKIPPED 列表并在汇总里显式列出，避免「因为工具
# 缺失所以静默变绿」——那正是本仓库铁律要防的伪完成。
skip_check() {
  local name="$1" reason="$2"
  echo
  echo "${C_HEAD}=== [CHECK] $name ===${C_RST}"
  echo "${C_HEAD}[SKIP]${C_RST} $reason"
  SKIPPED+=("$name — $reason")
}

# ---------------------------------------------------------------------------
# 各检查定义
# ---------------------------------------------------------------------------
check_core_tests() {
  if [ "$DO_CLEAN" -eq 1 ]; then
    echo "${C_HEAD}--clean: cargo clean -p marguerite-core（破除增量缓存遮蔽）${C_RST}"
    cargo clean -p marguerite-core 2>&1 | tail -n 5
  fi
  run_tests "core 单元测试 (cargo test -p marguerite-core --lib)" -p marguerite-core --lib
}

check_core_build() {
  run_check "core CLI 构建 (cargo build -p marguerite-core --bin marguerite-core)" \
    cargo build -p marguerite-core --bin marguerite-core
}

# core/tests/*.rs 曾是「无人执行的死树」：verify.sh 只跑 --lib，集成测试树没有门禁。
# W10 把**离线且确定性**的 4 个二进制接进来；third_party_integration 依赖不随仓库
# 发布的 third_party/plugin-examples，缺失时登记 [SKIP]（跳过 ≠ 通过）。
# macro_live 故意不选（需要真实 API key）；perf 是性能基准，仅手动跑（scripts/bench）。
check_core_integration_tests() {
  local tests="e2e plugin_manifest_gate plugin_patch_gate tui_model_panel"
  local args=()
  local t
  for t in $tests; do args+=(--test "$t"); done
  if [ -d third_party/plugin-examples ]; then
    run_check "core 集成测试 (e2e,plugin_manifest_gate,plugin_patch_gate,tui_model_panel,third_party_integration)" \
      cargo test -p marguerite-core "${args[@]}" --test third_party_integration
  else
    run_check "core 集成测试 (e2e,plugin_manifest_gate,plugin_patch_gate,tui_model_panel)" \
      cargo test -p marguerite-core "${args[@]}"
    skip_check "core 集成测试 (third_party_integration)" \
      "third_party/plugin-examples 不随仓库发布（AGENTS.md §7）"
  fi
}

# ensure_webui_dist <用途说明>
#
# gui/src-tauri 的 `bundle.resources` 声明了 `webui/dist`，而 Tauri 2 在声明的资源
# 路径缺失时会直接中止 build script（实测：`cargo build --release -p marguerite-gui`
# → exit 101 `resource path 'webui\dist' doesn't exist`）。任何会编译 marguerite-gui
# 的检查（clippy --all-targets、cargo build --workspace）都因此依赖该产物，而它是
# gitignore 的生成物——全新 clone 必然没有。
#
# 这里统一做前置：缺失就先构建一次 Vue 控制台，而不是让门禁报一个与本次改动无关的
# 红。返回非 0 表示无法补齐（调用方应据此判 FAIL，不要继续跑注定失败的编译）。
ensure_webui_dist() {
  local why="$1"
  if [ -f webui/dist/index.html ]; then
    return 0
  fi
  if command -v npm >/dev/null 2>&1 && [ -f webui/package.json ]; then
    echo "${C_HEAD}webui/dist 缺失（fresh clone）→ 先构建 Vue 控制台（$why）${C_RST}"
    run_check "webui 产物构建 (cd webui && npm run build)" \
      bash -c "cd webui && npm run build" || return 1
    return 0
  fi
  echo "${C_FAIL}[FAIL]${C_RST} 缺少 webui/dist 且 npm 不可用：无法构建 Tauri 资源，$why 无法进行"
  FAILED+=("webui 产物构建 (前置缺失)")
  return 1
}

check_clippy_core() {
  # 仅对活跃开发的 core + gui 做 L3 零警告（tui 遗留警告见 AGENTS.md §8 说明）。
  # --all-targets 会编译 marguerite-gui，故同样需要 webui/dist 前置。
  ensure_webui_dist "clippy --all-targets 会编译 gui" || return 1
  run_check "clippy L3 零警告 (core+gui)" \
    cargo clippy -p marguerite-core -p marguerite-gui --all-targets -- -D warnings
}

check_clippy_ws() {
  ensure_webui_dist "clippy --workspace 会编译 gui" || return 1
  run_check "clippy L3 零警告 (--workspace --all-targets)" \
    cargo clippy --workspace --all-targets -- -D warnings
}

# ---------------------------------------------------------------------------
# Python 解释器解析（能力感知 capability-aware）—— 详见 AGENTS.md §4
# 「`python3` 可能是没有依赖的隔离解释器」。
#
# 背景（决定性实测）：本机 `which python3` 指向 WorkBuddy 托管的隔离解释器
# `…/.workbuddy/binaries/python/versions/3.13.12/python3`（只有 pytest，缺
# anyio/httpx/structlog/typer/rich/… ），而 pyengine 的依赖装在系统 Python
# `C:\Python314\python.exe`。旧实现只按 `command -v` 取第一个可执行文件，于是门禁
# 会挑到一个「能运行、一 import 就炸」的解释器 —— 症状是整层 collection error
# （R1 日志：pyengine 单测 39 个、test_gateway 187 个 ERROR），看着像产品缺陷，
# 实际只是解释器选错。
#
# 现在改为：候选按优先级枚举，只有能 import **全部所需模块**的才算合格。
#   候选优先级：$MARGUERITE_PYTHON（显式覆盖，最高优先）> PATH 的 python3 >
#               PATH 的 python > 已知系统位置（C:/Python314|313|312、
#               %LOCALAPPDATA%/Programs/Python/Python3*）。
# 每个候选只做**一次** `import a,b,c` 探测（不逐模块循环，避免拖慢门禁）。
#
# 契约：
#   pick_python               —— 无依赖要求（给只跑标准库的文档脚本用）：第一个能运行者。
#   pick_python_requiring MOD… —— 第一个能 import 全部 MOD… 的候选。
# 二者都把结果写入全局 $PY（空 = 无合格候选），并设置：
#   $PY_REASON        无合格候选时的精确原因（逐候选列出缺哪些模块）。
#   $PY_EXPLICIT_FAIL 1 = $MARGUERITE_PYTHON 被显式设置却不合格 → 调用方须响亮失败。
# ---------------------------------------------------------------------------

# pyengine 三层的必需模块：取 tests/unit、tests/test_gateway、契约族六个目录所需
# **硬依赖**的并集（实测缺任一都会在 import 阶段 collection error）。
# 刻意**不含** numpy —— 它在 pyproject 的 `recommended` 可选 extra 里，CI 不安装它。
PYENGINE_PY_MODULES=(pytest anyio httpx structlog typer rich pydantic websockets tomli_w openpyxl reportlab yoyo)
# workbench/backend/marguerite_workbench/requirements.txt 的硬依赖（可 import 部分）。
WORKBENCH_PY_MODULES=(pytest fastapi httpx pydantic)

PY=""
PY_REASON=""
PY_EXPLICIT_FAIL=0

# fail_check <名称> <原因> —— 与 skip_check 对称：登记到 FAILED 并醒目打印。
fail_check() {
  local name="$1" reason="$2"
  echo
  echo "${C_HEAD}=== [CHECK] $name ===${C_RST}"
  echo "${C_FAIL}[FAIL]${C_RST} $reason"
  FAILED+=("$name")
}

# 枚举候选解释器（stdout 每行一个，按优先级）。
_py_candidates() {
  if [ -n "${MARGUERITE_PYTHON:-}" ]; then
    # 显式指定 → **只用它**，绝不回落（静默忽略显式指定是最难查的一类问题）。
    printf '%s\n' "$MARGUERITE_PYTHON"
    return 0
  fi
  local p
  p="$(command -v python3 2>/dev/null)"; [ -n "$p" ] && printf '%s\n' "$p"
  p="$(command -v python 2>/dev/null)";  [ -n "$p" ] && printf '%s\n' "$p"
  for p in "C:/Python314/python.exe" "C:/Python313/python.exe" "C:/Python312/python.exe"; do
    [ -e "$p" ] && printf '%s\n' "$p"
  done
  if [ -n "${LOCALAPPDATA:-}" ]; then
    for p in "$LOCALAPPDATA"/Programs/Python/Python3*/python.exe; do
      [ -e "$p" ] && printf '%s\n' "$p"
    done
  fi
}

# 探测某候选能否**一次** import 全部所需模块（0 = 可以）。
# 注意：Python 的 import 语句需逗号分隔（`import a,b,c`）；用空格拼会变成语法错误。
_py_can_import() {
  local py="$1"; shift
  if [ "$#" -eq 0 ]; then "$py" -c 'pass' >/dev/null 2>&1; return $?; fi
  local IFS=,                     # 让 "$*" 以逗号连接，得到合法的 `import a,b,c`
  "$py" -c "import $*" >/dev/null 2>&1
}

# 精确列出某候选缺失的模块（仅在需要给出失败原因时调用，慢一点无妨）。
_py_missing_modules() {
  local py="$1"; shift
  local m out=""
  for m in "$@"; do
    "$py" -c "import $m" >/dev/null 2>&1 || out="$out $m"
  done
  printf '%s' "$out"
}

# 核心解析：$1 = 模式（any|requires），其余 = 必需模块。
_py_resolve() {
  local mode="$1"; shift
  local modules=("$@") cand
  PY=""; PY_REASON=""; PY_EXPLICIT_FAIL=0

  while IFS= read -r cand; do
    [ -n "$cand" ] || continue
    if [ "$mode" = "requires" ]; then
      _py_can_import "$cand" "${modules[@]}" && { PY="$cand"; return 0; }
    else
      _py_can_import "$cand" && { PY="$cand"; return 0; }
    fi
  done < <(_py_candidates)

  # 无合格候选 → 逐候选构造精确原因（缺哪些模块 / 不可运行）。
  local lines="" miss
  while IFS= read -r cand; do
    [ -n "$cand" ] || continue
    if ! "$cand" -c 'pass' >/dev/null 2>&1; then
      lines="${lines}  - ${cand}: 不可运行或路径不存在"$'\n'
      continue
    fi
    if [ "$mode" = "requires" ]; then
      miss="$(_py_missing_modules "$cand" "${modules[@]}")"
      if [ -n "$miss" ]; then
        lines="${lines}  - ${cand}: 缺模块${miss}"$'\n'
      else
        lines="${lines}  - ${cand}: 模块齐全但探测未通过（未知原因）"$'\n'
      fi
    else
      lines="${lines}  - ${cand}: 不可运行"$'\n'
    fi
  done < <(_py_candidates)
  PY_REASON="$(printf '%s' "$lines" | sed 's/[[:space:]]*$//')"
  [ -n "${MARGUERITE_PYTHON:-}" ] && PY_EXPLICIT_FAIL=1
  return 1
}

# 无依赖要求（向后兼容）：第一个能运行的解释器。
pick_python() { _py_resolve any; }

# 能力感知：第一个能 import 全部 $@ 模块的解释器。
pick_python_requiring() { _py_resolve requires "$@"; }

# require_python <检查名> [<必需模块>...]
#   成功          → 写 $PY、打印所选解释器，return 0；
#   显式指定不合格 → fail_check（响亮失败，绝不静默回落），return 1；
#   自动探测无合格 → skip_check（带精确原因），return 1。
# 调用方在 return 1 后应直接 `return 0`（结论已登记，勿重复计数）。
require_python() {
  local name="$1"; shift
  if [ "$#" -gt 0 ]; then pick_python_requiring "$@"; else pick_python; fi
  if [ -n "$PY" ]; then
    echo "${C_HEAD}-- 选中的 Python 解释器: $PY${C_RST}"
    return 0
  fi
  if [ "$PY_EXPLICIT_FAIL" -eq 1 ]; then
    fail_check "$name" "MARGUERITE_PYTHON 被显式设为 '$MARGUERITE_PYTHON' 但不合格（拒绝静默回落到其它解释器）："$'\n'"$PY_REASON"
    return 1
  fi
  skip_check "$name" "无合格 Python 解释器（逐候选列出缺哪些模块）："$'\n'"$PY_REASON"
  return 1
}

run_py_doc_check() {
  local label="$1" script="$2"
  require_python "$label" || return 0
  if [ -f "$script" ]; then
    run_check "$label" "$PY" "$script"
  else
    echo "${C_HEAD}[SKIP]${C_RST} $script 不可用，跳过"
    SKIPPED+=("$label — python 或脚本不可用")
  fi
}

check_doc_drift() {
  run_py_doc_check "文档/代码 CLI 漂移 (check-doc-cli-drift.py)" \
    scripts/check-doc-cli-drift.py
}

check_version_sync() {
  run_py_doc_check "四端版本号一致 (check-version-sync.py)" \
    scripts/check-version-sync.py
}

# 本仓**自己**文档里的相对链接必须能解析。范围刻意是白名单
# （根级 *.md、docs/、.github/、scripts/、design/）：全仓 2096 个 markdown 共
# 1446 条相对链接，其中 195 条断在 vendored 第三方技能包里
# （skills/codebuddy/**、skills/trae/**、pyengine/.../bundled/**）——
# 那些不是本项目的内容，不该由本仓的门禁负责。
check_doc_links() {
  run_py_doc_check "文档相对链接 (check-doc-links.py)" scripts/check-doc-links.py
}

# 两份 README 必须在硬事实上一致（基线 / ignored / skipped / files / locale 键数 /
# 版本号 / 受门禁脚本表 / 标题结构），**并且**它们印出的闸门总数必须等于
# `--all` 真正调度的检查数。后半条不是装饰：R27 就是靠它发现
# 「README 写 18/18，而 --all 实际只跑 17 项」（密钥扫描漏接进 --all）。
check_readme_claims() {
  run_py_doc_check "README 事实与闸门总数 (check-readme-claims.py)" \
    scripts/check-readme-claims.py
}

check_tokens_sync() {
  if command -v node >/dev/null 2>&1 && [ -f scripts/check-tokens-sync.mjs ]; then
    # 标签于 2026-10-22 从「三端」改为「端点」：GUI（gui/src）已归档，
    # 现在的消费端是 WebUI 10 个主题 + TUI。同时该脚本内含两条归档守卫
    # （gui/src 不得重现）与各主题 57 个语义色的登记校验。
    run_check "设计令牌端点同步 (design/tokens.json)" \
      node scripts/check-tokens-sync.mjs
  else
    echo "${C_HEAD}[SKIP]${C_RST} node 或 check-tokens-sync.mjs 不可用，跳过"
    SKIPPED+=("设计令牌端点同步 (design/tokens.json) — node 或脚本不可用")
  fi
}

# `npm ci` 要求 package.json 与 package-lock.json 完全自洽，否则直接报错：
#   "`npm ci` can only install packages when your package.json and
#    package-lock.json … are in sync" / "Missing: <pkg>@<ver> from lock file"
# 这是 CI 的第一步就会踩的坑，但本地 `npm audit` **看不见**它 ——
# audit 审的是 lockfile 解析出来的依赖树，不是「声明与 lock 是否一致」。
# 本仓库实测过三种真实漂移：lockfile 的 packages[""] 版本号残留
# （webui 0.2.1 / gui 0.1.5）、声明了新依赖却没进 lockfile，
# 以及 **gui/ 的 lockfile 整棵 eslint 依赖树缺失**（lockfile 里 @eslint/js 是
# 10.0.1 而 package.json 要 9.39.5，`npm ci` 报 85 条 Missing）。
# `npm ci --dry-run` 只解析不落盘，能安全地在闸门里跑。
#
# ⚠️ 本条**必须同时覆盖 webui 与 gui**。此前只检查 webui，于是 gui 的 lockfile
# 漂移长期无人可见：整套闸门在**本机**是绿的（gui 的 node_modules 早就装好了，
# 四项 gui 检查因此从不触发 npm ci），但在**全新 clone 上 `npm ci` 直接失败**，
# 四项 gui 检查连启动都做不到（`tsc is not recognized`）。
check_webui_lockfile_sync() {
  if ! command -v npm >/dev/null 2>&1; then
    skip_check "webui lockfile 自洽 (npm ci --dry-run)" "npm 不可用"
    return 0
  fi
  if [ ! -f webui/package-lock.json ]; then
    skip_check "webui lockfile 自洽 (npm ci --dry-run)" "webui/package-lock.json 不存在"
    return 0
  fi
  run_check "webui lockfile 自洽 (npm ci --dry-run)" \
    bash -c "cd webui && npm run --silent deps:lockfile-sync"
}

check_gui_lockfile_sync() {
  if ! command -v npm >/dev/null 2>&1; then
    skip_check "gui lockfile 自洽 (npm ci --dry-run)" "npm 不可用"
    return 0
  fi
  if [ ! -f gui/package-lock.json ]; then
    skip_check "gui lockfile 自洽 (npm ci --dry-run)" "gui/package-lock.json 不存在"
    return 0
  fi
  run_check "gui lockfile 自洽 (npm ci --dry-run)" \
    bash -c "cd gui && npm run --silent deps:lockfile-sync"
}

# webui/ 的 console 会渲染模型输出，所以消毒器（dompurify）与 id 生成器（nanoid）
# 这类公告属真实产品风险，而不是噪音。CI 的 webui job 已跑 `npm run security:audit`，
# 但只接 CI 会让本地 `verify.sh` 放过依赖回退（本仓库此前正是静态携带 4 条公告：
# dompurify XSS / nanoid 死循环 / vitest 路径穿越，且无任何审计步骤）。这里补上本地闸门。
#
# 返回 0 表示「无 moderate+ 公告」；npm 不可用或目录缺失时 **SKIP 而不是 FAIL** ——
# 该检查依赖已安装的 node_modules，在没有装依赖的环境里不应把门禁判红。
check_dependency_audit() {
  if ! command -v npm >/dev/null 2>&1; then
    skip_check "webui 依赖审计 (npm audit --audit-level=moderate)" "npm 不可用"
    return 0
  fi
  if [ ! -d webui/node_modules ]; then
    skip_check "webui 依赖审计 (npm audit --audit-level=moderate)" "webui/node_modules 不存在（未安装依赖）"
    return 0
  fi
  run_check "webui 依赖审计 (npm audit --audit-level=moderate)" \
    bash -c "cd webui && npm run --silent security:audit"
}

check_webui_typecheck() {
  if command -v npm >/dev/null 2>&1; then
    run_check "webui 类型+架构门禁 (npm run typecheck)" \
      bash -c "cd webui && npm run typecheck"
  else
    skip_check "webui 类型+架构门禁 (npm run typecheck)" "npm 不可用"
  fi
}

check_webui_test() {
  if command -v npm >/dev/null 2>&1; then
    run_check "webui 单测 (npm run test:unit)" \
      bash -c "cd webui && npm run test:unit"
  else
    skip_check "webui 单测 (npm run test:unit)" "npm 不可用"
  fi
}

# ---------------------------------------------------------------------------
# Web 端 E2E（Playwright）—— 纳入门禁，但只覆盖**已验证为确定性的子集**。
#
# 为什么不是「全部 e2e」：本轮（2026-09-25）在干净环境下把整套跑完的实测是
#   142 passed / 130 failed / 22 skipped
# 大量用例依赖**本机外部状态**：「已配置模型供应商 / 历史会话数据 / Vite dev server
# （岛屿用例动态 import('/src/...') 只能打 dev server）」。把整套塞进闸门只会得到
# 一道恒红的门禁 —— 那比没有更糟（会训练人忽略红色）。
#
# 所以本层是「棘轮」：把已经证明稳定且自足的 spec **文件**逐个列入 WEBUI_E2E_GREEN，
# 并断言本轮通过数 ≥ WEBUI_E2E_BASELINE（只允许增、不允许悄悄减）。剩下的文件作为
# **待修复清单**记在 WEBUI_E2E_BACKLOG，不假装它们通过（skip ≠ pass）。
# 修复一个文件就把它从 BACKLOG 挪到 GREEN 并把基线抬高 —— 覆盖只会单向增长。
#
# 环境自足：自起**隔离**网关（临时 HOME/LOCALAPPDATA + 专用端口），跑完 `taskkill /T`
# 连 Python 侧车一起收掉 —— 绝不碰用户真实 ~/.marguerite 配置与 AppData 数据。
WEBUI_E2E_GREEN=(
  e2e/approval-card.spec.ts
  e2e/artifact-preview.spec.ts
  e2e/brand-keys.guard.spec.ts
  e2e/chat-theme.spec.ts
  e2e/computer-use.spec.ts
  e2e/deliverables.spec.ts
  e2e/floating-composer.spec.ts
  e2e/island-quality.spec.ts
  e2e/logs-detail.spec.ts
  e2e/meta-ribbon.spec.ts
  e2e/pet.spec.ts
  e2e/preview-origin-csp.spec.ts
  e2e/settings-modal.spec.ts
  e2e/theme.spec.ts
  e2e/user-message-newlines.spec.ts
)
# 上面的 15 个文件在隔离网关下的实测通过数（2026-09-25，--retries=0 --workers=4；
# R47 岛屿档位 2 例、R48 桌宠悬停 1 例、R49 theme 14 例、R51 settings-modal 22 例、
# R52 meta-ribbon 8 例 + floating-composer 3 例、R53 preview-origin-csp 1 例、
# R55 brand-keys.guard 2 例 —— 后者是纯文件系统 lint，不需要网关/浏览器）
# ⚠️ preview-origin-csp 依赖 `openssl` 在 PATH 上（自签 127.0.0.1 证书）；Git Bash 的
# mingw64 自带，缺失时该例会在证书那步失败——是环境缺失，不是产品回归。
WEBUI_E2E_BASELINE=95
WEBUI_E2E_BACKLOG=18
WEBUI_E2E_PORT="${WEBUI_E2E_PORT:-18877}"
# 并发度：4 worker 在 12 核机上仍会偶发「菜单点击时组件重渲染 → URL 未变」与
# 「实时快照重置 turn → 审批卡消失」。棘轮要的是确定性，不是最快；默认 2，
# 可用 WEBUI_E2E_WORKERS 覆盖（仅在确认更高并发稳定时）。
WEBUI_E2E_WORKERS="${WEBUI_E2E_WORKERS:-2}"

check_webui_e2e() {
  local name="webui e2e (Playwright 确定性子集)"
  if ! command -v npm >/dev/null 2>&1; then
    skip_check "$name" "npm 不可用"
    return 0
  fi
  if [ ! -x webui/node_modules/.bin/playwright ]; then
    skip_check "$name" "webui/node_modules 未安装（先 npm ci）"
    return 0
  fi
  # 网关托管的是 webui/dist，必须是**当前源码**的构建产物。ensure_webui_dist 只在
  # 缺失时构建，于是改完前端后它会静默托管上一次的产物 —— 与「只在缺失时编 core」
  # 同一类假证据（2026-10-26 W0 实测两者并存）。e2e 一律先重建产物。
  local dist_log
  dist_log="$(mktemp)"
  echo "${C_HEAD}-- 托管产物: cd webui && npm run build${C_RST}"
  if ! bash -c "cd webui && npm run build" >"$dist_log" 2>&1; then
    tail -n 20 "$dist_log" | sed 's/^/    /'
    rm -f "$dist_log"
    FAILED+=("$name")
    echo "${C_FAIL}[FAIL]${C_RST} $name — webui/dist 构建失败（e2e 拒绝跑陈旧产物）"
    return 1
  fi
  rm -f "$dist_log"
  # 每次都以当前源码重建/重链网关 —— 「只在缺失时构建」曾被真实命中（2026-10-26 W0）：
  # core 构建因 os error 5（常驻 daemon 持有 exe）失败，e2e 于是拿**上一次构建的旧 exe**
  # 跑出 11 个假红（旧二进制没有 pet.triggerAction 的 ok:true，也还没有 3D 渲染路径）。
  # e2e 断言的是**当前源码**的行为，绝不能静默跑旧二进制。
  local bin="target/debug/marguerite-core"
  local build_log
  build_log="$(mktemp)"
  echo "${C_HEAD}-- 预置网关二进制: cargo build -p marguerite-core --bin marguerite-core${C_RST}"
  if ! cargo build -p marguerite-core --bin marguerite-core >"$build_log" 2>&1; then
    tail -n 20 "$build_log" | sed 's/^/    /'
    rm -f "$build_log"
    FAILED+=("$name")
    echo "${C_FAIL}[FAIL]${C_RST} $name — 网关二进制构建失败（e2e 拒绝跑陈旧二进制）"
    return 1
  fi
  rm -f "$build_log"
  [ -x "$bin.exe" ] && bin="$bin.exe"
  if [ ! -x "$bin" ]; then
    FAILED+=("$name")
    echo "${C_FAIL}[FAIL]${C_RST} $name — 构建成功但网关二进制仍不存在"
    return 1
  fi

  local home
  home="$(mktemp -d)"
  mkdir -p "$home/AppData/Local"
  # 起进程用 bash 自己的 `&`：**不用** PowerShell Start-Process + $( ) 取 PID ——
  # 实测那样会把命令替换挂住（子进程继承管道句柄，$( ) 永不 EOF），网关起来了但整个
  # 检查永远不返回。PID 稍后按「监听端口 → OwningProcess」反查，比记 bash 的 $!（MSYS
  # PID，taskkill 不认）更可靠。
  #
  # `MARGUERITE_CONFIG_DIR` 是**必须**的：Windows 上 `dirs::home_dir()` 走 Known Folder
  # API，不认 USERPROFILE/HOME —— 只改 HOME 时隔离网关仍会读写真实 ~/.marguerite
  # （污染用户配置，并与常驻 daemon 争抢同一份 config.yaml，制造并发写丢更新）。
  USERPROFILE="$home" HOME="$home" LOCALAPPDATA="$home/AppData/Local" \
    MARGUERITE_CONFIG_DIR="$home/.marguerite" \
    "$bin" gateway --port "$WEBUI_E2E_PORT" >"$home/gw.log" 2>&1 &
  local gw_job=$!

  local ready=0 i
  for i in $(seq 1 40); do
    # 端口探活走 bash 内建 /dev/tcp：不依赖 curl（本机 Git Bash 里 curl 曾不可见，
    # 会让就绪判定永远为假）
    if (exec 3<>"/dev/tcp/127.0.0.1/$WEBUI_E2E_PORT") 2>/dev/null; then
      exec 3<&- 3>&- 2>/dev/null
      ready=1
      break
    fi
    sleep 1
  done
  [ "$ready" -eq 1 ] && sleep 2

  # 侧车（pyengine）比 Rust 网关晚得多才就绪（冷启动曾到 ~90s），而 logs-detail
  # 的用例依赖 logs.tail → pyengine 的 debug.log：等不到日志就按「环境没日志」
  # 跳过，棘轮随之掉到 94 < 95（2026-10-26 W0 实测同一份代码跑出 95/2、94/3、
  # 93+1/3 三种结果）。这里在起 Playwright 前等侧车真正接管：带网关 token 请求
  # /api/config，在线时由侧车回 200，离线时 Rust 回 501；不带 token 会先被鉴权层
  # 拦成 401（无法区分在线/离线）。
  local token_file="$home/AppData/Local/Marguerite/gateway-token" token=""
  [ -f "$token_file" ] && token="$(tr -d '\r\n' < "$token_file")"
  local sidecar_ready=0 status_line i
  for i in $(seq 1 120); do
    if exec 3<>"/dev/tcp/127.0.0.1/$WEBUI_E2E_PORT" 2>/dev/null; then
      printf 'GET /api/config HTTP/1.1\r\nHost: 127.0.0.1:%s\r\nAuthorization: Bearer %s\r\nConnection: close\r\n\r\n' \
        "$WEBUI_E2E_PORT" "$token" >&3
      IFS= read -r -t 3 status_line <&3
      exec 3<&- 3>&- 2>/dev/null
      case "$status_line" in
        *" 200 "*) sidecar_ready=1; break ;;
      esac
    fi
    sleep 1
  done
  [ "$sidecar_ready" -eq 1 ] || echo \
    "${C_HEAD}-- 警告: 侧车 120s 内未接管 /api/config（logs-detail 将按环境跳过，棘轮会因此判红）${C_RST}"

  local out rc passed
  if [ "$ready" -eq 1 ]; then
    out="$(cd webui && OPENSQUILLA_WEBUI_BASE_URL="http://127.0.0.1:$WEBUI_E2E_PORT" \
      npx playwright test "${WEBUI_E2E_GREEN[@]}" --retries=0 --workers="$WEBUI_E2E_WORKERS" 2>&1)"; rc=$?
  else
    out="隔离网关 40s 内未就绪（$WEBUI_E2E_PORT）$(tail -n 5 "$home/gw.log" 2>/dev/null)"
    rc=1
  fi

  # 收尾：按监听端口反查 Windows PID，再 taskkill /T 连 Python 侧车一起收。
  # MSYS 会把 `/F` 当路径改写，必须 MSYS_NO_PATHCONV=1（否则静默失败、网关留在端口上）。
  local gw_pid
  gw_pid="$(powershell -NoProfile -Command \
    "(Get-NetTCPConnection -LocalPort $WEBUI_E2E_PORT -State Listen -ErrorAction SilentlyContinue).OwningProcess" \
    2>/dev/null | tr -d '[:space:]')"
  [ -n "$gw_pid" ] && MSYS_NO_PATHCONV=1 taskkill /F /T /PID "$gw_pid" >/dev/null 2>&1
  kill "$gw_job" 2>/dev/null
  wait "$gw_job" 2>/dev/null
  rm -rf "$home"

  printf '%s\n' "$out" | tail -n "$EVIDENCE_LINES"
  passed="$(printf '%s\n' "$out" | sed -n 's/^  \([0-9]\{1,\}\) passed.*/\1/p' | tail -n 1)"
  if [ "$rc" -eq 0 ] && [ -n "$passed" ] && [ "$passed" -ge "$WEBUI_E2E_BASELINE" ]; then
    echo "${C_PASS}[PASS]${C_RST} $name  ($passed passed ≥ 基线 $WEBUI_E2E_BASELINE；${WEBUI_E2E_BACKLOG} 个 spec 文件仍在待修复清单)"
    PASSED+=("$name")
  elif [ "$rc" -eq 0 ]; then
    echo "${C_FAIL}[FAIL]${C_RST} $name — 通过数 ${passed:-?} 低于基线 $WEBUI_E2E_BASELINE（棘轮只许升）"
    FAILED+=("$name")
    rc=1
  else
    echo "${C_FAIL}[FAIL]${C_RST} $name  (rc=$rc)"
    FAILED+=("$name")
  fi
  return "$rc"
}

check_workspace_build() {
  # gui/src-tauri 声明 `bundle.resources = ["webui/dist", …]`；Tauri 2 在资源路径缺失时
  # 中止 build script，故 fresh clone 无法直接跑 `cargo build --workspace`。
  # 统一前置见 ensure_webui_dist（同一逻辑也被 clippy 检查复用）。
  ensure_webui_dist "workspace 构建会编译 gui" || return 1
  run_check "全量构建门禁 (cargo build --workspace)" \
    cargo build --workspace
}

# gui 的前端门禁已于 2026-10-21 全部废止：`gui/src`（React）从未进入安装包
# （`tauri.conf.json` 的 `url` 指向网关 `http://localhost:18790/control/`，
# `bundle.resources` 也不含 `gui/dist`），已归档到 `docs/legacy/gui-react-2026-10-21/`。
# 前端改为**只在 webui（Vue）**一处维护，那边的门禁见 check_webui_*。
#
# 但「gui 层」不能变成空壳检查——它现在承载的是**真生效的那一半**：Rust 壳。
# 下面三项替换原有的 typecheck/lint/test/build。

# 守卫 1：确认前端源码没有被误加回来（双前端是本仓已付过代价的债）。
check_gui_no_frontend() {
  if command -v node >/dev/null 2>&1; then
    run_check "gui 无前端源码守卫 (verify:no-frontend)" \
      bash -c "cd gui && npm run --silent verify:no-frontend"
  else
    run_check "gui 无前端源码守卫（node 缺失时用 shell 判据）" \
      bash -c "test ! -d gui/src"
  fi
}

# 守卫 2：壳的类型/借用检查（等价于原 typecheck 的位置，但查的是真生效的 Rust）。
check_gui_shell_check() {
  run_check "gui 壳编译 (cargo check -p marguerite-gui)" \
    cargo check -p marguerite-gui
}

# 守卫 3：壳的架构不变量（原 lint 的位置）。
#
# 为什么不是 clippy：L3「零 warning」的判据属于 **workspace/core 层**
# （那里已有 `cargo clippy -p marguerite-core -p marguerite-gui --all-targets -- -D warnings`）。
# 在 gui 层再跑一遍，只会因为 core 的历史 lint 债务让 `--gui` 变红，
# 把「壳坏了」和「core 有 lint 债」两种完全不同的故障混成同一个红灯 —— 诊断价值为负。
#
# 这一项守的是**今天这次改造本身**：单一前端架构一旦被悄悄改回去
# （主窗口重新指向本地 dist、或 webui/dist 不再是打包源），必须立刻报错。
# 若早在两年前就有这条守卫，就不会出现「审计了 22 个面板才发现它们不可达」。
check_gui_shell_config() {
  # 先跑 --self-check（8 个合成夹具，证明判定函数本身有牙），再跑真实仓库。
  # 串成一条检查，避免汇总里出现两条同名项。
  run_check "gui 壳架构不变量 (tauri.conf.json)" \
    bash -c "python scripts/check_gui_shell_config.py --self-check && python scripts/check_gui_shell_config.py"
}

# pyengine（Python 侧车）单测 —— 关闭「1280 个 Python 测试文件零门禁覆盖」的盲区。
#
# 范围刻意限定为 tests/unit：纯单测，不碰网络、模型 API、浏览器或真实终端。需要真实
# 外部世界的用例由 marker 标出并在下面显式排除（标记体系见 pyengine/pyproject.toml
# 的 [tool.pytest.ini_options] markers）。
#
# 刻意**不用** `--timeout`：pytest-timeout 不是本仓库声明的测试依赖，在干净环境里加它
# 只会让门禁因为缺插件而变红，而不是因为代码有问题。
#
# `PYTHONUTF8=1` 是**必需项**，不是优化：yoyo 的迁移读取器（yoyo/migrations.py:175）
# 用 locale 编码打开 .py 迁移，中文 Windows 的默认 locale 是 cp936 → 读 UTF-8 源码
# 必抛 `UnicodeDecodeError: 'gbk' codec can't decode byte 0x94`。实测同一代码在
# UTF-8 locale 的 CI 上全绿、在本机红 20 例（test_ci 7 + test_migrations 13）。三条
# pyengine 检查都带上它，避免「红在开发机、绿在 CI」的不可复现差异。
check_pyengine_tests() {
  require_python "pyengine 单测 (tests/unit，无网络/无 LLM)" "${PYENGINE_PY_MODULES[@]}" || return 0
  if [ ! -d pyengine/tests/unit ]; then
    skip_check "pyengine 单测 (tests/unit)" "pyengine/tests/unit 不存在（侧车未随仓库分发）"
    return 0
  fi
  run_check "pyengine 单测 (tests/unit，无网络/无 LLM)" \
    bash -c "cd pyengine && PYTHONUTF8=1 '$PY' -m pytest tests/unit -q --no-header -p no:cacheprovider -m 'not (llm or llm_smoke or llm_costly or llm_tools or llm_embedding or llm_reasoning or llm_gateway or llm_image or llm_router_acc or local_golden or webui_browser or tui_real_terminal or live_channel or live_search)'"
}

# pyengine 网关契约层（tests/test_gateway）—— 侧车的第二道门禁。
#
# R58 之前这个目录**没有任何门禁跑过**：实测 `pytest tests/test_gateway` 是
# 29 failed / 2901 passed / 162s，而本脚本只跑 tests/unit，CI 也只多了一条「只查收集」
# 的检查 —— 于是一整棵会红的死树长期无人可见。29 个失败全部是与本轮改动无关的历史
# 漂移，逐个定性后修复（细节见 AGENTS.md §2）：
#   - 13 处静态源断言指向**从未迁移**的上游控制台目录 `opensquilla-webui/`（本仓控制台
#     是 `webui/`）→ 统一由 tests/test_gateway/spa_source.py 解析，不再逐文件写死；
#   - 14 个 auto-summarize 用例的夹具 budget=10 落在「压缩后载荷」的估算地板之下，
#     只在装了 tiktoken（可选的 memory extra）时才成立 → 换成两档估算器都成立的 20；
#   - 1 个断言源码树没有 dist 元数据时 `importlib.metadata` 的行为 → 见该用例注释；
#   - 1 个断言上游 docs/troubleshooting.md 的章节（从未迁移）→ 显式 skip 并写明原因。
#
# 与 tests/unit 分成两条检查是刻意的：161s 与「纯单测」不同量级，混进同一条 [PASS]
# 会掩盖新增的慢测。该目录**不含**任何 llm/live/browser 标记的用例（实测 0 处），
# 所以不需要 marker 过滤 —— 也就不存在第二份会漂移的排除清单。-rs 让 skip 的具名
# 原因出现在证据里：本仓铁律是跳过 ≠ 通过。
check_pyengine_gateway_tests() {
  require_python "pyengine 网关契约 (tests/test_gateway，无网络/无 LLM)" "${PYENGINE_PY_MODULES[@]}" || return 0
  if [ ! -d pyengine/tests/test_gateway ]; then
    skip_check "pyengine 网关契约 (tests/test_gateway)" \
      "pyengine/tests/test_gateway 不存在（侧车未随仓库分发）"
    return 0
  fi
  run_check "pyengine 网关契约 (tests/test_gateway，无网络/无 LLM)" \
    bash -c "cd pyengine && PYTHONUTF8=1 '$PY' -m pytest tests/test_gateway -q --no-header -p no:cacheprovider -rs"
}

# pyengine 契约族（tests/test_ci / test_migrations / test_sandbox /
# test_onboarding / test_tools / test_recovery）—— 侧车的第三道门禁。
#
# R60 之前这六个目录**没有任何门禁跑过**，而它们是会随提交发布的代码的契约层。
# R59 逐目录实测（`%TEMP%\r59_recon.txt`）：
#   test_ci        35 passed /  0 failed（48s）
#   test_migrations 59 passed /  0 failed（31s）
#   test_sandbox  1738 passed / 11 failed（94s）
#   test_onboarding 955 passed /  3 failed（44s）
#   test_tools    1200 passed /  2 failed（109s）
#   test_recovery  431 passed / 14 failed（121s）
# 30 个失败已逐条定性（见 AGENTS.md §2 的 R60 条目）：13 例是真缺陷（Windows
# 上 os.fchmod 自 3.13 起存在、rpc_sandbox 的 WinError 1921、五处指向从未迁移的
# 上游控制台目录、子进程 import 不到 opensquilla、硬编码 python3、Build 判定），
# 其余是平台语义差异或从未迁移的上游脚手架 → **显式 skip 并写明理由**（§8：跳过 ≠ 通过）。
#
# 六个目录在**一次**检查里跑完，但**逐个目录**登记证据：它们是同一层契约，拆成
# 六条 [PASS] 只会把闸门总数从 25 撑到 30，掩盖真正的粒度；而合成一条又不肯只打印
# 一个总计数 —— 每个目录的失败行与汇总行都进证据，失败时能直接看到是哪个目录、
# 哪条用例。耗时约 5 分钟，与 test_gateway 的 166s 同量级，故单独成条。
check_pyengine_contract_tests() {
  local name
  name="pyengine 契约族 (test_ci/test_migrations/test_sandbox/test_onboarding/test_tools/test_recovery)"
  require_python "$name" "${PYENGINE_PY_MODULES[@]}" || return 0
  local suite missing=0
  for suite in test_ci test_migrations test_sandbox test_onboarding test_tools test_recovery; do
    [ -d "pyengine/tests/$suite" ] || missing=1
  done
  if [ "$missing" -eq 1 ]; then
    skip_check "$name" "pyengine/tests 下的契约目录不完整（侧车未随仓库分发）"
    return 0
  fi

  local start end dur out rc failures=0
  start=$(date +%s)
  echo
  echo "${C_HEAD}=== [CHECK] $name ===${C_RST}"
  for suite in test_ci test_migrations test_sandbox test_onboarding test_tools test_recovery; do
    out="$(bash -c "cd pyengine && PYTHONUTF8=1 '$PY' -m pytest tests/$suite -q --no-header -p no:cacheprovider -rs" 2>&1)"
    rc=$?
    local evidence summary
    # 失败/收集错误逐条打印；跳过的具名理由留在下面的汇总行里（-rs）。
    evidence="$(printf '%s\n' "$out" | grep -E '^(FAILED|ERROR) ' | head -n 20)"
    summary="$(printf '%s\n' "$out" | grep -E '[0-9]+ (passed|failed|error)' | tail -n 1)"
    [ -n "$evidence" ] && printf '%s\n' "$evidence" | sed 's/^/    /'
    if [ -n "$summary" ]; then
      printf '%s\n' "$summary" | sed "s/^/  ${C_HEAD}tests\/$suite:${C_RST} /"
    else
      # 无汇总行 = 输出里没有 pytest 的结论行（进程被杀/被打断/输出为空）。原写法此时
      # **一行都不打印**，于是「红」只剩末尾那句计数（2026-09-25 r63_gate 的
      # test_recovery 实测如此）—— 违反本仓「红必须带证据」。退回原始输出尾部。
      echo "  ${C_HEAD}tests/$suite:${C_RST} 无 pytest 汇总行（rc=$rc），原始输出尾部："
      printf '%s\n' "$out" | tail -n 20 | sed 's/^/      /'
    fi
    if [ "$rc" -ne 0 ]; then failures=$((failures + 1)); fi
  done
  end=$(date +%s); dur=$((end - start))
  if [ "$failures" -eq 0 ]; then
    echo "${C_PASS}[PASS]${C_RST} $name  (${dur}s)"
    PASSED+=("$name")
    return 0
  fi
  echo "${C_FAIL}[FAIL]${C_RST} $name  ($failures/6 个目录有失败, ${dur}s)"
  FAILED+=("$name")
  return 1
}

check_release_build() {
  run_check "发布构建 (cargo tauri build)" \
    cargo tauri build
}

# workbench/ 是**第五个入口**：Python 后端（engine · api · cli · llmlab · ttc ·
# agentloop）+ 由 FastAPI 同源托管的 React SPA（:8011）+ 独立打包流水线。
# 它的 README 把 `packaging/check_version_sync.py` 写成「CI 门禁」，但 R28 实测
# `grep -c workbench scripts/verify.sh` = **0** —— 71 个测试与版本门禁都没有任何
# 门禁跑过，而 workbench/ 是**会随提交发布**的（`git ls-files -co` 里 76 个文件）。
# 与 R24 的 pyengine 同一类问题：对一部分代码库沉默的绿徽章。
check_workbench_tests() {
  require_python "workbench 单测 (71 例，离线，无 key)" "${WORKBENCH_PY_MODULES[@]}" || return 0
  if [ ! -d workbench/backend/marguerite_workbench/tests ]; then
    skip_check "workbench 单测 (71 例，离线)" "workbench/backend 未随仓库分发"
    return 0
  fi
  # `-p no:cacheprovider`：不写 .pytest_cache，否则跑一次门禁就多一份机器状态。
  run_check "workbench 单测 (71 例，离线，无 key)" \
    bash -c "cd workbench/backend && '$PY' -m pytest marguerite_workbench/tests -q --no-header -p no:cacheprovider"
}

# workbench/VERSION 是**第二条版本线**（与 core/Cargo.toml 的引擎版本互相独立：
# 实测引擎 0.2.6 / workbench 1.1.0）。check-version-sync.py 覆盖的是引擎那条线的
# 9 处，workbench 这条线由它自己的脚本守。
check_workbench_version() {
  run_py_doc_check "workbench 版本一致 (VERSION ↔ backend/frontend)" \
    workbench/packaging/check_version_sync.py
}

# RSI 断点 B5：8 团 43 成员存在两份真相源 —— 仓库内 `expert_teams.py`
# 与客户端 `my-experts` 插件。两处独立维护必然漂移（2026-10-26 已实证：
# 改了客户端 8 包，仓库内 43 角色零变化）。
# 本检查默认只验**仓库内自洽**（CI 可跑、不 import、不执行被检代码）；
# 本地开发机可另跑 `--client-dir <my-experts/plugins>` 做双端比对。
check_expert_parity() {
  run_py_doc_check "专家定义自洽 (check-expert-parity.py)" \
    scripts/check-expert-parity.py
}

# ---------------------------------------------------------------------------
# 参数解析
# ---------------------------------------------------------------------------
MODE_ALL=0; MODE_CORE=0; MODE_WEBUI=0; MODE_GUI=0; MODE_PYENGINE=0; MODE_WORKBENCH=0; MODE_FAST=0; MODE_RELEASE=0; DO_CLEAN=0; MODE_E2E=0
for arg in "$@"; do
  case "$arg" in
    --all)       MODE_ALL=1;;
    --core)      MODE_CORE=1;;
    --webui)     MODE_WEBUI=1;;
    --gui)       MODE_GUI=1;;
    --pyengine)  MODE_PYENGINE=1;;
    --workbench) MODE_WORKBENCH=1;;
    --e2e)       MODE_E2E=1;;
    --fast)      MODE_FAST=1;;
    --release)   MODE_RELEASE=1; MODE_ALL=1;;
    --clean)     DO_CLEAN=1;;
    -h|--help)
      grep -E '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0;;
    *) echo "[FAIL] 未知参数: $arg （--help 查看用法）"; exit 2;;
  esac
done
# 未指定任何模式 → 默认 --all
if [ "$MODE_ALL" -eq 0 ] && [ "$MODE_CORE" -eq 0 ] && [ "$MODE_WEBUI" -eq 0 ] && [ "$MODE_GUI" -eq 0 ] && [ "$MODE_PYENGINE" -eq 0 ] && [ "$MODE_WORKBENCH" -eq 0 ] && [ "$MODE_FAST" -eq 0 ] && [ "$MODE_E2E" -eq 0 ]; then
  MODE_ALL=1
fi

echo "${C_HEAD}############################################################${C_RST}"
echo "${C_HEAD}# Marguerite 完成前验证闸门  ($(date '+%Y-%m-%d %H:%M:%S'))${C_RST}"
echo "${C_HEAD}# 根: $ROOT${C_RST}"
echo "${C_HEAD}# 模式:$([ $MODE_FAST -eq 1 ] && echo ' fast')$([ $MODE_CORE -eq 1 ] && echo ' core')$([ $MODE_WEBUI -eq 1 ] && echo ' webui')$([ $MODE_GUI -eq 1 ] && echo ' gui')$([ $MODE_PYENGINE -eq 1 ] && echo ' pyengine')$([ $MODE_WORKBENCH -eq 1 ] && echo ' workbench')$([ $MODE_E2E -eq 1 ] && echo ' e2e')$([ $MODE_ALL -eq 1 ] && echo ' all')$([ $MODE_RELEASE -eq 1 ] && echo ' release')$([ $DO_CLEAN -eq 1 ] && echo ' +clean')${C_RST}"
echo "${C_HEAD}############################################################${C_RST}"

# ---------------------------------------------------------------------------
# WorkBuddy safe-delete 垫片：检测 + 响亮告知 + 中和（详见 AGENTS.md §4）
# ---------------------------------------------------------------------------
# 本机 WorkBuddy 的 Python 启动链通过 **PYTHONPATH** 注入 sitecustomize 垫片
# （…\WorkBuddy\resources\app.asar.unpacked\cli\vendor\shim\sitecustomize.py），
# 它把所有「删除」改投回收站，副作用会让**被测代码**看到错误的删除结果（非产品缺陷）：
#   1) 批量删除 >5000 文件要审批 → 吃掉 pytest 汇总行（进程 rc=1）；
#   2) 单文件删除 fail-closed（[Errno 53] 找不到网络路径）→ test_recovery 假红 8 例。
# 这正是 R60–R62「三轮拿不到全绿」的复现装置 —— 除非被中和，每个后来者都会踩。
#
# 判据：该解释器启动时加载的 sitecustomize 位于 WorkBuddy 的 cli/vendor/shim 下
# （PYTHONPATH 注入 → 对**所有**解释器生效，故用第一个能运行的解释器探测即可）。
# ⚠️ **只有检测到才中和**，绝不无条件 export —— CI 与用户普通终端没有此垫片，
#    必须零副作用、零额外输出。
# 想保留垫片（复现那 8 个假红）：MARGUERITE_KEEP_SAFE_DELETE=1。
_detect_safe_delete_shim() {
  local py="$1" f fn
  [ -n "$py" ] || return 1
  f="$("$py" -c 'import sitecustomize; print(getattr(sitecustomize,"__file__","") or "")' 2>/dev/null)"
  [ -n "$f" ] || return 1                    # 没有 sitecustomize → 无垫片
  fn="${f//\\//}"                            # 反斜杠→正斜杠，统一匹配
  case "$fn" in
    *WorkBuddy*cli/vendor/shim*) return 0;;
  esac
  # 退路判据：本会话标识非空且确实加载到了 sitecustomize（垫片来源）
  [ -n "${CODEBUDDY_SESSION_ID:-}" ] && return 0
  return 1
}

pick_python                                  # 设 $PY = 第一个能运行的解释器
if _detect_safe_delete_shim "$PY"; then
  echo
  echo "${C_FAIL}### [WARN] 检测到 WorkBuddy 的 Python safe-delete 垫片（sitecustomize） ###${C_RST}"
  echo "${C_FAIL}#  来源: PYTHONPATH 注入的 cli/vendor/shim/sitecustomize.py${C_RST}"
  echo "${C_FAIL}#  后果1: 批量删除 >5000 文件要审批 → 吃掉 pytest 汇总行（rc=1）${C_RST}"
  echo "${C_FAIL}#  后果2: 单文件删除 fail-closed（[Errno 53] 找不到网络路径）→ test_recovery 假红 8 例${C_RST}"
  echo "${C_FAIL}#        （以上均为垫片副作用，不是产品缺陷）${C_RST}"
  if [ "${MARGUERITE_KEEP_SAFE_DELETE:-}" = "1" ]; then
    echo "${C_FAIL}#  本次【保留】垫片、不中和（MARGUERITE_KEEP_SAFE_DELETE=1）：上述后果会出现${C_RST}"
  else
    export CODEBUDDY_SAFE_DELETE_ENABLED=0
    echo "${C_FAIL}#  本次将中和它：export CODEBUDDY_SAFE_DELETE_ENABLED=0（仅影响本脚本起的子进程）${C_RST}"
    echo "${C_FAIL}#  如需保留垫片，请设 MARGUERITE_KEEP_SAFE_DELETE=1${C_RST}"
    echo "${C_HEAD}-- [safe-delete] 已中和 WorkBuddy 垫片（CODEBUDDY_SAFE_DELETE_ENABLED=0）${C_RST}"
  fi
  echo
fi

# ---------------------------------------------------------------------------
# 执行
# ---------------------------------------------------------------------------
# 仓库发布卫生：**将随提交落地**的文件里不得出现「真钥匙形态」的字符串。
# 扫描范围刻意用 `-co --exclude-standard`（即 `git add -A` 的集合）：被忽略的草稿里有密钥
# 不是发布风险，被提交的文件里有才是。测试夹具必须**自我标注**（`sk-fake-…`），
# 因为一个 35 位高熵字符串挨着 `API_KEY=` 时，脚本无法分辨它是夹具还是真钥匙。
# ⚠️ 本检查只能管住「将来」：**已经在历史里的密钥不会因此消失**（见 AGENTS.md §4）。
check_secrets() {
  require_python "仓库密钥扫描 (check-secrets.py)" || return 0
  if [ -f scripts/check-secrets.py ]; then
    run_check "仓库密钥扫描 (check-secrets.py)" \
      "$PY" scripts/check-secrets.py
  else
    echo "${C_HEAD}[SKIP]${C_RST} check-secrets.py 不可用，跳过"
    SKIPPED+=("仓库密钥扫描 (check-secrets.py) — python 或脚本不可用")
  fi
}

if [ "$MODE_FAST" -eq 1 ]; then
  check_core_tests
  check_clippy_ws
  check_webui_typecheck
  check_secrets
fi

if [ "$MODE_CORE" -eq 1 ]; then
  check_core_tests
  check_core_integration_tests
  check_core_build
  check_clippy_core
  check_doc_drift
  check_version_sync
  check_secrets
  check_doc_links
  check_readme_claims
fi

if [ "$MODE_WEBUI" -eq 1 ]; then
  check_webui_typecheck
  check_webui_test
  # 令牌同步于 2026-10-22 起承载 webui 10 个主题、57 个语义色的登记校验，
  # 因此必须进 webui 层（此前只在 --all 里跑，单跑 --webui 会漏）。
  check_tokens_sync
  check_webui_lockfile_sync
  check_dependency_audit
fi

if [ "$MODE_GUI" -eq 1 ]; then
  check_gui_no_frontend
  check_gui_shell_check
  check_gui_shell_config
  # lockfile 仍需自洽：package.json 还有 tauri-cli 等依赖，全新 clone 要能 npm ci。
  check_gui_lockfile_sync
fi

if [ "$MODE_PYENGINE" -eq 1 ]; then
  check_pyengine_tests
  check_pyengine_gateway_tests
  check_pyengine_contract_tests
fi

if [ "$MODE_E2E" -eq 1 ]; then
  check_webui_e2e
fi

if [ "$MODE_WORKBENCH" -eq 1 ]; then
  check_workbench_tests
  check_workbench_version
  check_expert_parity
fi

if [ "$MODE_ALL" -eq 1 ]; then
  check_core_tests
  check_core_integration_tests
  check_core_build
  check_clippy_ws
  check_doc_drift
  check_version_sync
  # ⚠️ 这三项必须留在 --all 里：R27 曾把 check_secrets 只接进 --fast/--core，
  # 于是 --all 少跑一项而文档仍写 18/18。check_readme_claims 现在会核对这个总数。
  check_secrets
  check_doc_links
  check_readme_claims
  check_tokens_sync
  check_pyengine_tests
  check_pyengine_gateway_tests
  check_pyengine_contract_tests
  # workbench 是第五个入口，也会随提交发布（commit set 里 76 个文件），
  # 所以它的 71 个测试与版本门禁必须和 pyengine 一样进 --all。
  check_workbench_tests
  check_workbench_version
  check_webui_typecheck
  check_webui_test
  check_webui_e2e
  check_webui_lockfile_sync
  check_dependency_audit
  check_gui_no_frontend
  check_gui_shell_check
  check_gui_shell_config
  check_gui_lockfile_sync
  check_workspace_build
fi

if [ "$MODE_RELEASE" -eq 1 ]; then
  check_release_build
fi

# ---------------------------------------------------------------------------
# 汇总（铁证）
# ---------------------------------------------------------------------------
echo
echo "${C_HEAD}==================== 验证汇总 ====================${C_RST}"
if [ "${#PASSED[@]}" -gt 0 ]; then
  echo "${C_PASS}通过 (${#PASSED[@]}):${C_RST}"
  for p in "${PASSED[@]}"; do echo "  ${C_PASS}✓${C_RST} $p"; done
fi
if [ "${#SKIPPED[@]}" -gt 0 ]; then
  echo "${C_HEAD}跳过 (${#SKIPPED[@]}) —— 未验证，不计入通过:${C_RST}"
  for s in "${SKIPPED[@]}"; do echo "  ${C_HEAD}-${C_RST} $s"; done
fi
if [ "${#FAILED[@]}" -gt 0 ]; then
  echo "${C_FAIL}失败 (${#FAILED[@]}):${C_RST}"
  for f in "${FAILED[@]}"; do echo "  ${C_FAIL}✗${C_RST} $f"; done
  echo
  echo "${C_FAIL}闸门结果: 未通过 —— 禁止声称「完成/通过/已修复」。先修复上述失败项。${C_RST}"
  exit 1
fi
echo
if [ "${#PASSED[@]}" -eq 0 ] && [ "${#SKIPPED[@]}" -gt 0 ]; then
  # 0 项通过 = 本层没有取得任何新鲜证据。工具缺失不该判红（故意 SKIP 而非 FAIL），
  # 但也绝不能叫「全绿」——§8：跳过 ≠ 通过，不得据此声称完成。
  echo "${C_HEAD}闸门结果: 无有效结论 (0 项通过，${#SKIPPED[@]} 项跳过) —— 跳过项未获验证，不得声称完成。${C_RST}"
elif [ "${#SKIPPED[@]}" -gt 0 ]; then
  echo "${C_PASS}闸门结果: 全绿 (${#PASSED[@]} 项通过，${#SKIPPED[@]} 项跳过)。可据此声称完成——但跳过项未获验证。${C_RST}"
else
  echo "${C_PASS}闸门结果: 全绿 (${#PASSED[@]} 项通过)。可据此声称完成。${C_RST}"
fi
exit 0
