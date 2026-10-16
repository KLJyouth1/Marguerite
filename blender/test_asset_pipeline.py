"""
资产管线纯 Python 单元测试（不依赖 Blender）。

运行：
    python -m unittest discover blender

为什么这些测试不依赖 Blender？
  asset_pipeline.py 的 bpy 部分用 try 守卫导入（原因见该文件头注释），
  所以这里的 spec 校验测试可以在任何 Python 环境跑——CI 无 Blender 也能
  拦截坏 spec。坏数据进不了 Blender 阶段，就不会产出坏 GLB。

覆盖点（≥8 用例）：
  1. 合法 spec 校验通过
  2. 非法 geometry.main 拒绝
  3. scale <= 0 拒绝
  4. material 边界（metallic/roughness 超 [0,1]）拒绝
  5. examples/ 全部示例可过校验（诚实性红线：示例是真实规格而非占位）
  6. LOD 单调递减校验（targetFaces 非递减拒绝）
  7. id 命名规范（小写字母/数字/下划线）
  8. 未知字段拒绝（前向兼容的收口策略）
  9. accent 枚举边界
 10. detail 枚举边界
 11. load_asset_spec 端到端（读文件 + 校验）
"""

import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from asset_pipeline import SpecError, load_asset_spec, validate_spec  # noqa: E402

EXAMPLES_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "examples")


def _valid_spec():
    """一份合法的基准 spec（对齐 propsSpec.ts 的 code_crystal 条目）。"""
    return {
        "id": "code_crystal",
        "name": "代码结晶",
        "kind": "code_crystal",
        "geometry": {"main": "octahedron", "accent": "prism", "detail": 2, "scale": 0.8},
        "material": {
            "color": [0.45, 0.65, 0.9],
            "metallic": 0.35,
            "roughness": 0.28,
        },
        "lod": [
            {"level": 0, "targetFaces": 320, "decimateRatio": 1.0},
            {"level": 1, "targetFaces": 140, "decimateRatio": 0.45},
        ],
    }


class ValidateSpecTests(unittest.TestCase):
    def test_valid_spec_passes(self):
        """用例 1：合法 spec 校验通过，返回空 warning 列表。"""
        warnings = validate_spec(_valid_spec())
        self.assertEqual(warnings, [])

    def test_invalid_geometry_main_rejected(self):
        """用例 2：geometry.main 不在枚举内 → SpecError。"""
        spec = _valid_spec()
        spec["geometry"]["main"] = "torus"
        with self.assertRaises(SpecError):
            validate_spec(spec)

    def test_scale_zero_or_negative_rejected(self):
        """用例 3：scale <= 0 → SpecError（网格会退化）。"""
        for bad_scale in (0, -1.5):
            spec = _valid_spec()
            spec["geometry"]["scale"] = bad_scale
            with self.assertRaises(SpecError):
                validate_spec(spec)

    def test_material_metallic_roughness_bounds(self):
        """用例 4：metallic/roughness 超出 [0, 1] → SpecError。"""
        for field, bad_value in (("metallic", 1.5), ("metallic", -0.1),
                                 ("roughness", 1.2), ("roughness", -0.05)):
            spec = _valid_spec()
            spec["material"][field] = bad_value
            with self.assertRaises(SpecError):
                validate_spec(spec)

    def test_all_examples_pass(self):
        """用例 5：examples/ 全部示例必须过校验（诚实性红线：示例是真实规格而非占位）。"""
        files = [f for f in os.listdir(EXAMPLES_DIR) if f.endswith(".json")]
        self.assertGreater(len(files), 0, "examples/ 目录必须至少有一份示例 spec")
        for name in files:
            path = os.path.join(EXAMPLES_DIR, name)
            spec, _ = load_asset_spec(path)  # 不抛异常即通过
            self.assertIsInstance(spec, dict)

    def test_lod_monotonic_decreasing(self):
        """用例 6：lod[].targetFaces 非单调递减 → SpecError。"""
        spec = _valid_spec()
        spec["lod"] = [
            {"level": 0, "targetFaces": 100, "decimateRatio": 1.0},
            {"level": 1, "targetFaces": 100, "decimateRatio": 0.5},  # 未递减
        ]
        with self.assertRaises(SpecError):
            validate_spec(spec)

    def test_lod_level_must_increase(self):
        """用例 6b：lod[].level 重复/乱序 → SpecError。"""
        spec = _valid_spec()
        spec["lod"] = [
            {"level": 1, "targetFaces": 100},
            {"level": 0, "targetFaces": 50},
        ]
        with self.assertRaises(SpecError):
            validate_spec(spec)

    def test_id_naming_convention(self):
        """用例 7：id 必须是小写字母/数字/下划线（命名规范 marguerite_<id>）。"""
        for bad_id in ("Code_Crystal", "code-crystal", "代码结晶", ""):
            spec = _valid_spec()
            spec["id"] = bad_id
            with self.assertRaises(SpecError):
                validate_spec(spec)

    def test_unknown_field_rejected(self):
        """用例 8：未知顶层字段 → SpecError（收口，避免 spec 漂移出契约）。"""
        spec = _valid_spec()
        spec["extraField"] = 1
        with self.assertRaises(SpecError):
            validate_spec(spec)

    def test_accent_enum_boundary(self):
        """用例 9：accent 必须在枚举内；省略/None 均合法。"""
        spec = _valid_spec()
        spec["geometry"]["accent"] = "torus"
        with self.assertRaises(SpecError):
            validate_spec(spec)
        # 省略 accent 合法
        spec2 = _valid_spec()
        del spec2["geometry"]["accent"]
        self.assertEqual(validate_spec(spec2), [])

    def test_detail_enum_boundary(self):
        """用例 10：detail 必须是 1/2/3。"""
        for bad_detail in (0, 4, 2.5, "2"):
            spec = _valid_spec()
            spec["geometry"]["detail"] = bad_detail
            with self.assertRaises(SpecError):
                validate_spec(spec)

    def test_missing_lod_warns_not_fails(self):
        """LOD 缺失不阻断，但返回 warning（P1 可跑，P2 视距优化后补齐）。"""
        spec = _valid_spec()
        del spec["lod"]
        warnings = validate_spec(spec)
        self.assertEqual(len(warnings), 1)
        self.assertIn("lod", warnings[0])


class LoadAssetSpecTests(unittest.TestCase):
    def test_load_example_file(self):
        """用例 11：load_asset_spec 端到端读 examples/code_crystal.json。"""
        path = os.path.join(EXAMPLES_DIR, "code_crystal.json")
        spec, warnings = load_asset_spec(path)
        self.assertEqual(spec["id"], "code_crystal")
        self.assertEqual(warnings, [])


if __name__ == "__main__":
    unittest.main()
