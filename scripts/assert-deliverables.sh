#!/usr/bin/env bash
# =============================================================================
# assert-deliverables.sh — 交付物存在性闸门
# -----------------------------------------------------------------------------
# 专治一类伪完成：「声称生成了文件 / 任务完成」但文件根本没落盘或为空。
# （典型场景：声称产出 HTML/PPT/报告，实则只调了工具、停在输入框。）
#
# 用法：
#   bash scripts/assert-deliverables.sh path/to/a.html path/to/b.pptx
#   bash scripts/assert-deliverables.sh "D:/Downloads/AI_News.html"
#
# 规则：
#   - 文件不存在        → [MISSING] 非零退出
#   - 文件存在但为空    → [EMPTY]   非零退出
#   - 全部存在且非空    → [OK] + 退出码 0
#
# 配合 AGENTS.md §8：在 present_files / 声称「已交付」之前必须先过此闸。
# =============================================================================
set -o pipefail

if [ "$#" -eq 0 ]; then
  echo "用法: assert-deliverables.sh <file> [<file> ...]"
  exit 2
fi

fail=0
for f in "$@"; do
  if [ ! -e "$f" ]; then
    echo "[MISSING] $f"
    fail=1
  elif [ -d "$f" ]; then
    echo "[OK-DIR] $f (目录)"
  elif [ ! -s "$f" ]; then
    echo "[EMPTY]   $f  (存在但 0 字节)"
    fail=1
  else
    sz=$(wc -c < "$f" | tr -d ' ')
    echo "[OK]      $f  (${sz} bytes)"
  fi
done

echo
if [ "$fail" -eq 0 ]; then
  echo "DELIVERABLES PRESENT — 所有交付物已落盘且非空，可声称已交付。"
  exit 0
else
  echo "DELIVERABLES INCOMPLETE — 存在缺失/空文件，禁止声称「已生成/已完成」。"
  exit 1
fi
