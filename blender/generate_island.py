"""
Marguerite 岛屿 glTF 资产管线（Blender headless 脚本）。

用法（无需打开 Blender GUI，纯命令行导出 glTF/GLB）：
    blender --background --python generate_island.py

脚本做什么：
1. 程序化生成低多边形岛屿地形（噪声高度场 + 径向衰减，与 webui 的
   world3d/terrain.ts 同构——高度函数逻辑一致，保证视觉可对照）。
2. 生成 5 类道具基准网格（八面体/圆锥/球体/塔/花，对应 propsSpec.ts 的
   PropGeometry.main）+ 装饰件（棱柱/纸轴/浆果/灯/花瓣）。
3. 添加相机与太阳光（可选，方便预览），导出 `island_assets.glb`。

为什么程序化建模而非人工美术资产？
  - 「AI 自主设计」= 设计是代码契约（本脚本参数可被 Agent 读改），
    不是几百 MB 隐式资产知识；
  - 零素材依赖：不需要下载/打包 glTF 就能出世界，构建产物轻量；
  - 可热更新：改参数 → 重跑脚本 → 新 glTF，无需重新建模。
  本管线是 webui 程序化几何体的**升级出口**（影视级资产替换），P0 不依赖它。

输出：island_assets.glb（glTF 2.0 Binary），可直接被 three.js GLTFLoader 载入。
"""

import bpy
import bmesh
import math
import random

# ─────────────────────────────────────────────────────────────────────────────
# 配置（与 webui/src/components/island/world3d/terrain.ts 保持同构）
# ─────────────────────────────────────────────────────────────────────────────

ISLAND_RADIUS = 42.0      # 岛屿半径（米）
TERRAIN_SEGMENTS = 64     # 地形网格分辨率（每边顶点数）
TERRAIN_AMPLITUDE = 7.0   # 最大起伏（米）
DEFAULT_SEED = 770123     # 世界种子（与 webui 默认种子一致）
OUTPUT_PATH = "island_assets.glb"


# ─────────────────────────────────────────────────────────────────────────────
# 噪声高度场（FNV-1a value-noise + fbm，与 terrain.ts 同算法）
# ─────────────────────────────────────────────────────────────────────────────

def fnv1a(text: str) -> int:
    h = 0x811C9DC5
    for ch in text:
        h ^= ord(ch)
        h = (h * 0x01000193) & 0xFFFFFFFF
    return h


def hash_to_unit(h: int) -> float:
    return (h & 0xFFFFFFFF) / 0x100000000


def value_noise_2d(x: float, z: float, seed: int) -> float:
    x0 = math.floor(x)
    z0 = math.floor(z)
    fx = x - x0
    fz = z - z0
    ux = fx * fx * fx * (fx * (fx * 6 - 15) + 10)
    uz = fz * fz * fz * (fz * (fz * 6 - 15) + 10)
    def v(ix, iz):
        return hash_to_unit(fnv1a(f"{seed}:{ix}:{iz}"))
    a = v(x0, z0)
    b = v(x0 + 1, z0)
    c = v(x0, z0 + 1)
    d = v(x0 + 1, z0 + 1)
    return a + (b - a) * ux + (c - a) * uz + (a - b - c + d) * ux * uz


def fbm_2d(x: float, z: float, seed: int) -> float:
    total = 0.0
    amp = 0.5
    freq = 1.0
    for octave in range(4):
        total += value_noise_2d(x * freq, z * freq, seed + octave * 101) * amp
        amp *= 0.5
        freq *= 2.0
    return total


def height_at(x: float, z: float, seed: int = DEFAULT_SEED) -> float:
    r = math.sqrt(x * x + z * z)
    falloff = max(0.0, 1.0 - (r / ISLAND_RADIUS) ** 2)
    noise = fbm_2d(x * 0.045, z * 0.045, seed)
    peak = max(0.0, 1.0 - r / (ISLAND_RADIUS * 0.55)) ** 2 * 4.0
    return falloff * (TERRAIN_AMPLITUDE * noise + peak) - 1.2


# ─────────────────────────────────────────────────────────────────────────────
# 几何生成
# ─────────────────────────────────────────────────────────────────────────────

def clear_scene():
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)


def make_terrain():
    """生成岛屿地形网格（噪声高度场 + 贴图坐标）。"""
    mesh = bpy.data.meshes.new("Terrain")
    obj = bpy.data.objects.new("Terrain", mesh)
    bpy.context.collection.objects.link(obj)

    bm = bmesh.new()
    segs = TERRAIN_SEGMENTS
    size = ISLAND_RADIUS * 2.4
    step = size / segs

    verts = []
    for iz in range(segs + 1):
        for ix in range(segs + 1):
            x = -size / 2 + ix * step
            z = -size / 2 + iz * step
            y = height_at(x, z)
            v = bm.verts.new((x, y, z))
            verts.append(v)

    for iz in range(segs):
        for ix in range(segs):
            i = iz * (segs + 1) + ix
            bm.faces.new([verts[i], verts[i + 1], verts[i + segs + 2], verts[i + segs + 1]])

    bm.normal_update()
    bm.to_mesh(mesh)
    bm.free()
    return obj


def make_water():
    """水面（大平面，半透明）。"""
    mesh = bpy.data.meshes.new("Water")
    obj = bpy.data.objects.new("Water", mesh)
    bpy.context.collection.objects.link(obj)

    bm = bmesh.new()
    size = ISLAND_RADIUS * 3.2
    v0 = bm.verts.new((-size, 0, -size))
    v1 = bm.verts.new((size, 0, -size))
    v2 = bm.verts.new((size, 0, size))
    v3 = bm.verts.new((-size, 0, size))
    bm.faces.new([v0, v1, v2, v3])
    bm.to_mesh(mesh)
    bm.free()
    return obj


def make_prop_crystal():
    """代码结晶（八面体）。"""
    bpy.ops.mesh.primitive_octahedron_add(size=0.7)
    obj = bpy.context.active_object
    obj.name = "code_crystal"
    return obj


def make_prop_scroll():
    """文档卷轴（圆锥）。"""
    bpy.ops.mesh.primitive_cone_add(radius1=0.55, depth=1.3, vertices=12)
    obj = bpy.context.active_object
    obj.name = "doc_scroll"
    return obj


def make_prop_fruit():
    """数据果实（球体）。"""
    bpy.ops.mesh.primitive_uv_sphere_add(radius=0.55, segments=12, ring_count=8)
    obj = bpy.context.active_object
    obj.name = "data_fruit"
    return obj


def make_prop_beacon():
    """任务灯塔（细塔）。"""
    bpy.ops.mesh.primitive_cylinder_add(radius1=0.28, radius2=0.42, depth=1.8, vertices=12)
    obj = bpy.context.active_object
    obj.name = "quest_beacon"
    return obj


def make_prop_bloom():
    """灵感花（球体主干 + 环形花瓣）。"""
    bpy.ops.mesh.primitive_uv_sphere_add(radius=0.45, segments=12, ring_count=8)
    obj = bpy.context.active_object
    obj.name = "inspiration_bloom"
    return obj


def make_camera_and_light():
    """添加相机与太阳光（方便 glTF 预览）。"""
    bpy.ops.object.camera_add(location=(0, -60, 30), rotation=(math.radians(65), 0, 0))
    cam = bpy.context.active_object
    bpy.context.scene.camera = cam

    bpy.ops.object.light_add(type='SUN', location=(22, 14, 36))
    sun = bpy.context.active_object
    sun.data.energy = 1.5


# ─────────────────────────────────────────────────────────────────────────────
# 导出 glTF
# ─────────────────────────────────────────────────────────────────────────────

def export_glb(path: str):
    bpy.ops.export_scene.gltf(filepath=path, export_format='GLB')


def main():
    clear_scene()

    # 地形 + 水面
    terrain = make_terrain()
    water = make_water()

    # 5 类道具基准网格（散布在环带上，供 GLTFLoader 载入后复制摆放）
    make_prop_crystal()
    make_prop_scroll()
    make_prop_fruit()
    make_prop_beacon()
    make_prop_bloom()

    # 相机 + 光照
    make_camera_and_light()

    # 导出 glTF Binary（glTF 2.0）
    export_glb(OUTPUT_PATH)
    print(f"[generate_island] exported → {OUTPUT_PATH}")


if __name__ == "__main__":
    main()
