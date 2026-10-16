# scripts/dev — 开发期脚手架

## make_fixture_glb.py：无 Blender 的最小 GLB 夹具生成器

本机没有安装 Blender（`blender` 不在 PATH），但 `src/components/island/world3d/glbHotReload.ts`
的消费端链路是「真实 `.glb` 字节 → `GLTFLoader.parse` → `validateGlbScene` 校验 → 可入库模板」。
如果把 loader mock 掉，测出来的只是「自己的 mock 能跑」。所以这个脚本用**纯 Python 标准库**
手写一份合法的 glTF 2.0 / GLB 二进制（12 字节 header + JSON chunk + BIN chunk），
作为 `glbHotReload.test.ts` 里真实字节用例的输入，证明整条链真的能跑。

字段语义对齐 `blender/schemas/prop_asset.schema.json`：`geometry.main ∈
octahedron/cone/sphere/tower/flower`、`detail ∈ {1,2,3}`、`scale > 0`。
脚本写完文件后会做一次纯 Python 结构自检（header / chunk 长度 / JSON 可解析 /
accessor 计数一致 / 索引不越界），打印 PASS/FAIL 并以 0/1 退出。

### 重新生成夹具

```bash
# 正常低多边形夹具（八面体，detail 2 → 32 三角形，1668 字节）
python scripts/dev/make_fixture_glb.py \
  --kind code_crystal \
  --out src/components/island/world3d/assets/fixtures/code_crystal.glb

# 超预算负例（高分辨率球体，60800 三角形 > 默认预算 50000）
python scripts/dev/make_fixture_glb.py \
  --budget-bust \
  --out src/components/island/world3d/assets/fixtures/oversized.glb
```

在仓库根目录下调用也可以（脚本只用标准库、不依赖工作目录）：

```bash
python webui/scripts/dev/make_fixture_glb.py --kind code_crystal --out <tmp>.glb
```

参数：`--kind`（五种道具 kind 之一）、`--detail`（1/2/3）、`--out`（输出路径，
缺省 `<kind>.glb`）、`--budget-bust`（生成超预算高面数网格）。

### 两条产出路径的区别

| | 夹具（本脚本） | 真实资产（Blender 管线） |
|---|---|---|
| 生成器 | `scripts/dev/make_fixture_glb.py`（纯 Python，无 Blender） | `blender/asset_pipeline.py`（`bpy` headless 建模导出） |
| 落盘位置 | `src/components/island/world3d/assets/fixtures/` | `public/island3d/<kind>.glb` |
| 用途 | **消费端单元测试**：给 `glbHotReload.test.ts` 喂真实字节 | **运行时热加载**：Vite 复制进 `dist`，世界加载后热替换程序化几何 |
| 是否进 dist | 否（在 `src/**` 下，只被 vitest 引用） | 是（`public/` 原样复制进构建产物） |

夹具是「Blender 管线的忠实替身」，只为测试链路；真正给玩家看的资产必须由 Blender
管线产出并放进 `public/island3d/`，登记到 `assets/manifest.json`（见该文件的资产目录说明）。
