#!/usr/bin/env bash
# =============================================================================
# package_tauri.sh — 打包桌面端安装包（可复现 + 带前置断言）
# -----------------------------------------------------------------------------
# 为什么需要它（2026-10-27 实测踩坑）：
#   1) **沙箱里必失败**：build.rs 要写 `target/release/resources/**`，沙箱会拒写，
#      表现为 `tauri-build 失败: 拒绝访问 (os error 5)` 或 `file-write` 被拦。
#   2) **并发 cargo 必失败**：另一个 `cargo check --workspace`（IDE/另一会话）会与
#      build.rs 抢同一个 `gui/src-tauri/resources/pyengine`，报
#      `清理目标目录失败 resources/pyengine: 拒绝访问`。
#   3) **WiX / NSIS 必须可达**：否则 tauri 去下载，代理 502 直接失败。
#   4) **`target/release/webui/dist` 会跨构建累积**：tauri bundling 是"合并覆盖、不清空"，
#      实测累积 11 个 `index-*.js` / 1633 个 assets（正常 380）。包里全是死文件。
#
# 用法：
#   bash scripts/package_tauri.sh                # MSI + NSIS
#   bash scripts/package_tauri.sh --nsis-only    # 只打 NSIS（省约 8 分钟）
#   bash scripts/package_tauri.sh --check        # 只做前置检查，不打包
#
# 退出码：0 = 成功；2 = 前置不满足（未开始）；1 = 打包或校验失败。
# =============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 2

WIX_DIR="/c/Program Files (x86)/WiX Toolset v3.14/bin"
NSIS_DIR="/c/Users/Administrator/AppData/Local/tauri/NSIS"
CARGO_BIN="$HOME/.cargo/bin"

MODE="both"
CHECK_ONLY=0
for a in "$@"; do
  case "$a" in
    --nsis-only) MODE="nsis" ;;
    --check) CHECK_ONLY=1 ;;
    *) echo "[FAIL] 未知参数: $a"; exit 2 ;;
  esac
done

fail() { echo "[FAIL] $*"; exit 2; }
ok()   { echo "[OK]   $*"; }

# ── 前置 1：无并发 cargo / rustc ────────────────────────────────────────────
running=$(tasklist 2>/dev/null | grep -icE '^(cargo|rustc)\.exe')
if [ "$running" != "0" ]; then
  echo "[BLOCK] 检测到 $running 个 cargo/rustc 正在运行："
  tasklist 2>/dev/null | grep -iE '^(cargo|rustc)\.exe' | head -5
  fail "打包必须独占构建目录（否则 build.rs 的 pyengine 同步会与对方互相删树）"
fi
ok "无并发 cargo/rustc"

# ── 前置 2：工具链可达 ──────────────────────────────────────────────────────
export PATH="$WIX_DIR:$NSIS_DIR:$CARGO_BIN:$PATH"
command -v candle.exe  >/dev/null 2>&1 || fail "找不到 candle.exe（WiX v3.14）"
command -v light.exe   >/dev/null 2>&1 || fail "找不到 light.exe（WiX v3.14）"
command -v makensis.exe >/dev/null 2>&1 || fail "找不到 makensis.exe（NSIS）"
ok "WiX: $(command -v candle.exe)"
ok "NSIS: $(command -v makensis.exe)"

# ── 前置 3：前端产物存在且非空 ──────────────────────────────────────────────
[ -f webui/dist/index.html ] || fail "webui/dist/index.html 不存在（先 npm run build）"
asset=$(grep -o 'assets/index-[A-Za-z0-9_-]*\.js' webui/dist/index.html | head -1)
[ -n "$asset" ] || fail "webui/dist/index.html 里没有 index-*.js 引用"
src_count=$(find webui/dist -type f | wc -l)
ok "仓库前端: $src_count 文件 · $asset"

if [ "$CHECK_ONLY" = "1" ]; then
  echo "[OK] 前置检查通过（未打包）"
  exit 0
fi

# ── 步骤 1：移走累积的包内前端（避免旧 index-*.js 混进安装包）──────────────
if [ -d target/release/webui/dist ]; then
  stale="/d/marg_audit_target/stale/webui-dist-$(date +%H%M%S)"
  mkdir -p /d/marg_audit_target/stale
  mv target/release/webui/dist "$stale" || fail "无法移走 target/release/webui/dist"
  echo "[STEP] 已把累积的包内前端移到 $stale"
fi

# ── 步骤 2：打包 ────────────────────────────────────────────────────────────
bundles="msi,nsis"
[ "$MODE" = "nsis" ] && bundles="nsis"
echo "[STEP] tauri build --bundles $bundles"
( cd gui && ./node_modules/.bin/tauri build --bundles "$bundles" ) || fail "tauri build 失败（看上方日志）"

# ── 步骤 3：校验包内前端 == 仓库前端（文件数 + 入口名）─────────────────────
dst="target/release/webui/dist"
[ -d "$dst" ] || fail "打包后找不到 $dst"
dst_count=$(find "$dst" -type f | wc -l)
dst_asset=$(grep -o 'assets/index-[A-Za-z0-9_-]*\.js' "$dst/index.html" | head -1)
[ "$dst_asset" = "$asset" ] || fail "包内入口 $dst_asset ≠ 仓库 $asset"
[ "$dst_count" = "$src_count" ] || fail "包内前端 $dst_count 文件 ≠ 仓库 $src_count（仍有累积或漏拷）"
ok "包内前端: $dst_count 文件 · $dst_asset（与仓库逐项一致）"

# ── 步骤 4：指纹 ────────────────────────────────────────────────────────────
echo
echo "===== 安装包指纹 ====="
for f in target/release/bundle/nsis/*.exe target/release/bundle/msi/*.msi; do
  [ -f "$f" ] || continue
  case "$f" in *0.2.13*|*0.2.12*|*0.2.16*) continue;; esac
  size=$(stat -c %s "$f")
  hash=$(sha256sum "$f" | cut -d' ' -f1)
  printf '%s\n  %s B  sha256=%s\n' "$f" "$size" "$hash"
done
