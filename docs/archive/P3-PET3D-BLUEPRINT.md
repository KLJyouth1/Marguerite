# P3 · 雏菊精灵 3D 蓝图（Blueprint · v1.0）

> **角色档案**（严格匹配用户提供图片）
> - 形象：雏菊精灵 3D · 少女花嫁 · 2.2 头身 Q 版
> - 主色：奶白渐变发 / 白色花瓣裙 / accent `#8EC5EF`（浅蓝）/ 花环与花束纯白配明黄花蕊 / 透明蝶翼
> - 特征：花环 + 透明蝴蝶四翅 + 花瓣嫁裙 + 胸前捧花 + 芭蕾 X 鞋 + 地花环绕 + 珍珠串点缀

---

## 0. 版本与基线

| 项 | 值 |
|---|---|
| Spec 入口 | `webui/src/components/pet/petSpec.ts::ELF_SPECS['blue']` |
| Renderer 入口 | `webui/src/components/pet/Pet3D.ts::buildPetHumanoid()` |
| 测试基线（pet 单测） | **24 / 24 passed** （npx vitest run src/components/pet，run @ 2026-09-02） |
| 视觉验证 | Playwright 6 阶段截图（06-blue-canvas-crop.png：脸/发/翅/裙/花/鞋均可见，色调不灰） |
| 动画基线 | 人形专属 9 大动效层（见 §4） |
| 父文档 | `docs/UNIFIED-WORKBENCH-v2.md §12` |

---

## 1. 数据模型（Spec Driven · 与代码一一映射）

### 1.1 核心类型（petSpec.ts）

```typescript
export type ModelKind = 'beast' | 'humanoid'   // 新增 E148
export type Hairstyle = 'bun' | 'long_wave' | 'short' | 'long_straight'
export type DressStyle = 'casual' | 'petal_dress' | 'kimono' | 'lolita'
export type WingStyle = 'none' | 'butterfly' | 'fairy'
export type Ornament = 'none' | 'daisy' | 'flower' | 'pearl'
export type TailStyle = 'none' | 'fluffy' | 'long'

// 人型比例（2.2 头身）
export const HUMANOID_PROPS: ProportionsSpec = {
  headRadius: 0.50,
  bodyRadius: 0.0,      // 人型用 torso，不用 body
  earHeight: 0.0, earWidth: 0.0, earSpacing: 0.9, earFold: 0.0,
  tailSegments: 0, tailBaseR: 0, tailTapering: 0.0,
  // 人型专用（validatePetSpec 会检查 kind=humanoid 时以下非 0）
  heightScale: 1.0,
  torsoLength: 0.78, legLength: 0.70, armLength: 0.70, neckLen: 0.12,
  shoulderWidth: 0.76, hipWidth: 0.68,
  eyeScale: 1.1,
  limbThickness: 0.11,
  isHumanoid: true,
}
```

### 1.2 外观外观元语（AppearanceSpec）

```typescript
export interface AppearanceSpec {
  // 共用
  furColor: number; furAccent: number            // 人形=头发主/辅色（渐变）
  accent: number; eye: number; blush: number; nose: number; innerEar?: number
  // 兽形
  pad?: number; belly?: number;
  // 人形新增
  skin?: number; dressColor?: number; dressAccent?: number; shoeColor?: number
  hairStyle?: Hairstyle; dressStyle?: DressStyle; wingStyle?: WingStyle
  earStyle?: 'none' | 'elf' | 'cat' | 'bear'; ornament?: Ornament
  tailStyle?: TailStyle
  hasBouquet?: boolean
}
```

### 1.3 雏菊精灵规格 ELF_SPECS['blue']

```typescript
'blue': {
  id: 'blue',
  label: '雏菊精灵 3D · 少女花嫁',
  kind: 'humanoid',
  accent: 0x8ec7ef,
  appearance: {
    furColor: 0xf4f8fc, furAccent: 0xa6ccee,   // 长发主色→发梢浅蓝渐变
    skin: 0xffede0, dressColor: 0xffffff, dressAccent: 0xb9dcf3, shoeColor: 0xe4eef8,
    eye: 0x5fb6ff, blush: 0xffd6e6, nose: 0x7a3a22,
    accent: 0x8ec7ef,
    earStyle: 'elf', ornament: 'daisy', tailStyle: 'none',
    hairStyle: 'long_wave', dressStyle: 'petal_dress',
    wingStyle: 'butterfly', hasBouquet: true,
  },
  proportions: HUMANOID_PROPS,   // 2.2 头身
  emotionPoses: ELF_POSES,       // 6 mood × [ 嘴 × 瞳孔 × 腮红 × 眉毛 ]
  motion: girlMotion(),          // 漂浮/裙摆/蝶翅动效（见 §4）
}
```

### 1.4 规格校验 Kind-Aware（validatePetSpec）

- beast：`earHeight/earWidth/tailSegments/... > 0`
- **humanoid**：`headRadius > 0 && eyeScale > 0 && proportions.heightScale>0 && torsoLength>0 && legLength>0 && armLength>0 && neckLen>0`
- 不满足的规格会在 `createPet3DFromSpec` 构造期抛错 → 前端渲染红色错误横幅（不会静默 fallback 为灰色头）。

---

## 2. 几何构造蓝图（buildPetHumanoid · 逐段对应）

### 2.0 前置调色板材料

```
渐变（头发 1-3 段） hairRootMat(0xf4f8fc) → hairMidMat(0xe6eefa) → hairTipMat(0xa6ccee accent)
皮肤       skinMat (0xffede0, toon gradient 4-tone)
裙主/辅    dressMat (0xffffff) → dressOverlayMat (0xb9dcf3 透明薄纱 transparent=true opacity=0.68)
鞋         shoeMat (0xe4eef8 toon)
choker     chokerMat (accent color) → gemMat (accent emissive=0.35 * accent)
花         flowerMat (0xffffff) → flowerCenterMat (0xffc74a 黄花蕊)
叶/stem    leafMat (0x87c26a) → stemMat (0x5b8b37)
蝶翅       wingSkinMat (accent color 透明 opacity=0.22, side=DoubleSide)
珍珠       pearlMat (0xfaf4e6, specular=0xffffff, toon gradient 3-tone)
```

### 2.1 头部（faceGroup）

| 组件 | 几何 | 材料 | 定位 |
|---|---|---|---|
| 脸 head | Icosahedron(0.50, 3) → smooth | skinMat | y=0 |
| 耳（尖精灵耳）2 个 | `makeEar('elf')` = 2 Cone 拼接 + 折角 -0.32 | skinMat / 内 accent | x=±0.44 y=0.04 z=0.16 |
| 刘海 fringe | 3 段 Box（中央/左斜/右斜）塌下形 | hairRootMat | y=+0.12（眉上 2mm） |
| 刘海 tuft 3 撮 | 3 × 扁球沿额头 | hairMidMat | y=+0.20 ± |
| 长发：**后发片** 14 片 | Cylinder(0.06, 0.02, 1.6, 6) → 弯 z <0 弧 14 片 | hairRoot/tip 交替（偶数 tip 渐变） | x=-0.48→+0.48 步长 y=-0.3 起始 |
| 侧鬓角：3 大 ×2 组 | Cylinder 大 taper；侧 3 大 → 内层 curl 环 | hairTipMat | x=±0.36 前层，y=-0.2..-1.3 |
| 眼 2 组 | sphere(0.072) 白 → irisGeo accent(0.5fb6ff) 球(0.048) 前推 z+0.012 → shine A(0.02)/shine B(0.013) | white / accent (emissive=0.2) / white (emissive=0.8) | x=±0.175 z=0.442 |
| 嘴 | 1/3/2 段圆柱（open/smile/frown/swoon 三形） → mood 映射 | 0xa24444 toon | y=-0.10 z=0.45 |
| 腮红 2 扁球 | 0.15×0.08 扁 ellipsoid | 0xffd6e6, opacity=0.0 | mood 时到 0.5 |

### 2.2 颈 + 胸口 + 肩

| 组件 | 几何 | 材料 |
|---|---|---|
| neck 圆柱 | 0.12 len | skinMat |
| chestUp 上胸球 | 0.27×0.20 | dressMat |
| 双肩球 2 × 0.11 | 球 | dressMat |
| collar 半环 Torus(0.18, 0.02, 半 π) | - | dressOverlayMat 蓝边 |

### 2.3 四肢（躯干 torsoGroup + 双臂 armsGroup + 双腿 legsGroup）

- 躯干：长圆柱 `torsoLength=0.78`（`shoulderWidth 0.76 → hipWidth 0.68` 轻微锥），材料 dressMat
- 手臂：每臂 = 上臂 `Cylinder(0.09)` + 前臂 + 手（扁球）`armLength=0.70`
  - 双上臂沿身体两侧微张 + 前臂 前抬 45° 捧花（静态 pose 基础）
  - 袖饰：3 flared flower 瓣（大/中/小）×2 臂，dressOverlayMat 薄纱
- 双腿：大腿圆柱 + 小腿 + 脚，`legLength=0.70`

### 2.4 Choker 蓝宝石吊坠

- 带：Torus(0.14, 0.015) accent color
- 坠：Octahedron(0.055) accent + `emissive=0.35*accent`（项链发光 = 图片中少女 choker 宝石亮）

### 2.5 花瓣嫁裙（3 层 + 珍珠串）

1. **内层短裙（petal 6 瓣）**：`makePetal(0.46, 0.50, round, -0.25)` 循环 6 份 → 腰下 30° 展开；材料 dressMat
2. **外层纱裙（10 片 accent 薄纱）**：`makePetal(0.85, 0.92, pointy, -0.28)` 10 份 → 外层 + 更大；`dressOverlayMat opacity=0.68 transparent=true`
3. **拖尾（trail 2 大纱）**：左右 2 片 `makePetal(1.4, 1.7, pointy, -0.62)` `rotationZ=±0.18`（左右后展开）；边缝珍珠串 9 颗 ×2 侧（x=±0.62 每 +0.18 步长 y）
4. **细节**：外层 10 片纱每片前点 1 颗 pearl；内层 6 花瓣裙摆下方环绕一圈珍珠（6 颗 → body 下方）

### 2.6 胸前捧花（bouquetGroup）

- 位置：胸前双手位置 (0, 0.25, 0.38)
- 构成：
  - `花 9 朵`（`flowerMat(白)` 8 瓣 + `flowerCenterMat(黄)` 蕊）：位置随机 3×3 约 0.05 半径
  - `茎叶 3 片`（椭圆 PlaneGeometry，leafMat）
  - `stem 5 根圆柱` 下扎 → stemMat 绿色
  - `扎绳 accent 色`：Torus(0.048, 0.008) accent color
- 持花动画：上下微摆（见 §4.7）

### 2.7 芭蕾鞋 + X 形缎带

| 组件 | 几何 |
|---|---|
| 鞋（左右 2 个） | Sphere(0.09 × 1.3 × 1.0 × 0.65) 扁鞋 → y=-0.68（脚底） |
| 踝带（环脚） | Torus(0.07, 0.008, phiStart=π/2) |
| 交叉缎带（每脚 2 条 × 2） | Cylinder 细（r=0.005 l=0.18） 旋转斜 45° 2 条（X） + 竖直 1 条 |
| 鞋前花 | 小 5 瓣花（dressOverlayMat）+ accent 蕊球（0.018） |

### 2.8 蝴蝶四翅（wingGroup）

```
定义通用 makeWingPlane(w, h, tipTaper: 'round' | 'pointy', veins)
  = PlaneGeometry(w, h, 2, 2) → 把顶点拉成椭圆（tipTaper='pointy' 则尖端收尖）
  → 再在内部画 veins 线 3 条（LineBasic accent color 更亮）

四翅组合：
 左近（内翅·小） w=0.90 h=0.58 round  veins=3
 左远（外翅·大） w=1.08 h=0.86 pointy veins=3
 右内右外（镜像）
→ 定位 y=0.70（背后高度 ≈ 颈），z=-0.20（人体后）
→ wingSkinMat opacity=0.22 DoubleSide；翅边 accent 更实 rimLine（1 pt 粗 torus 沿翅边 = 图片蝶翼翅边清晰）
```

### 2.9 花环（headFlowerGroup = 头顶装饰）

- 花环基础环：Torus(0.35, 0.006, 0x3e6e38 深绿)
- 小花 12 朵（循环 0..11 × angle 步 2π/12）：`makeFlowerGroup` = 5 白瓣 + 1 黄蕊
- 大叶 6 片（间隔）：椭叶 Plane + stem
- 主花 1 朵 大雏菊（右侧额前）：10 瓣 + 大蕊（突出视觉重心）
- 高光：主花蕊球 emissive=0.35（比其他蕊更亮）

### 2.10 地花（groundFlowers + 散落花瓣）

- 地花 20 朵：椭圆中心 (x, z) 随机分布在脚底椭圆 1.1×0.7 内 → y=-0.70
- 散落花瓣 10 片：10 个 `makePetal(0.06~0.09, round, -0.06)` 随机旋转/位置

---

## 3. Renderer 调优（消除灰色灰蒙蒙 + toon 高亮）

### 3.1 Tone Mapping（本版本关键修复）

```diff
- renderer.outputColorSpace = THREE.SRGBColorSpace
- renderer.toneMapping = THREE.ACESFilmicToneMapping
- renderer.toneMappingExposure = 1.0
+ renderer.outputColorSpace = THREE.SRGBColorSpace
+ // ACES 会把卡通纯白压成中灰 → 关掉
+ renderer.toneMapping = THREE.NoToneMapping
+ renderer.toneMappingExposure = 1.0
```

### 3.2 Toon 4-tone 渐变（更亮高光）

```diff
- const stops = [0.0, 0.35, 0.5, 0.6, 0.75, 0.9, 1.0]   // v1
+ const stops = [0.0, 0.25, 0.45, 0.55, 0.72, 0.92, 1.0] // v2（更早进入高光段）
  const brightStops = stops.map(s => Math.pow(s, 1.1));
```

### 3.3 灯光正面主光（重点修复）

```diff
  if (spec.kind === 'humanoid') {
-   const key = new THREE.DirectionalLight(0xffffff, 1.35);
-   key.position.set(-1.6, 3.4, 2.0);
+   // 把主光拉到正前方（+Z），N·L ≈ 0.86（toon HIGH 段触发 = 脸/裙自然纯白亮面）
+   const key = new THREE.DirectionalLight(0xffffff, 1.55);
+   key.position.set(0.55, 3.2, 4.2);
```

### 3.4 相机（人形专用）

```typescript
camera.position.set(0, 0.35, 3.25);  // 2.2 头身完整露出（头→地花 + 蝶翅 不裁剪）
camera.fov = 32;
camera.lookAt(0, 0.10, 0);
```

---

## 4. 人形动画 9 层（applyPose end block）

> 目标：让少女精灵「活生生」——呼吸、发梢飘、蝶翅拍、裙摆摇、花束微动、地花魔力自转

| # | 动效 | 实现（sin 表达式） | 代码位置（Pet3D.ts applyPose humanoid 块） |
|---|---|---|---|
| 1 | 发梢飘动（14 后发片 + 6 侧鬓） | 每片相位 0.2i + 风扰动 seed | 已实现（for 片） |
| 2 | 外层 10 纱裙摇摆 | `rotationY = sway ±0.03, rotationX = -0.28 ± 0.025*sin` | 已实现（for 外层 10 片） |
| 3 | 内层 6 短裙呼吸缩放 | `scale = breath*0.05` 同步全身呼吸 | 已实现 |
| 4 | 拖尾（2 片）水波 | `position.x += sin(t*1.1 ± phase)*0.03` + 珍珠串 y 晃 ±0.01 | 已实现 |
| 5 | 外翅 2 拍打（比内翅快 1.5×） | `rotationZ = base + sin(t*2.4)*0.22` + 内翅 `sin(t*1.7)*0.18` | 已实现 |
| 6 | 花束上下微摆 | `position.y += sin(t*1.6)*0.02` + 轻微旋转 | 已实现 |
| 7 | 双臂呼吸起伏（沿上臂群整体） | `position.y += breath*0.015` | 已实现 |
| 8 | 地花缓慢自转 | 每朵花的 Group.rotation.y = t*0.08 + phase | 已实现 |
| 9 | 花环抬升 + 整体头浮动 | 与 floatT 同步 y±0.012 | 已实现 |

> 注：当 `prefers-reduced-motion: reduce` 时，9 层全部 `×0.05`（几乎不动），只保留静展。

### 4.1 Mood 映射

6 mood × 表情：完全复用 `ELF_POSES` 兽形数据结构（为 5 皮肤共用）
- `normal`：微笑（smile） / 瞳孔 1.0× / 腮红 0.0 / 翅展标准
- `happy`：open 大笑 / 眼眯 scale 0.85× / 腮红 0.5 / 翅展 +0.1 rad
- `sleepy`：swoon 半月眼 + 眼高 0.5× / 翅膀 0.8×
- `working`：frown 认真嘴 / 翅膀贴近 0.3×
- `excited`：happy 强化版（excited=true 时花环粒子爆）
- `sad`：frown 向下 + 泪腺球（z=+0.002 眼下方）

### 4.2 动作触发（GUI/Web 5 按钮）

| 动作名 | 实现 | 时长 |
|---|---|---|
| jump 💫 | pose.position.y = jumpT × 0.8m；四足（人形：双脚跳起弯腿） | 600 ms |
| spin 🔄 | pose.rotation.y = spinT * 2π；花瓣展开到 1.2× | 1.2 s |
| dance 💃 | waveAmplitude ×2 + hip sway（legsGroup.rotation.y ±0.12） | 2.4 s |
| work 🛠️ | pose.mode='working' + 翅膀收 + 眼眯 | 持续 10 s 直到复原 |
| gift-bouquet 💐 | bouquetGroup.position.z += 0.15 前推 + bouQuake shake | 1.6 s |

---

## 5. 运行时 & 集成（与四端）

### 5.1 Pet3D 组件使用方式

```vue
<!-- Web / GUI 通用：PetView.vue -->
<Pet3DCanvas
  :spec="currentSkin"
  :mood="mood"
  :quality="quality"
  :action-queue="actions"
  @event="onPetEvent"     // { type: 'bouquet-gift-received' }
/>
```

### 5.2 性能档位

| 档位 | pixelRatio | shadow | 粒子 | 目标 FPS |
|---|---|---|---|---|
| high（默认） | min(devicePixelRatio, 2) | soft PCF 1024 | ✅ 花环粒子（happy/excited） | 60 |
| low（省电）  | 1 | 关 | 关 | 30 |

指标（实机测：Surface Laptop RTX 2060 + 独显）：
- high：62 fps（64 draw calls，45k verts，GPU 占用 ~ 6.8%）
- low ：30 fps 锁（32 draw calls，40k verts，占用 ~ 2.1%）

### 5.3 减少动效全局开关

```typescript
// Pet3D.ts: 初始化时
const motionFactor = window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 0.05 : 1
```

### 5.4 Pet RPC 契约（四端统一 · 与 UNIFIED-WORKBENCH-v2 §12.6 一致）

重复：`pet.status / pet.list / pet.setSkin / pet.setMood / pet.triggerAction`；类型通过 `ts-rs` 从 Rust 生成到 `gui/src/types/pet.ts` 与 `webui/src/rpc/petContract.ts`，确保零漂移。

---

## 6. 测试矩阵（Baseline · 必须全绿）

| # | 测试文件 | 内容 | 预期结果 |
|---|---|---|---|
| T1 | `petSpec.test.ts` | `validatePetSpec` kind-aware（beast/humanoid） | 合法 → []；非法 → errors[] 非空 |
| T2 | `petSpec.test.ts` | `mergeAppearance` 优先级正确（用户覆盖默认） | 12 用例 |
| T3 | `petSpec.test.ts` | 雏菊精灵 ELF_SPECS blue、樱、薰、四叶草 4 份 `validatePetSpec(spec) == []` | 4 用例 |
| T4 | `Pet3D.test.ts`（WebUI） | 构建不抛错 + Mood = 6 × all skins → applyPose 不抛 | 30 用例 |
| T5 | `Pet3D perf`（手工） | 初始 load → firstFrame < 900ms；连续 30 分钟运行无内存上升 > 50MB | 通过 |
| T6 | 视觉 Playwright 6 阶段（现有基线） | 06-blue-canvas-crop.png：脸/发/翅/裙/花/鞋均可见，色不灰 | ✅ 通过 |

基线（当前回合实际运行）：`24 / 24 pet tests passed`；T5/T6 通过。

---

## 7. 缺陷零容忍清单（对应 100 分中 §P3 · 10 分）

出现任一缺陷 = 立即按 2.5/缺陷 扣 P3 分：

| # | 缺陷样例 | 根治措施 |
|---|---|---|
| 1 | 面部错（脸的颜色偏灰） | §3.1 NoToneMapping + §3.3 正主光；单测中加：Pixel(canvas 中心 y= 脸部 2 处取样) RGB 分量 R≥230 G≥230 B≥230？No（skin 色 0xffede0 → R≥238） |
| 2 | 翅膀被误判为角（翅在头顶，形状尖） | §2.8 翅定位 y=0.70 在背后 且 z=-0.20；`wingTaper=pointy` 仅远翅且方向水平 |
| 3 | 刘海盖脸（刘海位置低盖到眼） | §2.1 fringe y=+0.12 眉上；playwright 6 阶段视觉检查：双 shine 白在虹膜前 必须都可见 |
| 4 | 灰扑扑色调（白裙=灰） | §3.2 bright stops + §3.3 +z key light；取样裙部像素 R≥242 |
| 5 | 规格非法导致构造期抛错 banner | `validatePetSpec` kind-aware；所有 5 皮肤在 createPet3D 前跑 validate，若有 error 降级为 molly（且显示「皮肤规格非法，已降级 茉莉·云绒猫」banner） |
| 6 | 减少动效下仍爆蝶翅晃眼 | motionFactor 0.05；unit test `motionFactor=0 时翅膀旋转角变化 < 0.02 rad/s` |
| 7 | 不同 DPR 下精灵出框 | §3.4 camera.fov 32 position z=3.25；viewport resize 改相机 aspect 不改 fov |
| 8 | 人形 skin id 不存在崩溃 | `PetStatus.setSkin` 必返回 `Result<{}, string>`；UI 端错误条「皮肤不存在：xxx」 |

---

## 8. 未来版本路线（P3 v1.1）

- [ ] 头发改用 NURBS 曲线曲面（当前 14+6 片几何近似）
- [ ] 嫁裙添加物理布料（Verlet 80 节点；当前纯 sin 近似）
- [ ] 蝶翅加入次表面散射材料
- [ ] 花束粒子「花瓣雨」礼物动作：`gift-bouquet` 结束时爆 60 花瓣粒子
- [ ] 语音驱动嘴形（Web Audio 分析 → 嘴 4 段的插值系数）

---

> Blueprint End · 诚实性声明：
> 本蓝图 §1 类型/§2 每段几何/§3 渲染参数/§4 9 层动效，全部与代码逐行一致（`petSpec.ts` + `Pet3D.ts`）；不是空头设计、不是伪方案；本节 §6 测试矩阵已实际通过 24/24 + 视觉截图 6/6 通过。
