#!/usr/bin/env bash
# clone-third-party.sh — 批量克隆 GitHub 仓库用于二次开发 / 第三方集成
#
# 封装从一次真实 26 仓生产克隆（harness 17 + computer_use 9）中提炼的韧性协议，
# 详见同目录 SKILL.md。违反任一条都可能陷入「静默 0 进度死循环」（git 根本没执行）。
#
# 协议速查（必须遵守）：
#   1) 用 mv 替代 rm 做清理（本机 rm 被 genie-trash 拦截：吞输出 + exit 1）
#   2) 每仓独立日志，主日志自建自拥（规避重定向目标无写权限时 git 静默不执行）
#   3) http.lowSpeedTime=30（弱网假死 30s 即中止；600 会让重试循环根本转不动）
#   4) --no-checkout + git archive|tar 替代直接 checkout（规避收尾阶段锁/中断致目录回滚）
#   5) 同名仓库加 owner 后缀（deepseek-harness 两个 owner 互不覆盖）
#   6) Windows 非法路径：archive 失败时回退逐路径 checkout（sparse 之前就报错，故逐路径）
#   7) 外层自愈循环：上限 40 阶段、阶段间 45s、每仓 ≤3 重试 + 网络预检
set +e

# ---------- 参数解析 ----------
TARGET=""
MANIFEST=""
while [ $# -gt 0 ]; do
  case "$1" in
    --target)   TARGET="$2";   shift 2;;
    --manifest) MANIFEST="$2"; shift 2;;
    -h|--help)  echo "Usage: clone-third-party.sh --target <dir> [--manifest <file>]"; exit 0;;
    *) echo "[ERR] unknown arg: $1" >&2; exit 2;;
  esac
done
if [ -z "$TARGET" ]; then
  echo "[ERR] --target is required (e.g. --target /path/to/third_party)" >&2
  exit 2
fi
mkdir -p "$TARGET" 2>/dev/null || { echo "[ERR] cannot create target: $TARGET" >&2; exit 2; }

TS="$(date +%s)"
GC="-c http.version=HTTP/1.1 -c http.lowSpeedLimit=1 -c http.lowSpeedTime=30 -c http.postBuffer=104857600"
LOG_DIR="$TARGET/_logs"
mkdir -p "$LOG_DIR" 2>/dev/null
MAIN_LOG="$LOG_DIR/clone_$TS.log"
: > "$MAIN_LOG"; echo "[LOG] $MAIN_LOG"
GARBAGE="$TARGET/_garbage_$TS"
mkdir -p "$GARBAGE" 2>/dev/null

# ---------- 默认 26 仓清单（harness 17 + computer_use 9）----------
DEFAULT_REPOS=(
  "harness deepseek-ai/deepseek-harness"
  "harness Hmbown/CodeWhale"
  "harness 0xsline/awesome-deepseek-harness"
  "harness Prism-Shadow/penguin-harness"
  "harness awesome-dsh-plugin/awesome-dsh-plugin"
  "harness zhayujie/CowAgent"
  "harness NanmiCoder/dsh-agent-teams"
  "harness PM-Shawn/Abu-Cowork"
  "harness dataelement/dsh-desktop"
  "harness HenryZ838978/deepseek-harness"
  "harness CommandCodeAI/command-code"
  "harness zhnt/loushang"
  "harness liuchen6667/deepseek-auto-evolving-harness"
  "harness christopherarter/superpowers-reasonix"
  "harness Devin-AXIS/iPolloWork"
  "harness Dominic789654/awesome-deepseek-harness"
  "harness vixues/LeAgent"
  "computer_use chengyangkj/Ros_Qt5_Gui_App"
  "computer_use showlab/computer_use_ootb"
  "computer_use go-vgo/robotgo"
  "computer_use mesutpiskin/computer-vision-guide"
  "computer_use mikeroyal/Digital-Forensics-Guide"
  "computer_use ranpox/awesome-computer-use"
  "computer_use suitedaces/computer-agent"
  "computer_use likaixin2000/ScreenSpot-Pro-GUI-Grounding"
  "computer_use xlang-ai/OpenCUA"
)

# ---------- 取清单（--manifest 每行: <分类> <owner/repo>，支持 # 注释）----------
REPOS=()
if [ -n "$MANIFEST" ]; then
  while IFS= read -r line; do
    line="$(printf '%s' "$line" | sed 's/#.*//')"
    [ -z "$(printf '%s' "$line" | tr -d '[:space:]')" ] && continue
    REPOS+=("$line")
  done < "$MANIFEST"
else
  REPOS=("${DEFAULT_REPOS[@]}")
fi
if [ ${#REPOS[@]} -eq 0 ]; then
  echo "[ERR] empty repo list (manifest empty or default missing)" >&2
  exit 2
fi

DONE_FILE="$TARGET/.cloned_ok"
touch "$DONE_FILE"

# ---------- 工具函数 ----------
valid() {
  local dest="$1"
  [ -d "$dest/.git" ] \
    && git -C "$dest" rev-parse HEAD >/dev/null 2>&1 \
    && [ "$(git -C "$dest" ls-files 2>/dev/null | wc -l)" -gt 0 ]
}

stash() {
  # 用 mv 替代 rm（协议 1：规避 genie-trash 对 rm 的拦截）
  local p="$1" t
  [ -e "$p" ] || return 0
  t="$GARBAGE/$(basename "$p")_$RANDOM"
  mv "$p" "$t" 2>/dev/null && return 0
  return 1
}

# 把一条 "<分类> <owner/repo>" 派生出全局变量（协议 5：同名加 owner 后缀）
deriv() {
  local entry="$1"
  CAT="$(printf '%s' "$entry" | cut -d' ' -f1)"
  REPO_PATH="$(printf '%s' "$entry" | cut -d' ' -f2)"
  OWNER="$(printf '%s' "$REPO_PATH" | cut -d'/' -f1)"
  RNAME="$(basename "$REPO_PATH")"
  DEST="$TARGET/$CAT/${RNAME}-${OWNER}"
  KEY="$CAT/$OWNER/$REPO_PATH"
}

clone_one() {
  local entry="$1"
  deriv "$entry"
  local tmp rlog
  tmp="$TARGET/_tmp_${RNAME}_$$_$RANDOM"
  rlog="$LOG_DIR/${CAT}_${RNAME}-${OWNER}.log"
  mkdir -p "$(dirname "$rlog")" 2>/dev/null
  # 1) 若 dest 已存在（旧失败残留），先挪走（mv 安全，协议 1）
  if [ -e "$DEST" ]; then
    stash "$DEST" || return 1
  fi
  # 2) 只取对象，跳过工作树 checkout（协议 4：规避收尾阶段锁/中断）
  git $GC clone --depth 1 --no-checkout "https://github.com/$REPO_PATH.git" "$tmp" > "$rlog" 2>&1 || { stash "$tmp"; return 1; }
  # 3) archive 一次性提取工作树（协议 4）
  git -C "$tmp" archive HEAD 2>/dev/null | tar -x -C "$tmp" >> "$rlog" 2>&1
  # 4) 协议 6：archive/tar 失败时（如 Windows 非法路径仓库）回退逐路径 checkout
  if ! valid "$tmp"; then
    echo "[FALLBACK-6] archive/extract incomplete, per-path checkout for illegal-path safety" >> "$rlog"
    git -C "$tmp" ls-tree -r --name-only HEAD 2>/dev/null | while read -r pname; do
      git -C "$tmp" checkout HEAD -- "$pname" 2>/dev/null || true
    done
  fi
  # 5) 落到最终位置（mv，原子重命名）
  valid "$tmp" || { stash "$tmp"; return 1; }
  mv "$tmp" "$DEST" 2>/dev/null || { stash "$tmp"; return 1; }
  return 0
}

# ---------- 外层自愈循环（协议 7）----------
TOTAL=${#REPOS[@]}
MAX_PHASES=40
p=0
while [ "$(wc -l < "$DONE_FILE")" -lt "$TOTAL" ] && [ "$p" -lt "$MAX_PHASES" ]; do
  p=$((p+1))
  echo "===== PHASE $p / $MAX_PHASES (ok=$(wc -l < "$DONE_FILE")/$TOTAL) =====" | tee -a "$MAIN_LOG"
  fail_this_phase=0
  for entry in "${REPOS[@]}"; do
    deriv "$entry"
    # 已完成则跳过
    if grep -qx "$KEY" "$DONE_FILE" 2>/dev/null; then
      continue
    fi
    if valid "$DEST"; then
      echo "$KEY" >> "$DONE_FILE"
      echo "[OK-cached] $KEY" | tee -a "$MAIN_LOG"
      continue
    fi
    # 网络预检：github 不可达则跳过本次 clone（不消耗重试配额）
    if ! curl -sI --max-time 10 https://github.com >/dev/null 2>&1; then
      echo "[SKIP-net] $KEY github unreachable" | tee -a "$MAIN_LOG"
      fail_this_phase=$((fail_this_phase+1))
      continue
    fi
    echo "[CLONE] $KEY" | tee -a "$MAIN_LOG"
    ok=0
    for i in 1 2 3; do
      clone_one "$entry"
      if valid "$DEST"; then
        echo "$KEY" >> "$DONE_FILE"
        echo "[OK] $KEY (attempt $i)" | tee -a "$MAIN_LOG"
        ok=1
        break
      fi
      echo "[RETRY $i] $KEY incomplete/reset" | tee -a "$MAIN_LOG"
      sleep 4
    done
    if [ "$ok" -ne 1 ]; then
      echo "[FAIL] $KEY" | tee -a "$MAIN_LOG"
      fail_this_phase=$((fail_this_phase+1))
    fi
  done
  echo "--- phase $p end: ok=$(wc -l < "$DONE_FILE")/$TOTAL fail_this_phase=$fail_this_phase ---" | tee -a "$MAIN_LOG"
  if [ "$fail_this_phase" -eq 0 ]; then
    echo "ALL DONE at phase $p" | tee -a "$MAIN_LOG"
    break
  fi
  echo "[WAIT] sleeping 45s before next phase (network may be down)" | tee -a "$MAIN_LOG"
  sleep 45
done

echo "===== FINAL =====" | tee -a "$MAIN_LOG"
echo "  OK  : $(wc -l < "$DONE_FILE")" | tee -a "$MAIN_LOG"
echo "  FAIL: $(grep -c '\[FAIL\]' "$MAIN_LOG")" | tee -a "$MAIN_LOG"
echo "  GARBAGE: $GARBAGE" | tee -a "$MAIN_LOG"
