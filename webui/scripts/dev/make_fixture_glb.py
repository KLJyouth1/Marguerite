#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
make_fixture_glb.py —— 不依赖 Blender 的最小 glTF 2.0 / GLB 生成器（消费端测试夹具）。

为什么要手写 GLB 字节，而不用 Blender 或 mock 掉 loader？
────────────────────────────────────────────────────────────────────────
本机没有 Blender（`blender` 不在 PATH），但消费端
`src/components/island/world3d/glbHotReload.ts` 的链路是
「真实 .glb 二进制 → GLTFLoader 解析 → validateGlbScene 校验 → 可入库模板」。

如果测试里 mock 掉 GLTFLoader，证明的只是「我们自己的 mock 能跑」——
真正的 GLTFLoader 能不能吃下我们产出的字节、accessor 的 min/max 对不对、
bufferView / 索引对齐合不合法，全都**没被验证**。所以这里用**纯标准库**
手写一份合法的 GLB 二进制（12 字节 header + JSON chunk + BIN chunk），
当作「真实字节」夹具：测试直接 fs 读文件 → GLTFLoader.parse → 断言数值。

字段语义对齐 blender/schemas/prop_asset.schema.json（Blender 管线的忠实替身，
只是用 Python 而非 bpy 建模）：
    geometry.main ∈ octahedron / cone / sphere / tower / flower
    geometry.detail ∈ {1, 2, 3}
    geometry.scale  > 0

用法：
    python scripts/dev/make_fixture_glb.py --kind code_crystal --out <path>.glb
    python scripts/dev/make_fixture_glb.py --budget-bust     --out <path>.glb
脚本末尾会对刚写出的文件做一次纯 Python 结构自检，打印 PASS/FAIL 并 exit 0/1。
"""

import argparse
import array
import json
import math
import os
import struct
import sys

# ── 契约常量（与 propsSpec.ts / prop_asset.schema.json 同源）──────────────
KINDS = ("code_crystal", "doc_scroll", "data_fruit", "quest_beacon", "inspiration_bloom")
MAIN_BY_KIND = {
    "code_crystal": "octahedron",
    "doc_scroll": "cone",
    "data_fruit": "sphere",
    "quest_beacon": "tower",
    "inspiration_bloom": "flower",
}
# detail 档位 → 段数（与 blender/asset_pipeline.py 的 seg 取值一致：6/12/20）
SEG_BY_DETAIL = {1: 6, 2: 12, 3: 20}

# 超预算夹具：高分辨率球体，三角形数 2*seg*(rings-1) 明显 > 默认预算 50000，
# 但顶点数 < 65536 仍可用 UNSIGNED_SHORT 索引（保持结构与普通夹具同构）。
OVERSIZED_SEG = 320
OVERSIZED_RINGS = 96

# glTF 常量
COMPONENT_FLOAT = 5126   # FLOAT
COMPONENT_USHORT = 5123  # UNSIGNED_SHORT
TARGET_ARRAY_BUFFER = 34962
TARGET_ELEMENT_ARRAY_BUFFER = 34963
CHUNK_JSON = b"JSON"
CHUNK_BIN = b"BIN\x00"


# ───────────────────────── 几何生成（低多边形 primitive）─────────────────────────

def _subdivide(positions, indices):
    """把每个三角形细分成 4 个（纯平面中点细分），用于按 detail 提升面数。"""
    pos = [list(p) for p in positions]
    cache = {}
    out = []

    def midpoint(a, b):
        key = (a, b) if a < b else (b, a)
        hit = cache.get(key)
        if hit is not None:
            return hit
        pa, pb = pos[a], pos[b]
        pos.append([(pa[0] + pb[0]) * 0.5, (pa[1] + pb[1]) * 0.5, (pa[2] + pb[2]) * 0.5])
        cache[key] = len(pos) - 1
        return cache[key]

    for i in range(0, len(indices), 3):
        a, b, c = indices[i], indices[i + 1], indices[i + 2]
        ab, bc, ca = midpoint(a, b), midpoint(b, c), midpoint(c, a)
        out.extend([a, ab, ca, b, bc, ab, c, ca, bc, ab, bc, ca])
    return pos, out


def _octahedron(subdivisions):
    """八面体（code_crystal）。subdivisions 每加 1，面数 ×4：8 / 32 / 128。"""
    positions = [
        [0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0],
        [-1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, -1.0, 0.0],
    ]
    indices = [
        0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1,
        5, 2, 1, 5, 3, 2, 5, 4, 3, 5, 1, 4,
    ]
    for _ in range(subdivisions):
        positions, indices = _subdivide(positions, indices)
    return positions, indices


def _cone(seg, radius=0.55, height=1.3):
    """圆锥（doc_scroll）：seg 个侧面三角 + seg 个底面三角。"""
    half = height * 0.5
    positions = [[0.0, half, 0.0]]  # 顶点
    for j in range(seg):
        phi = 2.0 * math.pi * j / seg
        positions.append([radius * math.cos(phi), -half, radius * math.sin(phi)])
    base_center = len(positions)
    positions.append([0.0, -half, 0.0])
    indices = []
    for j in range(seg):
        a = 1 + j
        b = 1 + ((j + 1) % seg)
        indices += [0, a, b]            # 侧面
        indices += [base_center, b, a]  # 底面
    return positions, indices


def _uv_sphere(seg, rings, radius=0.55, petal_lobes=0):
    """UV 球（data_fruit）；petal_lobes>0 时按径向余弦起伏做成花瓣状（inspiration_bloom）。"""
    positions = [[0.0, radius, 0.0]]  # 北极
    for i in range(1, rings):
        theta = math.pi * i / rings
        y = radius * math.cos(theta)
        ring_r = radius * math.sin(theta)
        for j in range(seg):
            phi = 2.0 * math.pi * j / seg
            r = ring_r
            if petal_lobes:
                # 花瓣起伏系数 1±0.28 恒 > 0，保证任何一轴尺寸都不为 0（非退化包围盒）
                r = ring_r * (1.0 + 0.28 * math.cos(petal_lobes * phi))
            positions.append([r * math.cos(phi), y, r * math.sin(phi)])
    bottom = len(positions)
    positions.append([0.0, -radius, 0.0])  # 南极

    indices = []
    for j in range(seg):                    # 顶部扇形
        indices += [0, 1 + j, 1 + ((j + 1) % seg)]
    for i in range(rings - 2):              # 中间四边形带 → 两个三角
        row0 = 1 + i * seg
        row1 = 1 + (i + 1) * seg
        for j in range(seg):
            j2 = (j + 1) % seg
            a, b = row0 + j, row0 + j2
            c, d = row1 + j, row1 + j2
            indices += [a, c, b, b, c, d]
    last = 1 + (rings - 2) * seg            # 底部扇形
    for j in range(seg):
        indices += [bottom, last + j, last + ((j + 1) % seg)]
    return positions, indices


def _tube(seg, r_bottom, r_top, height):
    """圆台/圆柱（quest_beacon 灯塔塔身）。"""
    half = height * 0.5
    positions = []
    for j in range(seg):
        phi = 2.0 * math.pi * j / seg
        positions.append([r_bottom * math.cos(phi), -half, r_bottom * math.sin(phi)])
    for j in range(seg):
        phi = 2.0 * math.pi * j / seg
        positions.append([r_top * math.cos(phi), half, r_top * math.sin(phi)])
    cb = len(positions)
    positions.append([0.0, -half, 0.0])
    ct = len(positions)
    positions.append([0.0, half, 0.0])
    indices = []
    for j in range(seg):
        j2 = (j + 1) % seg
        b0, b1 = j, j2
        t0, t1 = seg + j, seg + j2
        indices += [b0, t0, t1, b0, t1, b1]  # 侧面
        indices += [cb, b1, b0]              # 底盖
        indices += [ct, t0, t1]              # 顶盖
    return positions, indices


def build_shape(main, detail, scale):
    """按 geometry.main / detail / scale 生成 (positions, indices)。"""
    seg = SEG_BY_DETAIL[detail]
    rings = max(4, seg // 2)
    if main == "octahedron":
        positions, indices = _octahedron(detail - 1)   # → 8 / 32 / 128 面
    elif main == "cone":
        positions, indices = _cone(seg)
    elif main == "sphere":
        positions, indices = _uv_sphere(seg, rings)
    elif main == "tower":
        positions, indices = _tube(seg, 0.28, 0.42, 1.8)
    elif main == "flower":
        positions, indices = _uv_sphere(seg, rings, petal_lobes=5)
    else:
        raise ValueError(f"geometry.main 未知：{main!r}")
    positions = [[p[0] * scale, p[1] * scale, p[2] * scale] for p in positions]
    return positions, indices


def build_oversized():
    """超预算网格（负例）：高分辨率球体，三角形数远超默认预算。"""
    return _uv_sphere(OVERSIZED_SEG, OVERSIZED_RINGS)


def compute_normals(positions, indices):
    """按面积加权累加面法线再归一化（平滑法线），保证 NORMAL 是真实数据。"""
    normals = [[0.0, 0.0, 0.0] for _ in positions]
    for i in range(0, len(indices), 3):
        a, b, c = indices[i], indices[i + 1], indices[i + 2]
        pa, pb, pc = positions[a], positions[b], positions[c]
        ux, uy, uz = pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]
        vx, vy, vz = pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]
        nx = uy * vz - uz * vy
        ny = uz * vx - ux * vz
        nz = ux * vy - uy * vx
        for idx in (a, b, c):
            normals[idx][0] += nx
            normals[idx][1] += ny
            normals[idx][2] += nz
    out = []
    for n in normals:
        ln = math.sqrt(n[0] * n[0] + n[1] * n[1] + n[2] * n[2])
        if ln < 1e-12:
            out.append([0.0, 1.0, 0.0])
        else:
            out.append([n[0] / ln, n[1] / ln, n[2] / ln])
    return out


# ───────────────────────── 二进制打包 ─────────────────────────

def _pack_f32(values):
    a = array.array("f", values)
    if sys.byteorder != "little":
        a.byteswap()
    return a.tobytes()


def _pack_u16(values):
    a = array.array("H", values)
    if sys.byteorder != "little":
        a.byteswap()
    return a.tobytes()


def build_glb(kind, positions, indices):
    """组装一份合法的 glTF 2.0 GLB 字节串。"""
    if max(indices) >= 65536:
        raise ValueError("顶点索引超出 UNSIGNED_SHORT 范围（本夹具约定用 UNSIGNED_SHORT）")

    flat_pos = [c for p in positions for c in p]
    normals = compute_normals(positions, indices)
    flat_nrm = [c for n in normals for c in n]

    pos_bytes = _pack_f32(flat_pos)
    nrm_bytes = _pack_f32(flat_nrm)
    idx_bytes = _pack_u16(indices)

    bin_data = pos_bytes + nrm_bytes + idx_bytes
    bin_pad = (4 - len(bin_data) % 4) % 4
    bin_data += b"\x00" * bin_pad          # BIN chunk 必须 4 字节对齐（补 0x00）

    pos_off = 0
    nrm_off = len(pos_bytes)
    idx_off = len(pos_bytes) + len(nrm_bytes)

    xs = [p[0] for p in positions]
    ys = [p[1] for p in positions]
    zs = [p[2] for p in positions]
    min_pos = [min(xs), min(ys), min(zs)]
    max_pos = [max(xs), max(ys), max(zs)]

    gltf = {
        "asset": {"version": "2.0", "generator": "make_fixture_glb.py (Marguerite 测试夹具，无 Blender)"},
        "scene": 0,
        "scenes": [{"name": f"{kind}_scene", "nodes": [0]}],
        "nodes": [{"name": f"marguerite_{kind}", "mesh": 0}],
        "meshes": [{
            "name": f"marguerite_{kind}",
            "primitives": [{
                "attributes": {"POSITION": 0, "NORMAL": 1},
                "indices": 2,
                "material": 0,
                "mode": 4,
            }],
        }],
        "materials": [{
            "name": f"marguerite_{kind}_mat",
            "pbrMetallicRoughness": {
                "baseColorFactor": [0.45, 0.65, 0.9, 1.0],
                "metallicFactor": 0.35,
                "roughnessFactor": 0.28,
            },
        }],
        "buffers": [{"byteLength": len(bin_data)}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": pos_off, "byteLength": len(pos_bytes), "target": TARGET_ARRAY_BUFFER},
            {"buffer": 0, "byteOffset": nrm_off, "byteLength": len(nrm_bytes), "target": TARGET_ARRAY_BUFFER},
            {"buffer": 0, "byteOffset": idx_off, "byteLength": len(idx_bytes), "target": TARGET_ELEMENT_ARRAY_BUFFER},
        ],
        "accessors": [
            {"bufferView": 0, "componentType": COMPONENT_FLOAT, "count": len(positions), "type": "VEC3",
             "min": min_pos, "max": max_pos},
            {"bufferView": 1, "componentType": COMPONENT_FLOAT, "count": len(positions), "type": "VEC3"},
            {"bufferView": 2, "componentType": COMPONENT_USHORT, "count": len(indices), "type": "SCALAR",
             "min": [0], "max": [max(indices)]},
        ],
    }

    json_bytes = json.dumps(gltf, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    json_pad = (4 - len(json_bytes) % 4) % 4
    json_chunk = json_bytes + (b" " * json_pad)  # JSON chunk 必须 4 字节对齐（补空格 0x20）

    total = 12 + 8 + len(json_chunk) + 8 + len(bin_data)
    header = struct.pack("<4sII", b"glTF", 2, total)
    json_header = struct.pack("<I4s", len(json_chunk), CHUNK_JSON)
    bin_header = struct.pack("<I4s", len(bin_data), CHUNK_BIN)
    return header + json_header + json_chunk + bin_header + bin_data


# ───────────────────────── 自检（纯 Python 结构解析）─────────────────────────

def self_check(path):
    """对刚写出的 GLB 做一次纯 Python 结构解析，返回 (ok, 信息行列表, 问题列表)。"""
    problems = []
    info = []
    with open(path, "rb") as f:
        raw = f.read()
    size = len(raw)
    if size < 20:
        return False, info, ["文件过小，不是合法 GLB"]

    magic = raw[0:4]
    version, length = struct.unpack("<II", raw[4:12])
    if magic != b"glTF":
        problems.append(f"header.magic 非法：{magic!r}")
    if version != 2:
        problems.append(f"header.version 应为 2，实际 {version}")
    if length != size:
        problems.append(f"header.length={length} 与实际文件大小 {size} 不一致")

    json_obj = None
    bin_data = None
    offset = 12
    while offset + 8 <= size:
        clen, ctype = struct.unpack("<I4s", raw[offset:offset + 8])
        offset += 8
        chunk = raw[offset:offset + clen]
        offset += clen
        if ctype == CHUNK_JSON:
            try:
                json_obj = json.loads(chunk.decode("utf-8"))
            except Exception as e:  # noqa: BLE001
                problems.append(f"JSON chunk 无法解析：{e}")
        elif ctype == CHUNK_BIN:
            bin_data = chunk
    if offset != size:
        problems.append(f"chunk 累计长度 {offset} 与文件大小 {size} 不一致")

    if json_obj is None:
        problems.append("缺少 JSON chunk")
        return False, info, problems
    if bin_data is None:
        problems.append("缺少 BIN chunk")
        return False, info, problems

    buffers = json_obj.get("buffers", [])
    if not buffers or buffers[0].get("byteLength") != len(bin_data):
        problems.append("buffers[0].byteLength 与 BIN chunk 长度不一致")

    comp_size = {COMPONENT_FLOAT: 4, COMPONENT_USHORT: 2}
    type_size = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}
    buffer_views = json_obj.get("bufferViews", [])
    accessors = json_obj.get("accessors", [])

    for ai, acc in enumerate(accessors):
        bv = buffer_views[acc["bufferView"]]
        expect = acc["count"] * comp_size[acc["componentType"]] * type_size[acc["type"]]
        if bv["byteLength"] != expect:
            problems.append(f"accessors[{ai}] 计数字节 {expect} 与 bufferView.byteLength {bv['byteLength']} 不一致")

    # POSITION 的 min/max 必须与真实顶点数据一致（GLTFLoader 会用到）
    pos_acc = accessors[0]
    bv = buffer_views[pos_acc["bufferView"]]
    off = bv.get("byteOffset", 0)
    count = pos_acc["count"]
    vals = struct.unpack_from("<%df" % (count * 3), bin_data, off)
    vx = vals[0::3]
    vy = vals[1::3]
    vz = vals[2::3]
    real_min = [min(vx), min(vy), min(vz)]
    real_max = [max(vx), max(vy), max(vz)]
    if [round(v, 6) for v in pos_acc.get("min", [])] != [round(v, 6) for v in real_min]:
        problems.append(f"POSITION.min 与实际顶点不符：{pos_acc.get('min')} vs {real_min}")
    if [round(v, 6) for v in pos_acc.get("max", [])] != [round(v, 6) for v in real_max]:
        problems.append(f"POSITION.max 与实际顶点不符：{pos_acc.get('max')} vs {real_max}")

    # 索引必须全部指向已存在的顶点
    idx_acc = accessors[2]
    ibv = buffer_views[idx_acc["bufferView"]]
    ioff = ibv.get("byteOffset", 0)
    icount = idx_acc["count"]
    idx = struct.unpack_from("<%dH" % icount, bin_data, ioff)
    if max(idx) >= count:
        problems.append(f"索引最大值 {max(idx)} 越界（顶点数 {count}）")

    tris = icount // 3
    info.append(f"顶点数 = {count}")
    info.append(f"三角形数 = {tris}")
    info.append(f"索引类型 = UNSIGNED_SHORT（max={max(idx)}）")

    ok = not problems
    return ok, info, problems


# ───────────────────────── CLI ─────────────────────────

def _parse_args(argv):
    parser = argparse.ArgumentParser(
        prog="make_fixture_glb",
        description="无 Blender 的最小 GLB 生成器（Marguerite 消费端测试夹具）",
    )
    parser.add_argument("--kind", choices=KINDS, default="code_crystal",
                        help="道具种类（决定 geometry.main，对齐 propsSpec.ts / schema）")
    parser.add_argument("--detail", type=int, choices=[1, 2, 3], default=2,
                        help="低多边形面数档（1/2/3，对齐 schema 的 geometry.detail）")
    parser.add_argument("--out", default=None, help="输出 GLB 路径（缺省为 <kind>.glb）")
    parser.add_argument("--budget-bust", action="store_true", dest="budget_bust",
                        help="生成超预算高面数网格（负例测试用，三角形数 > 50000）")
    return parser.parse_args(argv)


def main(argv=None):
    args = _parse_args(sys.argv[1:] if argv is None else argv)

    if args.budget_bust:
        kind = args.kind
        positions, indices = build_oversized()
    else:
        kind = args.kind
        scale = 0.8
        positions, indices = build_shape(MAIN_BY_KIND[kind], args.detail, scale)

    out = args.out or f"{kind}.glb"
    out_dir = os.path.dirname(out)
    if out_dir:
        os.makedirs(out_dir, exist_ok=True)

    blob = build_glb(kind, positions, indices)
    with open(out, "wb") as f:
        f.write(blob)

    print(f"[make_fixture_glb] 写入 {out}")
    print(f"[make_fixture_glb]   字节数 = {len(blob)}")
    print(f"[make_fixture_glb]   main = {MAIN_BY_KIND[kind] if not args.budget_bust else 'oversized-sphere(sphere)'}"
          f"  detail = {args.detail}{'  [--budget-bust]' if args.budget_bust else ''}")

    ok, info, problems = self_check(out)
    for line in info:
        print(f"[make_fixture_glb]   {line}")
    if ok:
        print("[make_fixture_glb] 自检 PASS")
        return 0
    print("[make_fixture_glb] 自检 FAIL：", file=sys.stderr)
    for p in problems:
        print(f"[make_fixture_glb]   - {p}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
