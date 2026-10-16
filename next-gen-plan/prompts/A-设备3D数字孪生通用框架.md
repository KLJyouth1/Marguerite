# 任务书 A —— 设备 3D 数字孪生通用框架（Marguerite 版）

> 来源改造自「冰箱数字实验室复刻提示词」。原提示词要求从零做独立 React 广告级冰箱页；本任务书把它改造成 **Marguerite webui（Vue3 + three）内的通用设备数字孪生框架**，冰箱只是第一个参考实现。
> 落点：`webui/src/components/device-twin/`（新目录）。供研发小组 / 其他 AI 直接执行。

## 0. 角色与边界
你同时是：three.js 3D 工程师、程序化建模工程师、交互状态机工程师、教学可视化设计师、性能/QA 工程师。
**不要**另起 React 工程、不要做独立 HTML、不要下载外部 GLB/贴图。全部几何与材质程序化生成，复用 Marguerite 现有 three「岛屿工作空间」的渲染循环、质量分档与 2D 降级设施。

## 1. 通用框架（不是单个冰箱）
建一套可复用到任意 HA 设备（冰箱/空调/洗衣机/热水器…）的框架，目录：
```
webui/src/components/device-twin/
├── DeviceTwinStage.vue        # 舞台容器（画布 + 状态机切换 + 右侧面板槽位）
├── model/
│   ├── PartNode.ts             # 装配节点：独立 id / Group / metadata
│   ├── ViewStateMachine.ts     # product|open|explode|section|cycle|inspect
│   ├── KinematicsController.ts # 动作联锁控制器（统一状态，禁止按钮直改 Mesh）
│   └── MetadataCatalog.ts      # 部件档案驱动（从 metadata 自动生成说明卡）
├── sim/
│   ├── LumpedThermalModel.ts   # lumped 教学仿真（温度/功率/负荷）
│   └── FlowParticles.ts        # 通用「设备工作流粒子系统」
└── devices/
    └── fridge/FridgeModel.ts   # 冰箱 = 第一个参考实现（35+ 装配节点）
```

## 2. 必备能力（从原提示词提炼，通用化）
- **装配节点树**：每个部件独立 id/Group，能单独进入特写或参与爆炸；不要整壳一个 Mesh。
- **视图状态机**：整机 / 开合 / 爆炸 / 剖面 / 工作循环 / 部件特写 六态，统一状态驱动（`1 - exp(-dt*speed)` 缓动，禁止瞬移）。
- **动作联锁控制器**：抽屉拉出前必须先开门到安全角；关门前先收抽屉；进爆炸前自动收抽屉关门；爆炸中禁止不合理操作。
- **metadata 档案驱动**：`{id,name,english,category,intro,facts:[[k,v]]}`，UI 从 metadata 自动生成部件说明，不每个部件手写 HTML。
- **lumped 教学仿真**：温度/功率/压缩机负荷/热量平衡（Q排≈Q吸+P压），**界面必须显著标注「教学模拟 · 非实测」**。

## 3. 三套流体 → 通用「工作流粒子系统」
原冰箱的箱内空气循环/密闭制冷剂/向环境排热，泛化为设备工作流粒子系统，语义色：
- 冷流 = 青蓝（`var(--accent)` 系冷调）；
- 热流 = 暖橙；
- 工质 = 铜色。
三者视觉与解释不得混淆；用 CatmullRomCurve3 + TubeGeometry / 细粒子，禁止粗蓝发光管。

## 4. 数据契约（关键）
- 消费现有 `devices.status` / `devices.list` RPC。
- `configured:false` 时进入**内置演示模式**，用冰箱 mock 数据驱动，并在界面标注「演示数据 · 未连接真实设备」。
- 字段对齐 WP6 的 `useDevicesPanel`（id/name/hazard_level/transport/online）。

## 5. 设计系统铁律（之前踩过红）
- 色值/圆角/时长一律走 design token：`var(--accent)` / `var(--radius-*)` / `var(--dur-*)`，**禁止裸 hex、裸 px 时长、裸 border-radius**（architecture guard 会判红）。
- i18n 走 vue-i18n 现有 6 语言键位；文案不硬编码。
- 无 emoji；PBR 材质（MeshStandard/Physical），但色板受 design token 约束。

## 6. 性能与降级
- DPR clamp ≤2；重复件用 InstancedMesh；阴影只给关键灯；3D 不可用时降级到 2D 卡片（复用 WP6 的 DeviceIslandNode）。
- 桌面 60FPS 目标；远景降质，不堆百万三角。

## 7. 验收（每 YES/NO 自检）
- [ ] 35+ 独立装配节点、可单独特写/爆炸
- [ ] 六态状态机 + 动作联锁（先开门再抽抽屉、爆炸前自动复位）
- [ ] 三套粒子语义色区分、教学仿真实时联动面板
- [ ] configured:false → 演示模式（标注）；configured:true → 消费真实 RPC
- [ ] 复用同一接口可加空调/洗衣机（不复制冰箱特判）
- [ ] vitest 覆盖状态机迁移、联锁规则、仿真数值收敛（无 NaN/Infinity）
- [ ] 截图 QA：整机/开门/爆炸/剖面/循环/特写 六态
