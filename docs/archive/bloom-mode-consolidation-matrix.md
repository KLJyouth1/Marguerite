# K2.2 重复 / 冲突 / 互补矩阵（UnivAgent 三仓整合分析）

> 研究型产出（无代码改动）。基于 K2.1 `bloom-mode-nexustar-ui-map.md`、战略计划 v2.0 §3.2/§3.3、PRD 扫描两个仓库：
> - **Active**：`D:\Downloads\GUI_Agent\`（nexustar-ui / nexustar-api / desktop / nexustar-website / nexustar-website-next / admin-backend / gwwc / nexusops / docs）
> - **Archived**：`D:\Downloads\_GUI_Agent_archived_20260713\`（external/ 竞品源码 + gwwc/models/skills 旧实现 + docs/）
> - **Marguerite**：`D:\Downloads\Marguerite\marguerite\`（Rust Ratatui TUI 引擎，独立）
>
> 标注「待核实」者为未能在本轮逐文件验证、需后续确认的结论。路径均来自 `ls`/`find`/`Grep` 实证。

---

## 〇、已整合基线（正面信号，作为后续合并范本）

扫描发现 **HTTP 客户端已完成收敛**，可作为其他合并的范本：

- `nexustar-ui/src/api/client.ts` 现为统一 HTTP 内核（SSOT），注释明确「整合原 client.ts / client-root.ts / http.ts / tauri-fetch.ts 职责」。
- `api/http.ts`、`api/client-root.ts` 已降级为**向后兼容重导出层**（仅 `export ... from './client'`），仍被 `admin.ts`/`longRunning.ts`/`tools.ts` 引用，但无独立逻辑。
- `api/auth.ts`（登录/登出/token 端点，snake_case→camelCase `toUser` 边界映射）与 `api/auth-headers.ts`（Bearer 注入 + 401 处理）**职责清晰互补**，非冲突。

> 启示：D1–D6 的合并应仿此模式——先建 SSOT，旧件保留为兼容壳，再随引用点重构逐步删除，避免一次性大改引发回归。

---

## 一、重复（Duplicate）—— 同一能力在 ≥2 处实现

> 重复指「同一用户可见能力由两个及以上代码位置承担」。其中**受控拆分（如门面模式）不计入真重复**，仅作低风险提示。本节按「遗留 vs 活跃」「双前端」「视觉引擎」「跨屏壳」四类归纳。重复的危害主要是：双版本漂移（改 A 漏 B）、构建体积膨胀、新人不知该改哪处。合并策略统一仿「〇、已整合基线」：先立 SSOT，旧件留兼容壳。

| # | 能力 | 位置A（路径） | 位置B（路径） | 建议（保留/合并/删除） | 风险 |
|---|------|--------------|--------------|------------------------|------|
| D1 | 官网前端 | `GUI_Agent/nexustar-website/`（v1：`dist/`、`src/`、`README-ARCHIVED.md`） | `GUI_Agent/nexustar-website-next/`（v2 Next.js：`app/`、`components/`、`lib/`） | **合并→保留 v2**；v1 已标 `README-ARCHIVED.md` 归档，删除其 `src/` 防双版本漂移 | 中：两套构建/路由各自演进，PRD P1-3 仅指向 v2，v1 不应再接需求 |
| D2 | 技能系统 | `GUI_Agent/gwwc/skills/builtin/hermes-skills/`（遗留技能包） | `nexustar-ui/src/components/SkillDrawer.tsx` + `store/marketplaceStore.ts` + `store/toolsStore.ts` + `api/skills.ts` | **合并→保留 nexustar-ui**；将 hermes-skills 迁入统一技能目录，gwwc/skills 整体弃用 | 中：技能加载路径不一会导致「桌面端能用、官网市场看不见」的隔离断裂 |
| D3 | 粒子 / WebGL 视觉引擎 | `nexustar-ui/src/components/workgame/VoxelWorld.tsx`（R3F 实验） | `nexustar-ui/src/hooks/useSplashParticles.ts` + `useAudioParticles.ts` + `components/music/Visualizer.tsx`（Canvas2D/独立） | **合并→统一到 Bloom 引擎**（K1.4 `BloomBackground`）；Splash 保留一次性入场，其余复用同一 rAF+three 管线 | 高：三套渲染范式（R3F / Canvas2D / 计划中的 three）并存，包体积与 FPS 纪律难统一（见 Bloom 标准 §性能） |
| D4 | 聊天主控组件 | `nexustar-ui/src/components/chat/ChatSection.tsx`（旧统一壳） | `nexustar-ui/src/screens/WorkbenchV2Screen.tsx` + `components/unified/*`（新 `CHAT_SECTION='unified'` 壳） | **合并→保留 unified 壳**；`AgentScreen/SwarmScreen` 仍 import 旧 `ChatSection`，重构后删除旧件 | 中：两壳并存致 section-convergence churn，新增卡片逻辑易漏接 |
| D5 | 编排 / DAG 画布 | `nexustar-ui/src/components/orchestration/OrchestrationCanvas.tsx` + `OrchestrationDashboard.tsx` | `nexustar-ui/src/components/AgentFlowCanvas.tsx` + `screens/swarm/SwarmDashboard.tsx` + `swarm/SwarmDAGView.tsx` + `components/gome/CockpitPanel.tsx` | **合并→收敛到单一 orchestration 画布**；swarm/gome 作为该画布的模式而非独立实现 | 高：≥4 处 DAG/编排视图，节点/边数据模型若各异，跨屏状态无法互通 |
| D6 | 管理后台 | `GUI_Agent/admin-backend/`（Java SpringBoot，新建） | `GUI_Agent/gwwc/web/_deprecated_admin_20260731/` + archived `gwwc/admin/` | **保留 admin-backend**；两类遗留 admin 直接删除（已 deprecated） | 低：遗留 admin 已明确 `_deprecated` 标记 |
| D7 | Chat Store 拆解 | `store/chatStore.tsx`（Facade 门面） | `chatSessionStore.ts` / `chatMessageStore.ts` / `chatStreamStore.ts`（三子库） | **保留（受控拆分，非真重复）**：`chatStore.tsx` 显式 re-export 三子库，属有意门面模式，勿回写本体 | 低：新增消息逻辑严格走子库即可 |
| D8 | 自研模型 / 后端能力 | archived `_GUI_Agent_archived_20260713/gwwc/`（agent_loop / adaptive_model_cascade / adaptive_recovery 等） | `GUI_Agent/nexustar-api/app/`（线上后端） | **按需移植** archived 的级联/降级逻辑供 P0-2 本地自研模型，不整体搬（归档路径会失效） | 中：直接引用 archived 路径随归档失效，须复制进 nexustar-api |
| D9 | 全局样式表碎片 | `nexustar-ui/src/` 根级 6 个 CSS：`index.css`、`design-system.css`、`modules.css`、`user.css`、`splash.css`、`highlight-theme.css` + 组件级 `screen/screen.css`、`gome/gome.css`、`workbench-v2.css` | 主题系统 `theme/theme.ts` + `theme/tokens.css`（四主题，全 CSS 变量） | **收敛→样式变量化**：新组件一律走 `tokens.css` 变量；散落根级 CSS 逐步并入设计系统，硬编码色清理（与 C3 同源） | 中：多根 CSS 易产生选择器优先级冲突与主题不跟随 |
| D10 | 双栈可达性 / 降级逻辑 | `nexustar-ui/src/store/backendStore.tsx`（前端感知） | `nexustar-api` 后端切换 + archived `gwwc/adaptive_model_cascade.py`（降级策略） | **保留 backendStore 作前端态**；降级算法以 archived 为参考实现移植进 nexustar-api，避免前端承担后端决策 | 低：职责本就前后端分离，仅需对齐切换事件契约 |
| D11 | MAX 双带队 UI 分散（PRD ①最高优先级） | `components/chat/MaxDualPanel.tsx` + `components/DualLeaderPanel.tsx` + `components/MaxModeBanner.tsx` | `components/workbuddy/MaxImmersive.tsx` + `workbuddy/ChatConsole.tsx` + `workbuddy/QuestWorkbench.tsx` + `store/dualLeaderStore.ts` | **合并→收敛到统一 MAX 子系统**（`chat/MaxDualPanel` 作卡片核心，`workbuddy/*` 作沉浸态，去重 `DualLeaderPanel`/`MaxModeBanner` 与 MaxDualPanel 的重叠逻辑） | 高：PRD ①「零冗余」直接依赖；UI 散落会导致双带队卡片与沉浸态行为不一致、难统一测试 |

---

## 二、冲突（Conflict）—— 同一意图的不同实现相互抵触

> 冲突指「同一架构意图存在互斥的实现选择，必须二选一否则系统行为不确定」。与重复不同，冲突若不选边会导致运行时歧义（如 Bloom 接 EventBus 还是 store）。本节含 1 项**直接阻塞 K1.4** 的规划/现状冲突（C2），须优先消解。

| # | 意图 | 实现A | 实现B | 冲突点 | 统一建议 |
|---|------|-------|-------|--------|----------|
| C1 | 前端状态管理 | `core/store.ts` `create()`（zustand v5 兼容，所有现代 store 使用） | `core/shu.ts`（`Shu`/`useShu` 路径订阅中枢）+ `core/instances.ts`（`appShu/appChe/appMai` 遗留 16 文件 Context+zustand 混合，标注「暂不迁移」）+ React Context（`ConfirmProvider`）+ `useReducer`（`MessageBubble.tsx:463`） | 5 套并存机制，特征状态该用哪套无约束；Bloom 误用 `instances.ts`/`Shu` 即落入遗留层 | **唯一规范**：特征态一律走 `core/store.ts` `create()``（与 `modeStore/userStore` 同款）。`Shu`/`instances` 仅限 UI/意图/资源编排，`Context`/`useReducer` 不得承载跨组件特征态 |
| C2 | 事件 / 信号总线 | K1.4 规划：`BloomBackground` 订阅 **EventBus**（`message:sent`→SOLID_APPEAR 等，战略计划 §2.4） | K2.1 实测：**仓库无全局 EventEmitter**；跨模块信号经 store（`getState/subscribe`）或 SSE（`api/sse.ts`） | **规划与现状冲突**：K1.4 假设的 EventBus 不存在，自建总线会与 store/SSE 双轨 | **对齐现状**：Bloom 经 `useModeStore` 订阅 + `api/sse.ts` 监听消息事件，**不新建 EventBus**；需回修 K1.4 状态机接入描述（直接喂 K1.4） |
| C3 | 主题 / 配色系统 | `src/theme/theme.ts` + `theme/tokens.css`（四主题，全 CSS 变量 + `<html data-theme>`） | 散落 `design-system.css`/`modules.css`/`index.css`/`user.css`/`splash.css` 中的硬编码色值 | 主题变量体系与硬编码色并存，切主题时硬编码处不跟随，呈「半主题化」 | Bloom 及新增组件一律读 `--nx-color-*` 变量（仿 `useSplashParticles::readBrandTokens`）；硬编码色逐步清理（与 D9 同源） |
| C4 | 蜂群（swarm）模型 | `nexustar-ui/src/store/swarmStore.tsx` + `screens/swarm/*`（固定 swarm 实体 + 实体卡） | archived `external/deepseek-tui`（v0.8.5 起以 `agent_spawn` **取代**固定 swarm 实体） | J3 固定实体卡设计与实证更优的 spawn 模型冲突（战略计划 D7 已决） | **采纳 spawn 模型（D7）**：`swarmStore` 改 spawn 生命周期，J3 实体卡延后至 Phase E |
| C5 | WebGL 渲染范式 | R3F（`VoxelWorld`、`@react-three/fiber`/`drei`，声明式） | Canvas2D（`useSplashParticles`/`useAudioParticles`，命令式 rAF） | 两套范式混用，团队易在新增视觉时选错基底（与 D3 同源） | 统一：持久型视觉走 three/R3F（Bloom），一次性入场动效可保留 Canvas2D，FFT/粒子数学抽成共享 util |
| C6 | 账号 / 数据隔离事实源 | PRD P0-3：复用 `nexustar-api` 现有 SQLite/账号模块做 `user_id` 分区（本地优先） | PRD P1-4 / Phase 3：新建 `admin-backend`（Java）统管会员/用户/库表 | 两套账号真相源（Python 本地 vs Java 云端）可能双写不一致 | 明确**单一事实源**：先 Python 本地（P0-3），Java 上线后仅同步，禁两端各写用户态（待用户拍板，PRD 待确认 #1/#7） |
| C7 | Tauri / 浏览器双栈 fetch | `api/tauri-fetch.ts`（`@tauri-apps/plugin-http`，无 internals 回退原生 fetch） | 纯浏览器 `window.fetch`（构建/测试环境） | 非真冲突——`tauri-fetch` 已内置回退；但若组件直连 `@tauri-apps/*` 会破坏纯 Web 构建 | **约束**：Bloom 及所有新组件经 `api/client.ts` 取 fetch，禁直连 `@tauri-apps/*`/`fsNative`/`WindowBar`（K2.1 §f.2） |

---

## 三、互补（Complementary）—— 分属不同仓/模块、应集成的关注点

> 互补指「分属不同仓/模块、关注点正交、应通过契约集成而非合并」的能力。三仓（active/archived/Marguerite）的整合本质是「互补集成」，而非代码合并（战略计划 D2/D3）。本节 M1–M4 为 archived→active 的研究反哺，M5–M8 为 active 内部前后端/壳/视觉的接缝，M9–M10 为运维与文档的权威源治理。

| # | 模块A | 模块B | 互补关系 | 集成方式 |
|---|-------|-------|----------|----------|
| M1 | Marguerite TUI（`marguerite/`，Rust Ratatui 三栏 Workbench） | `nexustar-ui`（Tauri WebView GUI） | 同一 UnivAgent 的「终端原生」与「图形原生」双形态，共享后端契约 | 建 **bridge 契约层**：两前端共用 `nexustar-api` OpenAPI + 统一类型；TUI 与 GUI 不共享代码，仅共享「会话/记忆/技能」数据契约（战略计划 D3） |
| M2 | archived `external/hermes-agent`（SQLite+FTS5 五层记忆 + 自进化闭环） | `nexustar-ui/src/store/memoryStore.ts` + `screens/MemoryLayersView.tsx` + `ProjectMemoryScreen.tsx` | hermes 分层记忆架构是 active 记忆模块参考实现 | **移植架构（非代码）**：五层模型落到 `memoryStore`，FTS5 检索下沉 `nexustar-api`；archived 仅作设计参考 |
| M3 | archived `external/deepseek-tui`（`agent_spawn`、LSP 后编辑诊断、`web_run` 无头浏览） | `nexustar-ui` swarm/orchestration + `terminalStore.tsx`/浏览器面板 | spawn 模型、终端诊断、无头浏览是可补能力 | 采纳 spawn（C4/D7）；LSP 诊断与 `web_run` 进 P1 执行链路增强（Bloom §11） |
| M4 | archived `external/openclaw`（Gateway + ClawHub 13K+ 技能市场生态） | `nexustar-ui` `marketplaceStore` + `SkillDrawer` | 技能市场/插件化思路可反哺 active 市场 | 借鉴 ClawHub 市场治理（评分/审核），**不搬 Gateway 重量实现**；技能元数据对齐提升 `marketplaceStore` 互通性（喂 D2 合并） |
| M5 | `nexustar-api/app/`（后端） | `nexustar-ui/src/api/`（前端客户端，已收敛 SSOT `client.ts`） | 天然前后端互补，API 契约唯一接缝 | 保持 100% 相对导入、无 `@/` 别名（K2.1 约束）；Bloom 若需后端走 `api/client.ts`（C7） |
| M6 | Bloom `Background`（K1.4 计划） | `components/music/Visualizer.tsx` + `useAudioParticles` + `api`（音频） | 二者都需 FFT→粒子驱动，音频数据应 bypass React state 直送 shader uniform | **共享 AudioAnalyzer + 双核 FFT（512 瞬时 + 2048 谐波）**（战略计划 §2.5），抽为 Bloom 引擎子模块；音乐可视化即 Bloom 的 AudioReactive 相位 |
| M7 | archived 自研模型（`gwwc/adaptive_model_cascade` 等） | `nexustar-api` P0-2 本地降级后端 | 线上不可达时加载旧自研模型，是 G2 可靠自包含关键 | 复制降级/级联逻辑进 `nexustar-api`，由 `backendStore` 切换；模型文件随归档快照保留，不依赖 archived 路径（喂 D8/D10） |
| M8 | `desktop/`（Rust/TS Tauri 壳，`src-tauri/`） | `nexustar-ui`（WebView 内容） | 原生壳承载窗口/sidecar/离线，GUI 渲染交互 | Bloom 须 `pointer-events:none` 且勿 import `WindowBar`/`fsNative`（K2.1 §f.2）；桌面能力按 D6 缓至质量门禁稳定后 |
| M9 | `nexusops/`（active 跨仓运维脚本：`api/`、`desktop/`、`nexustar-ui/`、`scripts/`） | archived `scripts/`、`services/`、`start_services.bat` | 运维/启动/部署工具两仓各有一份 | **保留 nexusops 为权威运维入口**；archived 脚本仅作历史参考，新运维逻辑不入 archived |
| M10 | `GUI_Agent/docs/`（active：K3 整合枢纽 + 30+ 审计/设计） | archived `docs/`（旧架构/约束文档） | active docs 已是权威，archived docs 含已演进前的旧约束 | **active 为唯一文档真相源**；archived docs 仅追溯用，新结论不写回 archived（避免 D2 式双源） |
| M11 | archived `external/` 其余工具（claude-code-source、claude-code-plugins、claude-mem、agent-s、ds4、opencode） | `nexustar-ui` / `nexustar-api` 对应能力 | 多为 Claude Code 系复刻 / 单一能力实验，与 active 全栈定位重叠度低 | **研究参考为主，低移植价值**：claude-mem 的记忆思路可并入 M2 五层记忆；其余仅作竞品佐证，不搬代码（见战略计划 K4-deep） |

---

## 四、汇总与建议（Top 5 整合动作，按质量/风险杠杆排序）

> 杠杆 = 风险下降幅度 ÷ 改动成本。括号内标注是否直接喂 K1.4（BloomBackground）。

**整体判断**：当前仓库的重复以「遗留 vs 活跃」为主（双官网、双技能、双 admin、gwwc 整包），冲突以「机制未归一」为主（状态 5 套、事件总线规划/现状错配、主题半变量化），互补以「archived→active 研究反哺」与「Marguerite TUI↔GUI 双形态」为主。整合不必大改，按「先立规范（C1/C2）→ 再收敛视觉（D3/M6）→ 后清遗留（D1/D6）」的节奏即可在低风险下显著提质。

1. **【最高杠杆】消解事件总线冲突（C2）→ 直接喂 K1.4。** 回修 K1.4 状态机接入，使其经 `useModeStore` + `api/sse.ts` 订阅，而非不存在的 EventBus。成本极低（仅文档对齐），却决定 Bloom 能否正确挂钩消息生命周期。**风险：高（接错则 Bloom 永不触发）。**
2. **统一前端状态机制（C1）。** 立规：所有特征态走 `core/store.ts` `create()``；冻结 `core/instances.ts`/`core/shu.ts` 仅作遗留/编排用。`store/bloomStore.ts` 已按此约定设计（K2.1 §e），作为范本推广。**风险：中（增量约束即可，无需大改）。**
3. **收敛粒子/WebGL 视觉引擎（D3 + C5 + M6）→ 喂 K1.4。** 以 `BloomBackground` 为唯一持久型 three/R3F 视觉基座，Splash 保留一次性 Canvas2D，音乐 `Visualizer` 复用其 `AudioAnalyzer`+双核 FFT。统一 `useFpsMonitor` 降级纪律（FPS<30 减半、<20 暂停）。**风险：高（包体积/FPS/视觉一致性）。**
4. **单官网 + 弃遗留（D1 + D6）。** 官网仅留 `nexustar-website-next`，删 v1 `src/` 与两处 `_deprecated_admin`。**风险：低，纯清理。**
5. **编排画布与聊天壳收敛（D4 + D5）。** 编排统一到 `OrchestrationCanvas`（swarm/gome 作模式）；聊天统一到 `unified` 壳，旧 `ChatSection` 随 `Agent/SwarmScreen` 重构后删除。**风险：中（跨屏状态互通需谨慎）。**

### 与 K1.4（BloomBackground）的强关联
- **必须做**：C2（事件接入）、C1（bloomStore 机制）、D3/C5/M6（视觉基座与音频共享）、K2.1 §e 挂载点（`app-root` 首子节点、`position:fixed;inset:0;zIndex:0;pointer-events:none`）。
- **禁止**：在 Bloom 中新建 EventBus、import `@tauri-apps/*`/`fsNative`/`WindowBar`、使用 `@/` 别名、复用 `SplashFlow` 一次性 rAF 生命周期。
- **喂入顺序**：先 C2/C1（地基）→ 再 D3/C5/M6（引擎）→ 最后挂载（K2.1 §e）。

### 合并执行范本（来自「〇、已整合基线」）
HTTP 客户端合并已验证可平滑进行：`client.ts` 成为 SSOT，`http.ts`/`client-root.ts` 退为兼容重导出壳。建议 D1–D6 沿用同法——先立 SSOT，旧件留兼容壳，随引用点重构逐步删除，避免回归。

### K1.4 事件接入修正对照（消解 C2 的执行清单）

战略计划 §2.4 为 Bloom 规划了 EventBus 事件，但仓库无 EventBus。下表给出**逐事件的实际接入机制**，供 K1.4 实现直接采用：

| §2.4 规划事件 | 实际机制（仓库现有） | 接入代码要点 |
|--------------|---------------------|-------------|
| `message:sent` → SOLID_APPEAR | `store/chatStreamStore` 的 send 流程 + `api/sse.ts` | 在 `sendMessage` 成功后置 `useBloomStore.getState().trigger('SOLID_APPEAR')` |
| `ai:thinking` → DISSOLVE | `api/sse.ts` 的 `ai:thinking` SSE 帧 | 订阅 SSE 事件总线（非自建 EventBus）转发至 bloomStore |
| `ai:response` → PARTICLE_BLOOM | `chatStreamStore` 收到首块响应 | 同上，首 token 到达即触发 |
| `audio:playing` → AudioReactive | `components/music/Visualizer` 的 AudioAnalyzer | 复用 M6 共享 FFT，直送 shader uniform |
| `mode:change` → 强度调整 | `store/modeStore`（`useModeStore`） | `useModeStore((s)=>s.mode)` 订阅，按 office/plan/max 调 intensity |

> 结论：Bloom 状态机不变，**仅把「EventBus 订阅」替换为「store.getState/subscribe + api/sse 监听」**，零新增全局机制，与 C1 规范一致。

### 待核实项（未逐文件验证）
- `nexustar-website/` v1 是否仍有线上流量或仅静态归档（据 `README-ARCHIVED.md` 推测已弃，待确认）。
- `gwwc/skills/builtin/hermes-skills` 与 `marketplaceStore` 技能 schema 差异度（需 diff 字段）。
- archived `gwwc/adaptive_model_cascade.py` 与 `nexustar-api` 现有模型降级逻辑重迭率（P0-2 移植成本待估）。
- Marguerite TUI 与 nexustar-ui 是否已存在 bridge 契约文件（战略计划称「独立 + bridge 文档对接」，具体文件待核实）。
- `store/swarmStore.tsx` 当前是否已部分采纳 spawn 模型，还是仍纯固定实体（影响 C4 落地工作量）。

---

## 五、整合路线图时序（对齐 PRD Phase 与战略计划）

| 阶段 | 关联矩阵项 | 动作 | 说明 |
|------|-----------|------|------|
| Phase 0（地基/可靠性） | C2、C1、D8/D10、M7 | 立状态/事件规范；移植自研模型降级逻辑 | 与 PRD P0-1/P0-2/P0-3 同步，不给 Bloom 留技术债 |
| Phase 0 末 / K1.4 启动 | C2、C1、D3/C5/M6 | 实现 `BloomBackground` + `bloomStore` | 严格按 K2.1 §e 挂载，禁 EventBus/`@tauri-apps` 直连 |
| Phase 1（MAX 体验） | D4、D5、C4 | 聊天统一 unified 壳；编排收敛；swarm 转 spawn | 删旧 `ChatSection`；`swarmStore` 改 spawn（D7） |
| Phase 2（执行链路） | M3、M4 | 采纳 deepseek-tui LSP 诊断 / openclaw 市场治理 | 进 P1 路线图（Bloom §11） |
| Phase 3（官网/账号） | D1、C6 | 单官网 v2；明确账号单一事实源 | 解决 C6 双源风险（待用户拍板） |
| 持续治理 | D9/C3、M9/M10、D6 | 样式变量化；运维/文档权威源收敛 | 低优先级清理，可随日常 PR 进行 |

## 六、风险矩阵（按严重度汇总）

| 严重度 | 项 | 触发后果 |
|--------|----|---------|
| 🔴 高 | C2（EventBus 不存在） | Bloom 状态机接空，特效永不触发 |
| 🔴 高 | D3/C5（视觉引擎分裂） | 包体积膨胀、FPS 不达标、Bloom 视觉不一致 |
| 🔴 高 | D5（编排画布 ≥4 处） | 跨屏编排状态不可互通，swarm/gome 数据孤岛 |
| 🟡 中 | C1（状态机制 5 套） | 特征态落错层，难以测试/迁移 |
| 🟡 中 | C3/D9（主题/样式碎片） | 切主题部分界面不跟随，选择器冲突 |
| 🟡 中 | D1/D2（双官网/双技能） | 版本漂移、技能隔离断裂 |
| 🟡 中 | C6（账号双事实源） | 用户态双写不一致 |
| 🟢 低 | D6/D7/D10、M9/M10 | 纯清理/职责已分离，回归风险小 |

## 七、方法论与证据来源

- **扫描手段**：`ls`/`find` 列目录；`Grep` 搜 `particle|theme|Store|Context|EventBus|client` 等关键词；逐文件读 `core/*`、`api/client.ts`、`api/http.ts`、`api/auth*.ts`、`store/chat*.tsx`、`App.tsx` 头部注释。
- **三角验证**：以 K2.1 模块图为基线，战略计划 §3.2/§3.3 给高层清单，PRD 给能力意图，三者交叉确认重复/冲突/互补归属。
- **不臆造原则**：所有路径来自本轮命令实证；无法确认者（如 v1 线上流量、技能 schema diff）一律标「待核实」，不填入确定性结论。
- **与既有交付物关系**：本矩阵是 K2.2（战略计划 §3.5 标注 ⏳ 待产出）。其结论直接喂 K1.4（C2/C1/D3/M6）、K2.4 迁移风险清单（第六节）、K3 整合文档（M1/M2 桥接）。

## 八、附录：PRD 能力域 × 矩阵项映射

> 将 PRD §2.2 六大能力域与本节矩阵项对齐，便于按产品优先级排整合顺序。

| PRD 能力域 | 涉及矩阵项 | 整合优先级 | 备注 |
|-----------|-----------|-----------|------|
| ① MAX 双带队（零冗余） | D4、D5、C4 | 高 | unified 壳 + 编排收敛 + swarm spawn，直接决定「零冗余」目标 |
| ② 执行链路增强 | M3、M4、D2 | 中 | deepseek-tui LSP 诊断 / openclaw 市场治理 / 技能合并 |
| ③ 账号与数据隔离 | C6、D2 | 高（阻塞） | 单一事实源未定会直接破坏 G3，需用户拍板 |
| ④ 官网（交流区/聊天室） | D1 | 中 | 单官网 v2，删 v1 防漂移 |
| ⑤ 桌面端后端（双栈降级） | D8、D10、M7、C2 | 高 | 自研模型移植 + backendStore 切换，喂 K1.4 事件接入 |
| ⑥ 控制面板（自研） | D6、M9 | 低 | 弃遗留 admin，nexusops 为运维权威源 |
| 横切：Bloom 视觉身份 | C1、C2、C3、D3、C5、M6 | 最高 | K1.4 直接依赖 C2/C1/D3/M6，须先于特效开发消解 |

## 九、一句话结论

UnivAgent 当前**真实重复集中在「遗留 vs 活跃」（双官网、双技能、双 admin）与「视觉引擎分裂」（R3F/Canvas2D/计划 Bloom）**；**最危险的冲突是 K1.4 规划的 EventBus 在仓库中并不存在（C2）**，须立即对齐；**三仓整合的本质是互补集成（M1–M10）而非代码合并**，其中 archived 的 hermes 五层记忆、deepseek-tui 的 spawn 模型、openclaw 的市场治理最值得反哺 active。

*文档结束。K2.2 矩阵基于 K2.1 模块图 + 两仓实际目录/代码扫描合成；路径来自本轮 `ls`/`find`/`Grep` 实证，未能确认处已标「待核实」，未做任何代码修改。*
