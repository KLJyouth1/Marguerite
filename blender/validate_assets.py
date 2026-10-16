"""
Marguerite 资产规格校验器（无 Blender 依赖）。

扫描 examples/ 目录下全部 *.json，逐个用 asset_pipeline.validate_spec() 校验，
输出人/机可读报告。任何一份 spec 非法 → exit 1；全部通过 → exit 0。

为什么单独一个脚本（而不是塞进 asset_pipeline.main）？
  - CI 门禁语义清晰：`python blender/validate_assets.py` 是「坏 spec 不入库」的守门员，
    与单个资产的构建命令（blender --background ...）职责分离；
  - 无 Blender 环境也能跑（validate_spec 是纯 Python），配合 unittest 构成
    两道不依赖 Blender 的质量闸门。

用法：
    python blender/validate_assets.py            # 校验 examples/ 全部 JSON
    python blender/validate_assets.py <dir>...   # 校验指定目录（可多个）
"""

import json
import os
import sys

# 让脚本可从任意 cwd 调用（import 同目录的 asset_pipeline）
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from asset_pipeline import SpecError, validate_spec  # noqa: E402

DEFAULT_DIRS = (os.path.join(os.path.dirname(os.path.abspath(__file__)), "examples"),)


def _iter_json_files(dirs):
    for d in dirs:
        if not os.path.isdir(d):
            print(f"[validate_assets] 跳过（目录不存在）：{d}", file=sys.stderr)
            continue
        for name in sorted(os.listdir(d)):
            if name.endswith(".json"):
                yield os.path.join(d, name)


def validate_dir(dirs) -> int:
    """校验指定目录下全部 JSON spec，返回 exit code（0=全过，1=有失败）。"""
    total = 0
    passed = 0
    failed = 0

    for path in _iter_json_files(dirs):
        total += 1
        rel = os.path.relpath(path)
        try:
            with open(path, "r", encoding="utf-8") as f:
                spec = json.load(f)
        except (OSError, json.JSONDecodeError) as e:
            failed += 1
            print(f"[FAIL] {rel}: JSON 读取/解析失败：{e}")
            continue

        try:
            warnings = validate_spec(spec)
        except SpecError as e:
            failed += 1
            print(f"[FAIL] {rel}: {e}")
            continue

        passed += 1
        note = f"（{len(warnings)} 条警告）" if warnings else ""
        print(f"[PASS] {rel}{note}")
        for w in warnings:
            print(f"       ⚠ {w}")

    print(f"\n[validate_assets] 共 {total} 份：通过 {passed}，失败 {failed}")
    return 1 if failed > 0 else 0


def main(argv=None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    dirs = tuple(argv) if argv else DEFAULT_DIRS
    return validate_dir(dirs)


if __name__ == "__main__":
    sys.exit(main())
