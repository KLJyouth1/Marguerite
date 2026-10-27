# 三个抖音视频拆解 × Marguerite 智能体/技能/团队/工作台改进路线

> 来源：用户 2026-10-21 给出的三条抖音视频。
> **模型无关原则**：视频里出现的模型名（Fable / Opus / Tripo / Astra / DeepSeek 等）一律不作为依赖，
> 本文只提炼**工作流结构、角色分工、方法范式**；Marguerite 侧统一走模型路由（本地 omnipilot + 云端），
> 用户实际生产以 deepseek-v4.1 为主力。
> 整理日期：2026-10-21。

---

## 一、视频内容提取

### 视频 1｜极简AI（3%）：DeepSeek Harness 自动构建四轴机器

- 链接：https://www.douyin.com/video/7674842569754299698
- 同作者/同系列参照（站内检索完整内容）：
  - 四足机器人 CAD：https://www.iesdouyin.com/share/video/7673398049053494582
  - Machine Skill（物理仿真）：https://www.iesdouyin.com/share/video/7696175669423639823
  - Anatomy Skill（解剖式科普）：https://www.iesdouyin.com/share/video/7696818658722831985
  - 自建工作台（场景窗口）：https://www.iesdouyin.com/share/video/7691249766273010959
  - 桃花源记代码世界（成本样本）：https://www.iesdouyin.com/share/video/7698220829738227313
  - 漫剧四 Skill 流水线：https://www.iesdouyin.com/share/video/7687604513196231988
  - Skill 搓法教程：https://www.iesdouyin.com/share/video/7651618623198334251

核心内容：
1. **Harness + Skill 自动工程**：按 Skill 模板输入机器类型/尺寸/结构/关节数/输出格式 → 模型先理解机械结构并拆解建模步骤 → 执行器调用 text-to-CAD skill → 依次生成主机身、腿、髋关节、膝、足端，**形成完整装配层级**（关节间有明确连接关系，可继续做站立/迈步运动演示），输出可编辑的 STEP/STL，而不是一张"像机器人的外观图"。
2. **Machine Skill（仿真 > 解释）**：先研究真实结构和尺寸，把零件、材质、**物理计算**一起做进网页；涡扇/火箭发动机的推力与状态数据由内部物理模型**实时计算**（不是写死的数字），支持外壳剖切、逐层拆解、换角度观察；最终是单个 HTML，浏览器打开就跑，还能直接录成展示视频。
3. **Anatomy Skill（物体本身就是解释）**：把抽象原理做成可运转、可交互的机械模型动画；再叠加讲解配音成为科普视频；对应 Karpathy 的四层理解递进——简明语言 → 示意图 → 可交互网页 → 定制讲解视频。
4. **Skill 四件套**：SKILL.md（说明书/触发条件）+ prompts（指令模板）+ scripts（可执行脚本）+ examples（参照样例）；**渐进式披露**——SKILL.md 只放概述与触发，具体模板按需读取，避免上下文爆仓、token 浪费。
5. **Skill 流水线**：漫剧案例用 4 个 skill（剧本 → 分镜 → 生图 → 视频）串成一条管道，角色一致性锁在规则里。
6. **成本过程透明**：桃花源记案例"思考 34 分钟没写代码 → 拆 7 项任务 → 289 步、19 个文件、压成 1 个 HTML、1.04 亿 token、292 次请求、3.93 元"。

### 视频 2｜陆三金：《小王子》3D 游戏作者公开制作流程

- 链接：https://www.iesdouyin.com/share/video/7696910394345917747
- 深度拆解：https://www.iesdouyin.com/share/video/7697945243907263779
- 商业视角复盘：https://www.iesdouyin.com/share/video/7698223165834595584

完整流水线（1 人 16 天）：
1. **写需求**：PRD 文档梳理人物、场景、剧情。
2. **定参考图**：先出一张满意的场景图，再补俯视/测试角度；人物出 T-pose 四视图。
3. **拆资产**：让 AI 看参考图列出需要单独建模的资产（岩石、玫瑰、椅子……），标注出来**由人确认**，其余在 Blender 直接搭。
4. **生成资产**：角色用多视图 + 四边面拓扑（方便后续绑骨），道具用高精度模型，网格不对局部重生成；8K 贴图。
5. **照图复刻**：拿着参考图 + 3D 资产，让执行 agent 操控 Blender/Unity 组装，差异处迭代累加。
6. **独立审查（关键）**：再派一个**独立的审查 agent**，多机位盲审——与参考图不一样就打回重做。
7. **绑骨与动作**：Blender + Auto-Rig Pro 自动绑骨蒙皮（由 agent 用提示词驱动）；Mixamo 动作库取动画，没有现成动作的先摆近似造型，再用 IK 逐帧对到目标位置。
8. **修 bug**：交给**另一个模型**做后期迭代。

作者三点启发：
1. 从统一的参考图开始，整体风格最可控；
2. 复杂场景启用**对抗式审查**，让另一个 agent 专门挑刺；
3. 从建模、绑骨到写代码基本都是 agent 在干活。

评论区与相关视频的补充（同样重要）：
- **别让 AI 自己批卷子**：主力模型写，另一个（更便宜的）模型当检查员；
- **平替链**：生图→即梦/可灵；图生 3D→Tripo 或腾讯混元 3D（开源）/Blender；引擎→Unity 或 Godot；
- **卡点不在 AI**：品味、审美、发行与注意力才是稀缺项；
- 越大的项目，"一直记得自己在干什么"的长上下文能力越关键。

### 视频 3｜橘皮：DeepSeek 1017 代码渲染视频直逼 Opus 5

- 链接：https://www.douyin.com/video/7698292106071684521
- 同主题最完整讲述：https://www.iesdouyin.com/share/video/7698639727523532041
- 音乐卡点做法：https://www.iesdouyin.com/share/video/7691596550673419520
- 19 种风格/1830 帧：https://www.iesdouyin.com/share/video/7697591209846030464
- 方法全教程：https://www.iesdouyin.com/share/video/7697865403163523811

核心内容：
1. **代码渲染，不用视频生成模型**：Remotion、HyperFrames，或 HTML/Canvas/Three.js/WebGL、Pillow 逐帧 + numpy 合成音乐；30 秒 900 帧，每一拍可复现。
2. **审美/执行分工**：贵的模型只负责定方向、拆参考、写分镜；写代码、渲染、改 bug、返工这些最费 token 的重活交给便宜的强代码模型（实测 14.69 元/30 分钟，对订阅制约 90 倍价差）。
3. **模糊词接不住**："高级一点、丝滑一点"每个模型理解都不同；必须用动效师的精确语言——出场 ease-out（前快后慢）、大标题与小字拉开字重、节奏卡 129 BPM。
4. **参考视频反推**：把喜欢的视频丢给模型，让它拆出缓动、转场、配色、BPM，再照拆出的分镜出样片；能打动你的东西也会打动同口味的人。
5. **分镜先行**：先列每个镜头几秒、画面、文案，确认后再渲染——改分镜比改成片便宜得多。
6. **抽帧自审**：渲染中每秒截几十帧做检查，以"挑剔的真人导演"视角给自己挑毛病，自我迭代。
7. **音乐卡点**：算法检测节拍，切镜吸附拍点，画面跟着鼓点走。
8. **Skill 诅咒**：好 skill 用的人越多，观众越快审美疲劳，内容迅速平庸；作者刻意不用现成 skill，而用"私有参考反推"。
9. 提示词要素：平台/尺寸/时长、真实素材（品牌色/Logo/录屏）、分镜确认、全局风格与限制（如避免蓝紫 AI 味）、抽帧自审。

---

## 二、跨视频共性：九条可迁移范式

1. **流水线 > 单点**：一个技能/一个 agent 干一件事，产出结构化结果交给下一环（剧本→分镜→图→视频；需求→拆解→CAD→装配）。
2. **三角色分工 > 全能包办**：定方向的"审美者"、干重活的"执行者"、独立挑刺的"审查者"分离，审查者甚至换模型。
3. **参考锚点 > 空想 prompt**：统一参考图、参考视频反推、照图复刻、多机位 diff——先把"什么是对的"变成可比对的锚。
4. **不能自己批卷子**：执行者与审查者必须上下文隔离，必要时异源。
5. **精确语言 > 模糊形容词**：机械参数、动效曲线、BPM、字重——把感受词翻译成可执行 DSL。
6. **物体即解释**：能交互的仿真/可运行产物 > 静态文字说明。
7. **便宜阶段设门**：资产清单、分镜表先人工确认，再进入昂贵的渲染/生产。
8. **全程透明可复现**：步数、文件数、token、请求数、成本实时可见、过程可复现。
9. **个人配方资产化**：公共 skill 导致同质化，把自己的审美与成功流程蒸馏成私有资产。

---

## 三、对照 Marguerite 现状的六个维度改进机会

> 每条格式：**现状 → 差距 → 建议（开发项）**。只写实地核实过的现状。

### A. 智能体（agents）

**现状**：`plan/`（PlanGate）、`governance/completion_gate.rs`、`governance/review.rs`（AutoReview 已能对 diff 出结构化裁决）、`vision/`（diff / modlens / grounding / stitch / evidence）、`harness/`（run_isolated、独立预算、并发闸门、meta 自改进）、`agent/turn_budget.rs`、`agent/task_checkpoint.rs`。

- **A1. 红队盲审 agent（P0）**：AutoReview 只审代码 diff，缺"产物对参考"的视觉审查。新增独立 reviewer 角色：拿参考锚点（图/分镜/规格）对产出做多机位盲审，复用 `vision/modlens/stitch/diff` 做证据比对，**强制经 `harness::run_isolated` 派生（独立上下文、独立预算），可指定异源模型**；不通过则回执行队列并附 diff 证据。
- **A2. 蓝图门 Blueprint Gate（P0）**：PlanGate 增强为"分镜表/资产清单/界面草图先确认再执行"，蓝图结构化落盘并成为验收契约；"先出分镜"在工程类任务里等价于"先出任务分解清单 + 验收口径"。
- **A3. 按角色路由（P0）**：director（定方案/拆参考）→强模型；worker（写码/渲染/长循环）→便宜或本地模型（omnipilot）；reviewer→异源模型。基础设施 model router + `pin_active_model` + `bloom_failover` 已具备，补"角色→策略"映射并在 UI 可见。
- **A4. 抽帧自审循环（P1）**：渲染/页面/视频类任务自动跑"生成→截图→视觉打分→迭代"闭环（browser CDP 已在 `browser/cdp.rs`），评分与每轮证据落盘，达阈值或达预算轮次停止。
- **A5. 参考反推 agent（P1）**：输入链接/视频/图片 → 输出结构化设计语言（配色、缓动、转场、构图、BPM、信息层级），形成 `reference manifest` 供后续任务锁风格。

### B. 技能（skills）

**现状**：内置 YAML 技能（`skills/*.yaml`，prompt 模板）、`core/src/skills/{engine,loader,registry}.rs`（含 GitHub 安装/签名）、`tools/skill_tools.rs`、`security/skill_scan.rs`、以及 .skills 目录下 SKILL.md + references 的重型技能（如 blender-skills）。

- **B1. 技能流水线编排（P1）**：声明式 pipeline YAML（每个 skill 声明 inputs/outputs schema 与质量门，前环产物自动成后环输入），漫剧四件套/游戏流水线可直接表达；在 /control/skills 增加管道视图。
- **B2. 技能四件套规范对齐（P1）**：把 YAML 轻技能与 SKILL.md 重型技能统一到同一规范（概述+触发、渐进式披露、prompts/scripts/examples），并提供脚手架（对齐 skill-creator 产出）。
- **B3. 物理仿真技能（P2）**：Machine/Anatomy 模式——"原理/设备 → 可交互 HTML 仿真（物理模型实时算）+ 可生成讲解视频"，与 html skill、browser 录屏、配音能力组合。
- **B4. 参考反推技能（P1）**：视频/图片→设计语言拆解（blender-skills 已有 reference-analysis 模板，泛化到 UI/动效/视频）。
- **B5. 私有配方蒸馏（P2）**：在 `harness/distill.rs` 基础上增强"个人风格锁"——成功案例（参考锚点 + 决策 + 审美偏好）→ 私有 skill，规避公共 skill 同质化。
- **B6. CAD/工程技能族（P2）**：text-to-CAD、Blender headless（`tools/blender.rs` 已通）、STEP/STL 导出；**显式建模"装配层级/连接关系"语义**（机械系统而非外观）。

### C. 工具（tools）

**现状**：`tools/builtin/`（fs/browser/editor/query/web…）、`browser/cdp.rs`、`tools/computer_use.rs`、`tools/blender.rs`、`tools/media.rs`、桌面自动化（UIA/COM）、webui 音乐引擎已有频谱/节拍算法。

- **C1. 代码渲染视频管线（P1）**：`render_html_to_video(url|html, fps, audio?, bpm?)`：headless 浏览器逐帧截帧 + ffmpeg 合成 + 音轨；Remotion 类能力本地化，全程回环、素材同源。
- **C2. 节拍检测工具（P1）**：音频→BPM/拍点时间轴（numpy 能量/自相关），切镜与动画时间线吸附拍点；webui `useAudioReactivity` 的 beat 检测下沉为可命令行复用的工具。
- **C3. 资产清单分解器（P1）**：参考图/需求 → 结构化资产清单（名称/类型/来源/动作/负责人），带"待确认"状态，供蓝图门使用。
- **C4. 能力注册表 + 平替链（P2）**：同一能力挂多个工具（3D：Blender/混元 3D；引擎：Unity/Godot；视频：ffmpeg/Remotion），主工具失败自动降级；`harness/registry.rs` 已有 Maturity 字段，补 fallback 顺序与切换证据。
- **C5. 本地 3D 资产生成（P2）**：接入开源混元 3D 类本地部署，符合本地优先/回环不变量；Mixamo 类动作库做离线镜像。

### D. 智能体团队（agent teams）

**现状**：`agent/team.rs`、`agent/subagent.rs`、`harness/subagent.rs`（ConcurrencyGate/IsolatedSubagents/SidechainTranscript）、workbench `expert_teams.py` 已声明 8 个专家团（PPT/全栈/战略/数据/法务/投研/游戏/产品）。

- **D1. 流水线团队模板（P1）**：把"PRD 师→资产师→工程师→审查员→修复师"做成可复用团队模板（角色、输入契约、产出、路由模型），落到 /control/agents 与 expert-center。
- **D2. 审查员独立性强制（P0）**：team 定义里 critic 角色强制 isolated 上下文 + 异源模型，作为 CompletionGate 前置；"自己批卷子"在配置层即拒绝。
- **D3. 长任务过程看板（P0）**：任务树/步骤/请求数/成本/产物实时可见（见 F2）。
- **D4. 结构化交接（P1）**：角色间以 manifest（参考图、资产清单、模型文件、分镜）交接，禁止只靠对话上下文传递。

### E. 专家（experts）

**现状**：workbench 8 个专家团（declarative 角色+checklist）、/control/expert-center、外部 WorkBuddy 插件（senior-dev-engineer）。

- **E1. 新增四个专家包（P1/P2）**：
  - 动效导演：精确动效语言库（easing/字重/BPM/转场）+ 分镜模板 + 抽帧审查清单；
  - 游戏制作总监：PRD→参考图→资产拆解→绑骨动作→引擎全流程（含平替链）；
  - 机械/仿真工程师：CAD 装配语义 + 物理建模 + 可交互仿真；
  - 视频管线架构师：代码渲染管线、卡点、音频、自审循环。
- **E2. 方法论回灌（P1）**：把"分镜先行、对抗审查、参考锚点、审美/执行分离、自己不批卷子"蒸馏进专家文档，senior-dev-engineer 增补"多角色分工与审查员独立"铁律。
- **E3. 专家内置平替表（P2）**：每个领域专家自带"首选/开源/本地/免费"工具链（Tripo→混元 3D、Unity→Godot 等）。

### F. 工作台（workbench）

**现状**：webui 控制台（fleet/editor/terminal/browser-cockpit/wallpaper/music 等）、`tui/workbench/dsh_slots.rs`（TUI 已有槽位概念）、workbench React（:8011，agentloop/llmlab/ttc/harness）、`usage/attribution.rs`、/control/usage、/control/task、/control/evolve。

- **F1. 场景化布局（P1）**：DSH 作者的"场景跟着内容走"（Blender 场景 3 窗、视频场景 4 窗，无固定模板）→ webui 支持按任务类型保存/自动编排面板（建模/视频/编程/写作场景），技术上与音乐模式布局变动、TUI dsh_slots 同源。
- **F2. 长任务实时看板（P0）**：task/evolve 页展示实时任务树、步数、token/请求数/累计成本（桃花源案例式数字）、产物时间线；usage 归因数据已有，补实时大数字与预算刻度。
- **F3. 蓝图门 UI（P0）**：执行前展示分镜表/资产清单/验收口径，一键确认/调整。
- **F4. 产物对比视图（P1）**：参考锚点 vs 产出并排多机位 diff，把 vision 证据上屏。
- **F5. 视频制作场景（P1）**：C1 渲染管线 + C2 节拍 + 音频引擎 + 镜头脚本编排成一个视频场景布局。
- **F6. 本地资产库（P2）**：本地 3D 模型/动作库/素材检索，本地优先。

---

## 四、落地优先级

### P0（与现有工作直接咬合，改动小、收益大）
1. **D2/A1 审查员独立 + 红队盲审**：CompletionGate 前置，复用 isolated subagent + vision diff，配套"自己不能批卷子"的配置拒绝规则。
2. **A2/F3 蓝图门**：PlanGate 结构化 + 前端确认 UI（工程任务先出分解清单/验收口径，创意任务先出分镜）。
3. **A3 角色路由**：director/worker/reviewer 三档策略接到现有 model router，UI 可见、可切换。
4. **F2/D3 长任务实时看板**：步数/token/请求/成本实时数字与预算刻度。

### P1（一到两个工作流版本）
5. B1 技能流水线编排（含产物 schema 与质量门）
6. C1/C2 代码渲染视频管线 + 节拍检测（复用 CDP、ffmpeg、已有音频引擎）
7. F1 场景化布局
8. C3/D4 资产清单分解器 + 结构化交接
9. B4/A5 参考反推技能与 agent
10. E1（动效导演、游戏总监两个专家包先行）
11. A4 抽帧自审循环、F4 产物对比视图

### P2（探索性/重资产）
12. B3 物理仿真技能（Anatomy/Machine 模式）
13. B6/C5 CAD/3D 工具链与本地混元 3D
14. B5 私有配方蒸馏
15. C4/E3 工具平替链
16. F5/F6 视频场景与本地资产库

---

## 五、与 Marguerite 既有架构的咬合点（实现时复用，不另起体系）

- 隔离/预算/并发：`harness::run_isolated`、`SubagentBudget`、`ConcurrencyGate`、`turn_budget.rs`
- 检查点续跑：`harness/checkpoint.rs`、`agent/task_checkpoint.rs`、`gateway/checkpoint_rpc.rs`（P2-11 已落一期）
- 审查与门：`governance/review.rs`（diff 评审）、`completion_gate.rs`、`plan/`
- 视觉证据：`vision/{modlens,stitch,diff,evidence,grounding}.rs`、`browser/cdp.rs`
- 模型路由：`model/router.rs`、`gateway/router.rs::pin_active_model`、`model/bloom_failover.rs`、`model/ensemble.rs`
- 技能：`core/src/skills/`、`harness/registry.rs`、`security/skill_scan.rs`
- 团队：`agent/team.rs`、`harness/subagent.rs`、workbench `expert_teams.py`
- 成本：`usage/attribution.rs`、`workbench/usage.rs`、harness `dsh_bridge` 里的 UsageAggregator
- 布局：`tui/workbench/dsh_slots.rs`、webui 音乐模式布局变动机制
- 音频/视频：webui `audioEngine`/`useAudioReactivity`、`tools/media.rs`、ffmpeg 工具链

---

## 六、边界说明

- 视频 1 分享标题被截断（"四轴机器"），其精确画面未能直接抓取（抖音页面 JS 渲染、静态页无描述）；本文以同作者/同主题系列完整内容（四足机器人 CAD、Machine Skill、Anatomy、DSH 工作台）还原其方法论，未虚构具体画面。
- 视频 3 原视频为 DeepSeek 1017 更新实测，站内同主题最完整讲述（7698639727523532041）与分享文案一致，核心数据（14.69 元/约 30 分钟/90 倍价差/129 BPM）取自该讲述。
- 所有模型名仅作案例引用，不构成开发依赖；Marguerite 实现统一走模型路由。
