# 任务书 C —— 岛屿 Data-Driven 程序化建筑生长（Marguerite 版）

> 来源改造自「建筑生成提示词（Living City Diorama）」。原提示词要求独立 WebGL 施工沙盘工程；本任务书把它**扩展进 Marguerite 现有 `webui/src/components/island/`**（islandSpec/islandWorld/islandBuilders），强化数据驱动与「建筑真实生长」底座。
> 落点：扩展现有 island 目录。**禁止另起工程、禁止 scaleY 拉伸假装长高**。

## 0. 角色与边界
你是 three.js 程序化建模 + 数据驱动架构工程师。目标是让岛屿上的建筑按工序**真实生长**，并让「设备接入 / 任务进度」映射为地块施工阶段。复用现有 island 渲染循环与风格。

## 1. 核心原则（从原提示词提炼）
- 模型与逻辑分离；场景与 UI 分离；动画与模型分离；**时间系统统一驱动世界变化**；对象 Entity 化；**场景配置 Data Driven**；动画由状态机驱动；不把业务硬编码进模型。
- 不要一个 scene.js 几千行；拆 core/systems/entities/render/ui。

## 2. Data-Driven 场景
- 引入 `scene.json` 风格配置（建筑/车辆/工人/塔吊/道具/路线/光照/施工任务清单）。
- **加建筑不改核心代码**：新场景 = 加一份 JSON。在现有 island builder 体系里加一个「施工地块」实体类型，由配置驱动。

## 3. 建筑按工序真实生长（最关键）
Building **不是**一个完整 GLB / 不是 scaleY 拉伸。拆成工序阶段，随 `constructionProgress 0..1` 真实出现：
```
Stage 01 柱子 → 02 梁 → 03 楼板 → 04 脚手架 → 05 工人进入
→ 06 外墙 → 07 窗户 → 08 脚手架拆除 → 完工
```
BuildingStage 枚举：`FOUNDATION / STRUCTURE / FRAME / FACADE / WINDOWS / INTERIOR / FINISHED`。
**State From Time**：拖动时间轴从 Day10 回 Day3，建筑要回到 Day3 形态——不是只能向前播的动画。
不同阶段改模型/材质/道具/工人/车辆密度（地基阶段挖掘机多、结构阶段塔吊活跃、立面阶段脚手架多、完工拆围挡加绿化）。

## 4. 把设备接入/任务进度映射为施工阶段
- 设备接入成功 = 该地块「开工」（地基→首层）；
- 设备任务完成 = 「封顶」；
- 进行中 = 对应楼层施工（工人/塔吊活跃）。
这样岛屿上的「工作」不再是装饰，而是真实任务/设备状态的可视化投影。

## 5. 工作过程细节（按性能预算裁剪、实例化）
- 塔吊：塔/臂/吊钩/钢索/载荷，臂旋转 + 吊钩升降 + 小车前后，完成「材料区吊起→旋转→楼顶放下」循环；
- 工人：状态机 IDLE/WALK/WORK/CARRY，沿 waypoint graph 移动，不同 scheduleOffset，不机械同步；
- 工程车辆：沿 CatmullRom 路线，起步/巡航/刹停/停靠，车轮旋转、搅拌筒/挖掘臂/吊臂独立动画；
- 重复件（护栏/锥桶/路灯/树/脚手架管）用 InstancedMesh，不堆 draw call。

## 6. 时间/昼夜/天气（轻量版）
- 虚拟世界时间（1 real s = N game min，倍速 0.5/1/2/5/10x）；
- 昼夜平滑过渡（太阳位置/天光/雾/阴影/曝光连续变，禁止白天蓝晚上黑硬切）；
- 夜间灯用 emissive 为主 + 少量 PointLight；
- 雨/雪用 Points/InstancedMesh（不堆独立 Mesh），雨雪渐进（snowCoverage 0→1），雨天湿地面 roughness↓ reflection↑。

## 7. 点击与状态卡
- 点建筑：显示 Level/进度%/工人数/当前工序/预计完工；点工人/车辆：状态/任务/目的地。
- hover 弱 outline、点击强 outline，不改模型色（与任务书 B 的 `<EntityStateCard>` 共用）。

## 8. 设计系统铁律
- 色值/时长/圆角走 design token；**禁裸 hex/裸 px 时长/裸 border-radius**。
- i18n 6 语言；无 emoji；木桌暖光，不蓝紫冷调。
- 只在自己落点新增/扩展，共享接线（路由/islandContract）由协调者串行收口。

## 9. 性能
- Draw call < ~300、三角 < ~1.5M；远景 LOD/合批；关键物投影，远物关 castShadow；ACESFilmic tone mapping。

## 10. 验收
- [ ] 建筑真实生长（柱→梁→板→脚手架→外墙→窗），**无 scaleY 拉伸**
- [ ] 时间轴拖动可回放到任意施工阶段（State From Time）
- [ ] 设备接入/任务进度映射为地块施工阶段
- [ ] 塔吊/工人/工程车辆有真实工作循环，不匀速绕圈、不机械同步
- [ ] 昼夜/雨/雪渐进切换不硬切
- [ ] vitest 覆盖：进度→阶段映射、时间回放、施工任务事件触发
- [ ] 截图 QA：地基/中段施工/封顶/完工 + 夜景
