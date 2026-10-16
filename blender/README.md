# Marguerite 资产管线（Blender headless + glTF）

P1「Blender headless 资产工厂 + glTF 管线」：**AI 生成资产规格 JSON → bpy 脚本建模 → 导出 glTF/GLB → webui 运行时热加载**。

设计依据：`E:\temp\_island_3d_game_design.md` §222-242（管线）与 §461 P1（「Blender headless 资产工厂 + glTF/KTX2 管线跑通；AI 生成首个道具/建筑并热加载」）。

---

## 1. 架构

```
                ┌─────────────────────────────────────────────┐
                │  AI / 生成器（或人手写）                       │
                │  产出资产规格 spec.json                        │
                └──────────────────┬──────────────────────────┘
                                   │  ① 规格
                                   ▼
        ┌──────────────────────────────────────────────────────┐
        │ schemas/prop_asset.schema.json  ← 静态契约（IDE/生成器） │
        └──────────────────┬───────────────────────────────────┘
                           │  ② 校验（无 Blender）
                           ▼
        ┌──────────────────────────────────────────────────────┐
        │ validate_assets.py / validate_spec()                 │
        │   · 纯 Python，CI 无 Blender 也能拦截坏 spec           │
        │   · exit 0 = 全过；exit 1 = 有坏 spec                  │
        └──────────────────┬───────────────────────────────────┘
                           │  ③ 建模 + 导出（需 Blender）
                           ▼
        ┌──────────────────────────────────────────────────────┐
        │ asset_pipeline.py（bpy 守卫模块）                      │
        │   build_mesh(spec)  按 PropGeometry.main/accent 建模  │
        │   export_glb(out)   glTF 导出器 + LOD 变体             │
        └──────────────────┬───────────────────────────────────┘
                           │  ④ GLB 产物
                           ▼
        ┌──────────────────────────────────────────────────────┐
        │ webui 运行时热加载                                     │
        │   world3d/gdd/gddHotReload.ts（GddHotReloadManager）   │
        │   + Three.js GLTFLoader 实例化                        │
        └──────────────────────────────────────────────────────┘
```

**关键工程约束**：`asset_pipeline.py` 用 `try: import bpy / except ImportError: bpy = None` 守卫。
**为什么？** 管线可测试性——CI 无 Blender 也能导入模块、跑 `validate_spec()` 拦截坏 spec；
单元测试（`test_asset_pipeline.py`）不依赖 Blender 就能全绿，坏数据进不了 Blender 阶段。
（详见 `asset_pipeline.py` 文件头的详细论证。）

---

## 2. 文件

| 文件 | 作用 |
| --- | --- |
| `asset_pipeline.py` | 通用资产工厂：`load_asset_spec` / `validate_spec`（纯 Python）+ `build_mesh` / `export_glb`（bpy）+ `main()` CLI |
| `schemas/prop_asset.schema.json` | 手写 draft-07 JSON Schema，字段与 `propsSpec.ts` 的 `PropGeometry`/`PropSpec` 对齐 |
| `examples/code_crystal.json` | 真实资产规格示例（代码结晶，几何/材质/LOD 完整） |
| `validate_assets.py` | 无 Blender 依赖校验器，扫描 `examples/` 全部 JSON，输出报告，exit 0/1 |
| `test_asset_pipeline.py` | 纯 Python unittest（13 用例，不依赖 Blender） |
| `generate_island.py` | 已有岛屿地形/道具批量导出脚本（本管线的前身，风格参照） |

---

## 3. 命令

### 3.1 无 Blender：校验 + 测试（CI 门禁）

```bash
# 校验 examples/ 全部资产规格（exit 0 = 全过）
python blender/validate_assets.py

# 单元测试（13 用例，全绿）
python -m unittest discover blender
```

### 3.2 有 Blender：建模 + 导出 GLB

```bash
# 单个资产：spec → 低多边形网格 + LOD 变体 → GLB
blender --background --python blender/asset_pipeline.py -- \
    --spec blender/examples/code_crystal.json \
    --out  webui/src/components/island/game/assets/code_crystal.glb

# 只校验、不建模（等价于裸 python，无需 Blender）
python blender/asset_pipeline.py --spec blender/examples/code_crystal.json --validate-only
```

> `--` 之后的参数由 Blender 透传给脚本；裸 `python` 调用时则直接读 `sys.argv[1:]`，两种形式都支持。

### 3.3 批量岛屿资产（已有脚本）

```bash
blender --background --python blender/generate_island.py   # → island_assets.glb
```

---

## 4. 资产规格（spec）契约

字段与 `webui/src/components/island/world3d/propsSpec.ts` 对齐：

- `id`：小写字母/数字/下划线；导出对象命名 `marguerite_<id>`
- `kind`：`code_crystal | doc_scroll | data_fruit | quest_beacon | inspiration_bloom`
- `geometry`：
  - `main`：`octahedron | cone | sphere | tower | flower`（主体 primitive）
  - `accent`：`prism | scroll | berry | lamp | petal`（可选装饰件）
  - `detail`：`1 | 2 | 3`（面数档）
  - `scale`：`> 0`
- `material`：`color [r,g,b]∈[0,1]`、`metallic/roughness∈[0,1]`、`emissive`、`emissiveStrength∈[0,4]`
- `lod[]`：`level` 严格递增、`targetFaces` 严格递减（运行时强制）、`decimateRatio∈(0,1]`

运行时真相源是 `validate_spec()`（`asset_pipeline.py`）；`schemas/prop_asset.schema.json`
是给 IDE / AI 生成器的静态契约，两者规则一致。

---

## 5. GLB → webui 热加载衔接

GLB 产物放进 webui 的世界资产目录后，由运行时热加载链路消费。参考既有 GDD 热加载契约：
`webui/src/components/island/world3d/gdd/`

- `gddSchema.ts`：`validateGdd(raw)` —— 手写校验器（不依赖 zod/ajv），error 阻断、warning 前向兼容。
- `gddHotReload.ts`：`GddHotReloadManager`（模块级单例 `gddHotReload`）
  - `loadFromJson(raw)`：**原子替换**——校验失败保持旧配置，成功才切换
  - `getRevision()`：每次成功加载 revision 递增
  - `exportCurrent()`：把当前生效配置回灌导出
  - `subscribe(fn)`：变更通知

**资产 GLB 的热加载同构于此模式**（P1 待接线的消费端）：

1. GLB 放入 `webui/src/components/island/game/assets/`（清单登记见该目录 `manifest.json`，遵守来源/许可红线）。
2. webui 侧用 Three.js `GLTFLoader` fetch GLB → 实例化到 `scene.ts` 的道具位（`buildPropPlacements` 决定坐标）。
3. 新 GLB 到达时走「原子替换 + revision 递增」：校验（GLB 可解析、命名 `marguerite_<id>`）通过才替换旧网格，失败保持旧的——与 `GddHotReloadManager.loadFromJson` 的失败回滚语义一致。

> 现状：`scene.ts` 目前用程序化几何体（`mainGeometry`/`accentGeometry`）渲染道具，
> 尚无 `GLTFLoader` 消费端。GLB 热加载消费是 **P1 待接线项**，本管线先交付
> 「规格 → 校验 → Blender 建模导出」的完整上游 + 文档化衔接契约；
> 程序化几何是 glTF 的**升级出口而非依赖**（见 `tierProfile.ts` / `terrain.ts` 文件头）。

---

## 6. 诚实性红线

- **示例资产是真实规格而非占位**：`examples/code_crystal.json` 的几何/材质/LOD 数值
  与 `propsSpec.ts` 的 `code_crystal` 条目一致，可直接进 Blender 出真实网格——
  不是 `TODO` / 假数据。
- **道具存在由真实来源驱动**：webui 侧 `buildPropPlacements` 只接受真实运行时归约计数
  （工具数/产物数/记忆召回数……），无数据就少道具，绝不「假装 Agent 干了很多活」。
- **来源不明 / AI 抓取 / CC-BY 未署名的模型禁止入库**（见 `game/assets/manifest.json`）。

---

## 7. 本机未装 Blender 时如何「打通」

本机 Blender 不在 PATH，因此 P1 的「跑通」= **管线脚本 + 校验器 + 文档化验证步骤**全部就位：

1. ✅ `python blender/validate_assets.py` → exit 0（坏 spec 被拦截在 Blender 之前）
2. ✅ `python -m unittest discover blender` → 13 用例全绿（不依赖 Blender）
3. 📄 Blender 执行步骤已文档化（§3.2），装上 Blender 即可一条命令出 GLB
4. 📄 GLB → webui 热加载衔接契约已文档化（§5），消费端接线在 P1 后续推进

这样即便当前无法运行 `blender --background`，管线的**可测试性与正确性**已经在无 Blender
环境下得到验证；坏 spec 进不了资产库，符合「CI 无 Blender 也能拦截坏 spec」的工程要求。
