"""
Marguerite 通用道具资产工厂（Blender headless 资产规格 → glTF/GLB）。

设计依据：E:\\temp\\_island_3d_game_design.md §222-242「资产管线」
  AI 生成资产规格 JSON → bpy 脚本建模 → 导出 glTF/GLB → webui 运行时热加载
以及 §461 P1「Blender headless 资产工厂 + glTF/KTX2 管线跑通；
AI 生成首个道具/建筑并热加载」。

用法（在 Blender 内跑建模 + 导出）：
    blender --background --python asset_pipeline.py -- --spec examples/code_crystal.json --out out/code_crystal.glb

用法（无 Blender 的纯校验模式，CI 也能跑）：
    python asset_pipeline.py --spec examples/code_crystal.json --validate-only

为什么 bpy 用 try 守卫（`try: import bpy / except ImportError: bpy = None`）？
  本模块的**规格校验**（validate_spec / load_asset_spec）是纯 Python 逻辑，
  而**建模导出**（build_mesh / export_glb）才需要 Blender 运行时。用 try 守卫后：
  1. 本机/CI 没装 Blender 时，`import asset_pipeline` 不会崩，照样能跑
     validate_spec 拦截坏 spec（错误早于 Blender 启动几分钟才发现就太浪费）；
  2. 单元测试（test_asset_pipeline.py）可以不依赖 Blender 全绿，
     回归门槛留在「纯 Python」层——坏 spec 进不了 Blender 阶段；
  3. 同一份文件在 Blender 内（bpy 可用）和 Blender 外（bpy=None）行为一致，
     校验逻辑只有一份，不会出现「测试过的校验 ≠ 线上校验」的漂移。
  这就是「管线可测试性」：CI 无 Blender 也能拦截坏 spec，坏数据不进资产库。

用法入口 main()：解析 `--` 之后的参数，兼容「Blender 调用」和「裸 python 调用」
两种 argv 形式（Blender 会把 `--` 后的参数透传给脚本）。
"""

import argparse
import json
import os
import sys

# ─────────────────────────────────────────────────────────────────────────────
# bpy 守卫：Blender 外可导入本模块做纯校验（原因见文件头注释）
# ─────────────────────────────────────────────────────────────────────────────
try:
    import bpy  # type: ignore
    import bmesh  # type: ignore
except ImportError:
    # Blender 不在环境里 → 建模/导出函数会显式报错，但校验/加载纯函数照常可用
    bpy = None
    bmesh = None


# ─────────────────────────────────────────────────────────────────────────────
# 契约常量（与 webui/src/components/island/world3d/propsSpec.ts 的 PropGeometry 对齐）
# ─────────────────────────────────────────────────────────────────────────────

# PropGeometry.main 允许值：5 种主体 primitive
GEOMETRY_MAINS = ("octahedron", "cone", "sphere", "tower", "flower")
# PropGeometry.accent 允许值：5 种附加 primitive（可选）
GEOMETRY_ACCENTS = ("prism", "scroll", "berry", "lamp", "petal")

# 命名规范：所有导出对象/网格统一前缀，方便 webui 侧按名字查找/替换
NAME_PREFIX = "marguerite_"

# material 参数边界（PBR，与 glTF 标准一致）
METALLIC_MIN, METALLIC_MAX = 0.0, 1.0
ROUGHNESS_MIN, ROUGHNESS_MAX = 0.0, 1.0
EMISSIVE_MIN, EMISSIVE_MAX = 0.0, 4.0  # 允许 >1 以支持 HDR 自发光（灯/结晶辉光）


class SpecError(ValueError):
    """规格非法（字段缺失/类型错/越界/枚举不匹配）。message 是给 AI/人看的修复线索。"""


# ─────────────────────────────────────────────────────────────────────────────
# 纯 Python 校验（不依赖 bpy）——validate_spec 是本模块的「真相源」
# ─────────────────────────────────────────────────────────────────────────────

def _require(cond: bool, message: str):
    if not cond:
        raise SpecError(message)


def _check_str(value, field: str) -> str:
    _require(isinstance(value, str) and value.strip() != "", f"{field} 必须是非空字符串")
    return value


def _check_number(value, field: str, lo=None, hi=None) -> float:
    _require(isinstance(value, (int, float)) and not isinstance(value, bool),
             f"{field} 必须是数字")
    v = float(value)
    if lo is not None:
        _require(v >= lo, f"{field} 必须 >= {lo}（当前 {v}）")
    if hi is not None:
        _require(v <= hi, f"{field} 必须 <= {hi}（当前 {v}）")
    return v


def _check_color(value, field: str):
    """颜色是 [r, g, b]，每个分量在 [0, 1]。"""
    _require(isinstance(value, list) and len(value) == 3, f"{field} 必须是 [r, g, b] 三元组")
    for i, c in enumerate(value):
        _require(isinstance(c, (int, float)) and not isinstance(c, bool),
                 f"{field}[{i}] 必须是数字")
        _require(0.0 <= float(c) <= 1.0, f"{field}[{i}] 必须在 [0, 1]（当前 {c}）")


def _check_geometry(geom, path: str = "geometry"):
    _require(isinstance(geom, dict), f"{path} 必须是对象")
    allowed = {"main", "accent", "detail", "scale"}
    unknown = set(geom) - allowed
    _require(not unknown, f"{path} 含未知字段：{sorted(unknown)}（允许：{sorted(allowed)}）")

    main = geom.get("main")
    _require(main in GEOMETRY_MAINS,
             f"{path}.main 必须是 {list(GEOMETRY_MAINS)} 之一（当前 {main!r}）")

    if "accent" in geom and geom["accent"] is not None:
        accent = geom["accent"]
        _require(accent in GEOMETRY_ACCENTS,
                 f"{path}.accent 必须是 {list(GEOMETRY_ACCENTS)} 之一或省略（当前 {accent!r}）")

    detail = geom.get("detail")
    _require(detail in (1, 2, 3), f"{path}.detail 必须是 1/2/3（当前 {detail!r}）")

    # scale <= 0 会让网格退化成零体积/负翻转，直接拒绝
    _check_number(geom.get("scale"), f"{path}.scale", lo=0.0)
    _require(float(geom["scale"]) > 0.0, f"{path}.scale 必须 > 0（当前 {geom['scale']}）")


def _check_material(mat, path: str = "material"):
    _require(isinstance(mat, dict), f"{path} 必须是对象")
    allowed = {"name", "color", "metallic", "roughness", "emissive", "emissiveStrength"}
    unknown = set(mat) - allowed
    _require(not unknown, f"{path} 含未知字段：{sorted(unknown)}（允许：{sorted(allowed)}）")

    if "name" in mat:
        _check_str(mat["name"], f"{path}.name")
    if "color" in mat:
        _check_color(mat["color"], f"{path}.color")
    if "metallic" in mat:
        _check_number(mat["metallic"], f"{path}.metallic", METALLIC_MIN, METALLIC_MAX)
    if "roughness" in mat:
        _check_number(mat["roughness"], f"{path}.roughness", ROUGHNESS_MIN, ROUGHNESS_MAX)
    if "emissive" in mat:
        _check_color(mat["emissive"], f"{path}.emissive")
    if "emissiveStrength" in mat:
        _check_number(mat["emissiveStrength"], f"{path}.emissiveStrength", EMISSIVE_MIN, EMISSIVE_MAX)


def _check_lod(lod, path: str):
    _require(isinstance(lod, dict), f"{path} 必须是对象")
    allowed = {"level", "targetFaces", "decimateRatio"}
    unknown = set(lod) - allowed
    _require(not unknown, f"{path} 含未知字段：{sorted(unknown)}（允许：{sorted(allowed)}）")

    level = lod.get("level")
    _require(isinstance(level, int) and not isinstance(level, bool) and level >= 0,
             f"{path}.level 必须是 >= 0 的整数（当前 {level!r}）")

    faces = lod.get("targetFaces")
    _require(isinstance(faces, int) and not isinstance(faces, bool) and faces > 0,
             f"{path}.targetFaces 必须是 > 0 的整数（当前 {faces!r}）")

    if "decimateRatio" in lod:
        _check_number(lod["decimateRatio"], f"{path}.decimateRatio", 0.0, 1.0)
        _require(float(lod["decimateRatio"]) > 0.0,
                 f"{path}.decimateRatio 必须 > 0（当前 {lod['decimateRatio']}）")


def validate_spec(spec) -> list:
    """
    纯 Python 校验资产规格（不依赖 Blender）。

    返回 warning 列表（可空）；非法时抛 SpecError（error 级问题直接拒绝该 spec）。
    校验规则与 blender/schemas/prop_asset.schema.json 保持一致——
    schema 是给外部工具（IDE/AI 生成器）看的机器契约，这里是运行时真相源。
    """
    _require(isinstance(spec, dict), "spec 顶层必须是对象")

    allowed_top = {"id", "name", "kind", "version", "geometry", "material", "lod"}
    unknown = set(spec) - allowed_top
    _require(not unknown, f"spec 含未知字段：{sorted(unknown)}（允许：{sorted(allowed_top)}）")

    # id：命名规范 marguerite_<id> 的 <id> 部分，要求小写字母/数字/下划线
    spec_id = spec.get("id")
    _check_str(spec_id, "id")
    _require(all(ch.islower() or ch.isdigit() or ch == "_" for ch in spec_id),
             f"id 只允许小写字母/数字/下划线（当前 {spec_id!r}）")

    _check_str(spec.get("name"), "name")

    # kind 对齐 propsSpec.ts 的 PropKind（5 种道具语义）
    from_kinds = ("code_crystal", "doc_scroll", "data_fruit", "quest_beacon", "inspiration_bloom")
    kind = spec.get("kind")
    _require(kind in from_kinds, f"kind 必须是 {list(from_kinds)} 之一（当前 {kind!r}）")

    if "version" in spec:
        _check_str(spec["version"], "version")

    _check_geometry(spec.get("geometry"), "geometry")
    if "material" in spec:
        _check_material(spec["material"], "material")

    # LOD：可选但若给出必须是数组；再做单调递减校验（面数随级别递减）
    lods = spec.get("lod")
    warnings = []
    if lods is not None:
        _require(isinstance(lods, list) and len(lods) > 0, "lod 必须是非空数组")
        for i, entry in enumerate(lods):
            _check_lod(entry, f"lod[{i}]")
        levels = [entry["level"] for entry in lods]
        _require(levels == sorted(levels) and len(set(levels)) == len(levels),
                 f"lod[].level 必须严格递增且不重复（当前 {levels}）")
        faces = [entry["targetFaces"] for entry in lods]
        for i in range(1, len(faces)):
            _require(faces[i] < faces[i - 1],
                     f"lod[].targetFaces 必须随级别单调递减（lod[{i - 1}]={faces[i - 1]} → lod[{i}]={faces[i]}）")

    # 软提醒（不阻断）：LOD 缺失时 P1 仍可跑，但 P2 视距优化建议补齐
    if not lods:
        warnings.append("spec 缺少 lod 数组：导出将只有单一精度网格，视距优化需后续补齐")

    return warnings


def load_asset_spec(path: str):
    """读取并校验资产规格 JSON。返回 (spec, warnings)；非法抛 SpecError。"""
    with open(path, "r", encoding="utf-8") as f:
        raw = json.load(f)
    warnings = validate_spec(raw)
    return raw, warnings


# ─────────────────────────────────────────────────────────────────────────────
# bpy 建模（需要 Blender 运行时）
# ─────────────────────────────────────────────────────────────────────────────

def _require_bpy(action: str):
    if bpy is None:
        raise RuntimeError(
            f"{action} 需要 Blender 运行时（bpy），但当前环境未安装 Blender。"
            "纯校验请用 validate_spec() 或 `python asset_pipeline.py --validate-only`。"
        )


def _clear_scene():
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)


def _make_main(geom):
    """按 PropGeometry.main 建低多边形主体网格（参数与 generate_island.py 同源）。"""
    main = geom["main"]
    detail = geom["detail"]
    # detail 档位 → 段数（1=极简 8 面、2=标准 32 面、3=精细 64 面，与 propsSpec 注释一致）
    seg = 6 if detail == 1 else (12 if detail == 2 else 20)

    if main == "octahedron":
        bpy.ops.mesh.primitive_octahedron_add(size=0.7)
    elif main == "cone":
        bpy.ops.mesh.primitive_cone_add(radius1=0.55, depth=1.3, vertices=seg)
    elif main == "sphere":
        bpy.ops.mesh.primitive_uv_sphere_add(radius=0.55, segments=seg, ring_count=max(4, seg // 2))
    elif main == "tower":
        bpy.ops.mesh.primitive_cylinder_add(radius1=0.28, radius2=0.42, depth=1.8, vertices=seg)
    elif main == "flower":
        bpy.ops.mesh.primitive_uv_sphere_add(radius=0.45, segments=seg, ring_count=max(4, seg // 2))
    else:
        raise SpecError(f"geometry.main 未知：{main!r}")
    return bpy.context.active_object


def _make_accent(accent):
    """按 PropGeometry.accent 建装饰件（可选，渲染层可省略）。"""
    if accent == "prism":
        bpy.ops.mesh.primitive_cone_add(radius1=0.18, depth=0.6, vertices=4)
    elif accent == "scroll":
        bpy.ops.mesh.primitive_cylinder_add(radius1=0.12, radius2=0.12, depth=1.0, vertices=8)
    elif accent == "berry":
        bpy.ops.mesh.primitive_uv_sphere_add(radius=0.16, segments=8, ring_count=6)
    elif accent == "lamp":
        bpy.ops.mesh.primitive_uv_sphere_add(radius=0.2, segments=10, ring_count=8)
    elif accent == "petal":
        bpy.ops.mesh.primitive_torus_add(major_radius=0.4, minor_radius=0.1, major_segments=12, minor_segments=6)
    else:
        raise SpecError(f"geometry.accent 未知：{accent!r}")
    return bpy.context.active_object


def _make_material(spec):
    """按 spec.material 建 PBR 材质（名称规范 marguerite_<id>_mat）。"""
    mat_cfg = spec.get("material", {})
    mat = bpy.data.materials.new(name=f"{NAME_PREFIX}{spec['id']}_mat")
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    if bsdf is not None:
        if "color" in mat_cfg:
            r, g, b = mat_cfg["color"]
            bsdf.inputs["Base Color"].default_value = (r, g, b, 1.0)
        if "metallic" in mat_cfg:
            bsdf.inputs["Metallic"].default_value = float(mat_cfg["metallic"])
        if "roughness" in mat_cfg:
            bsdf.inputs["Roughness"].default_value = float(mat_cfg["roughness"])
        if "emissive" in mat_cfg:
            er, eg, eb = mat_cfg["emissive"]
            strength = float(mat_cfg.get("emissiveStrength", 1.0))
            bsdf.inputs["Emission Color"].default_value = (er, eg, eb, 1.0)
            bsdf.inputs["Emission Strength"].default_value = strength
    return mat


def _uv_unwrap(obj):
    """自动 UV 展开（Smart UV Project），保证 glTF 侧贴图坐标可用。"""
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.mode_set(mode='EDIT')
    bpy.ops.mesh.select_all(action='SELECT')
    bpy.ops.uv.smart_project(angle_limit=1.15)
    bpy.ops.object.mode_set(mode='OBJECT')


def build_mesh(spec):
    """
    按资产规格建低多边形网格（主体 + 可选装饰件），命名 marguerite_<id>，
    UV 展开并挂材质。返回主对象（装饰件已 join 进主对象，保持单网格便于实例化）。
    """
    _require_bpy("build_mesh()")
    warnings = validate_spec(spec)  # 双保险：即使上层漏校验，这里也拦一道
    geom = spec["geometry"]

    _clear_scene()
    scale = float(geom["scale"])

    main_obj = _make_main(geom)
    main_obj.name = f"{NAME_PREFIX}{spec['id']}"
    main_obj.scale = (scale, scale, scale)

    accent_obj = None
    if geom.get("accent"):
        accent_obj = _make_accent(geom["accent"])
        # 装饰件放在主体上方（与 webui scene.ts 的 buildProp 布局一致：accent.y = 1.5*scale）
        accent_obj.location = (0.0, 1.5 * scale, 0.0)
        accent_obj.scale = (scale * 0.8, scale * 0.8, scale * 0.8)

    # join 装饰件进主对象 → 单 mesh，webui 侧一次 load 即得完整道具
    if accent_obj is not None:
        bpy.ops.object.select_all(action='DESELECT')
        accent_obj.select_set(True)
        main_obj.select_set(True)
        bpy.context.view_layer.objects.active = main_obj
        bpy.ops.object.join()

    mat = _make_material(spec)
    main_obj.data.materials.append(mat)

    _uv_unwrap(main_obj)
    return main_obj


def _apply_decimate(obj, ratio: float):
    """按 decimateRatio 减面（LOD 生成），比例来自 spec.lod[]。"""
    if ratio >= 1.0:
        return
    bpy.context.view_layer.objects.active = obj
    mod = obj.modifiers.new(name="lod_decimate", type='DECIMATE')
    mod.ratio = ratio
    bpy.ops.object.modifier_apply(modifier=mod.name)


def export_glb(out_path: str, spec=None):
    """
    调 glTF 导出器导出 GLB。若 spec.lod 给出多档 LOD，
    对每个 LOD 复制网格减面后以 `<id>_lod<level>` 命名一并导出（单文件多变体）。
    """
    _require_bpy("export_glb()")

    out_dir = os.path.dirname(out_path)
    if out_dir:
        os.makedirs(out_dir, exist_ok=True)

    # LOD 变体：LOD0 是完整精度，更高级别按 decimateRatio 逐档减面
    if spec is not None and spec.get("lod"):
        lods = sorted(spec["lod"], key=lambda e: e["level"])
        base = bpy.context.active_object
        for entry in lods[1:]:  # LOD0 即 base，无需复制
            dup = base.copy()
            dup.data = base.data.copy()
            dup.name = f"{NAME_PREFIX}{spec['id']}_lod{entry['level']}"
            bpy.context.collection.objects.link(dup)
            _apply_decimate(dup, float(entry.get("decimateRatio", 0.5)))

    bpy.ops.export_scene.gltf(filepath=out_path, export_format='GLB')
    return out_path


# ─────────────────────────────────────────────────────────────────────────────
# CLI 入口
# ─────────────────────────────────────────────────────────────────────────────

def _parse_args(argv):
    """解析 `--` 之后的参数（Blender 透传形式：`blender --background --python asset_pipeline.py -- --spec x --out y`）。"""
    if "--" in argv:
        argv = argv[argv.index("--") + 1:]
    else:
        # 裸 python 调用时直接用 sys.argv[1:]
        argv = argv[1:]

    parser = argparse.ArgumentParser(prog="asset_pipeline", description="Marguerite 道具资产工厂")
    parser.add_argument("--spec", required=True, help="资产规格 JSON 路径")
    parser.add_argument("--out", help="输出 GLB 路径（--validate-only 时可省略）")
    parser.add_argument("--validate-only", action="store_true", help="只做纯校验，不建模/导出（无需 Blender）")
    return parser.parse_args(argv)


def main(argv=None):
    argv = list(sys.argv if argv is None else argv)
    args = _parse_args(argv)

    try:
        spec, warnings = load_asset_spec(args.spec)
    except SpecError as e:
        print(f"[asset_pipeline] 校验失败：{e}", file=sys.stderr)
        return 1
    except OSError as e:
        print(f"[asset_pipeline] 读取失败：{e}", file=sys.stderr)
        return 1

    for w in warnings:
        print(f"[asset_pipeline] 警告：{w}")

    if args.validate_only:
        print(f"[asset_pipeline] 校验通过：{args.spec}")
        return 0

    if not args.out:
        print("[asset_pipeline] 错误：非 --validate-only 模式必须提供 --out", file=sys.stderr)
        return 1

    try:
        build_mesh(spec)
        export_glb(args.out, spec)
    except RuntimeError as e:
        print(f"[asset_pipeline] 执行失败：{e}", file=sys.stderr)
        return 1

    print(f"[asset_pipeline] 导出成功 → {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
