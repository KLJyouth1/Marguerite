# Bloom Mode — Phase 1 MAX 子系统收敛侦察（实证 recon）

> 路径：`D:\Downloads\Marguerite\bloom-mode-max-subsystem-recon.md`
> 日期：2026-08-06
> 关联：`bloom-mode-canvas-convergence.md`（D5 编排画布 recon，模板源）、Bloom Mode 工程标准 v2.0（R17/R18/R19/R20）、Phase 1 MAX 子系统（D4/D11）
> 方法：read-only recon（逐文件读源码 + 正则扫描），不修改任何文件，不跑 git，不起子代理
> 注：`src/types/orchestration-graph.ts`（D5 模型文件）本轮由另一 worker 并行创建，尚未存在；统一模型以 `bloom-mode-canvas-convergence.md` §3 为准。

---

## 0. 执行摘要

本 recon 对 Phase 1 MAX 子系统三个候选组件做实证盘点，结论：

- **三个候选全部存在，但路径与 D5 不同**：`DualLeaderPanel.tsx`（位于 `src/components/`，非 `orchestration/`）、`MaxImmersive.tsx`、`PipelineReview.tsx`（均位于 `src/components/workbuddy/`）。
- MAX 子系统本质 = **三种不同的"编排运行"表达**：双领队双轨卡片（DualLeaderPanel）、沉浸式管线 + AgentFlowCanvas（MaxImmersive）、GoMe 阶段管线 + 真 DAG（PipelineReview）。它们读 **3 套独立 store / 类型**，描述的概念与 D5 的 `OrchestrationRun`（node/edge/run/status/role）高度重叠，但**字段名、状态词表、阶段划分各写一遍**。
- **P0 硬门结果**：R17（emoji 功能图标）**命中**（DualLeaderPanel 9 处 + MaxImmersive 1 处）；R18（紫→粉/靛→粉渐变）**不命中**（仅见 cyan→purple 的 stream 装饰渐变）；R19（硬编码色）**命中**（MaxImmersive 最严重，大量游离 hex/rgba）；R20（bounce 缓动）**不命中**。
- **复用结论**：✅ 应复用 D5 的 `OrchestrationRun` 模型 + 适配器 + `<OrchestrationTimeline>`；PipelineReview 的 `SwarmDAGView` 已是 D5 的 C3 画布，可直接并入。需新增第 5 个适配器 `dualLeaderToGraph`。**双领队左右分轨布局是 MAX 专属 UX，保持独立封装**。

**评级**：🟢 模型/适配器层零行为风险可先行；🟡 渲染器改写须保留双轨布局与沉浸式交互；**无外部阻塞**。

---

## 1. 实证盘点

### 1.1 文件清单（全部存在）

| # | 文件 | 技术 | 渲染内容 | 数据源 |
|---|------|------|----------|--------|
| M1 | `src/components/DualLeaderPanel.tsx`（707 行） | 纯 div 双轨 + StageIndicator + EventStream | 左 Flash 蜂群列（≤11 SwarmAgentCard）+ 右 Pro 专家列（主/子专家）+ 顶部阶段条 + 底部事件流 + 分层安全扫描 drawer | `useDualLeaderStore`（`src/store/dualLeaderStore`）+ 受控 props；SSE 事件经 `effectiveEvents`；`/security/scan/*` API |
| M2 | `src/components/workbuddy/MaxImmersive.tsx`（462 行） | `AgentFlowCanvas`（immersive+miniMap）+ 浮动面板 | 顶部 5 阶段管线条（⚡MAX）+ 全屏画布 + 左上任务委派 + 左下蜂群分片 + 右下专家团队 | `useWorkBuddyStore`（maxStage/maxProgress）+ `useSwarmStore`（nodes）+ `useAgentStatusStore`（byName）+ SSE `/max/dual-leader/stream` + `fetchGoMeTasks` 轮询 |
| M3 | `src/components/workbuddy/PipelineReview.tsx`（354 行） | 阶段管线条 + 产物 Tab + `SwarmDAGView`（ReactFlow） | `AgenticPipeline`（4 阶段）+ `ArtifactReview`（report/code/files/security）+ `SwarmDagCard`（真 DAG） | props 驱动（`selectedTask`/`artifacts`/`artifactTab`），不直接依赖 gomeStore；`fetchSwarmDefinition`/`deriveSwarmDag` |

> 关联但未改写在本 recon 范围：`src/components/chat/MaxDualPanel.tsx`、`src/components/MaxModeBanner.tsx`、`src/screens/MaxWorkbenchScreen.tsx`（198 行，MAX 入口屏）。

### 1.2 导出组件 + props 形状

**M1 `DualLeaderPanel`** — `export function DualLeaderPanel(props: DualLeaderPanelProps)`：
```ts
interface DualLeaderPanelProps {
  state?: DualLeaderState;            // 受控：来自 src/types/dualLeader
  events?: DualLeaderEvent[];
  eventStreamMax?: number;            // 默认 10
  'data-testid'?: string;
  securityScans?: Record<string, SecurityScanBadgeResult>;
  currentSkillId?: string;
  onSecurityScanError?: (e: Error) => void;
}
```
再导出 `StageIndicator` / `SwarmAgentCard` / `EventStream` 及类型，向后兼容。

**M2 `MaxImmersive`** — `export default function MaxImmersive()`：**零 props**（纯 store/SSE 驱动）。内部子组件 `MaxPipelineBar`（`{currentStage: MaxPipelineStage; progress}`）、`SwarmFloatingPanel`（`{nodes: Record<string,SwarmStepView>}`）、`ExpertFloatingPanel`（无 props）。

**M3 `PipelineReview`** — `export default function PipelineReview(props: PipelineReviewProps)` + 命名导出 `AgenticPipeline`/`ArtifactReview`/`SwarmDagCard`：
```ts
interface PipelineReviewProps {
  selectedTask: { phase: GoMePhase; progress: number; swarmId?: string } | null;
  artifacts: GoMeArtifact[];
  artifactTab: ArtifactTab;
  onTabChange: (tab: ArtifactTab) => void;
  onRefreshSwarm?: () => void;
}
```

---

## 2. 数据模型与分歧矩阵（已核对源码）

### 2.1 节点/边模型

| 概念 | M1 DualLeaderPanel | M2 MaxImmersive | M3 PipelineReview | D5 `OrchestrationRun` |
|------|--------------------|-----------------|-------------------|------------------------|
| **节点** | `SwarmAgent{id,role,goal,status,output}` + `ExpertMember{id,role,goal,status,output}`（`src/types/dualLeader.ts:39-58`） | `SwarmStepView`（store）状态 + `AgentLive`（statusMap，按 name） | `GoMeTask`/steps → 经 `SwarmDAGView` 已是 `GraphNode` 形态 | `GraphNode{id,runId,label,agentName?,role,status,task?,output?,deps[],...}` |
| **节点 id 主键** | 不透明 `id`（各自生成） | `node_id` / agent `name` | Swarm DAG `node_id` | 规范 `${runId}:${localId}` |
| **边/依赖** | **无**（双轨平行，无跨节点边） | **无**（仅 floating 列表） | `SwarmDAGView` 已有 `edges`（即 D5 C3） | `GraphEdge[]{source,target,kind?}` |
| **角色分类** | 自由字符串 `role`（architect/engineer/qa/main_expert/reviewer/formatter） | `cluster` 维度（reasoning/execution）走 AgentLive | `role` 由 Swarm DAG 提供 | `AgentRole`（12 值固定并集）+ `cluster?` |
| **运行级阶段** | `DualLeaderStage`：analyze/plan/execute/aggregate/**self_repair**/refine（6 阶段） | `MaxPipelineStage`：idle/analyze/plan/execute/aggregate/refine/done（去 self_repair，加 idle/done） | `GoMePhase`：queued/analyzing/executing/reviewing/completed/failed | `mode:'max'`，`status:GraphNodeStatus`；阶段需另设字段 |

### 2.2 状态词表（3+ 套，部分重叠）

```
M1  AgentStatus         : pending | running | completed | failed
M2  SwarmStepView       : pending | running | done | failed | waiting（store 实际）
M2  AgentStatus(byName) : running | ...（agentStatusStore）
M3  GoMePhase           : queued | analyzing | executing | reviewing | completed | failed
D5  GraphNodeStatus     : pending|queued|analyzing|executing|reviewing|running|done|completed|failed|available|idle
```

→ **关键分歧**：M1 用 `completed`，M2/M3 用 `done`/`completed` 混用（SwarmStepView=`done`，GoMePhase=`completed`）。D5 模型两者都收，但**适配器必须做 `completed↔done` 归一**（M1 的 `completed`→D5 `done` 或保留为 `completed`，需定调）。

### 2.3 阶段划分分歧（MAX 专属）

- M1 双领队 6 阶段含 `self_repair`，M2 5 阶段不含 `self_repair`（合并进 refine）但含 `idle/done` 包裹态。
- 两套阶段**命名不同但语义同构**（analyze→plan→execute→aggregate→refine 是公共骨架）。
- D5 的 `OrchestrationRun.status` 是节点级状态，**没有"运行级管线阶段"字段**。→ 复用须决定：将 MAX 阶段映射为 `run.metrics`/新增 `run.pipelineStage?: MaxPipelineStage`，或把每个阶段当作一个汇聚节点（graph 化）。

### 2.4 重复实现（浪费 + 漂移风险）

| 重复项 | 出现处 | 说明 |
|--------|--------|------|
| 阶段/管线进度条 | M1 `StageIndicator`、`M2 MaxPipelineBar`、`M3 AgenticPipeline` | 3 套独立圆点+连线渲染，**逻辑同构、样式各写** |
| 状态→配色 | M1 `SEVERITY_COLOR`、M2 内联 ternary（`#22c55e`/`#ef4444`/`#f87171`…）、M3 `SEVERITY_COLORS` | 3 套，且 M2 大量硬编码（R19 债） |
| 双轨 agent 卡片 | M1 `AgentTrack`/`SwarmAgentCard` | MAX 专属，无 D5 对应 |
| 真 DAG | M3 `SwarmDAGView` | **即 D5 的 C3 画布**，应直接并入收敛，而非另写 |

---

## 3. 统一模型复用提案（决策点）

### 3.1 直接复用 D5 资产

- `OrchestrationRun`（含 `mode:'max'` 已预留）：每个 MAX 运行 = 一个 `run`。
- `GraphNode`：SwarmAgent / ExpertMember / SwarmStepView / GoMeTask-step 统一为节点；`role` 映射到 `AgentRole`（M1 自由字符串需做 `role`→`AgentRole` 归一，未知角色落 `member`）。
- `deps:string[]`：M1/M2 无依赖可留空；M3 的 Swarm DAG 边直接转 `GraphEdge[]`（已天然兼容 C3）。
- **新增第 5 适配器 `dualLeaderToGraph(state: DualLeaderState, events?): OrchestrationRun`**：
  - 左轨 `swarmAgents`（cluster='flash'）→ nodes；右轨 `expertTeam.mainExpert+subExperts`（cluster='pro'）→ nodes。
  - `id` 规范化为 `${runId}:${id}`；`status` `completed`→`done`。
  - `run.status` 由 `state.stage` 推导；`run.mode='max'`。
- `OrchestrationTimeline`（D5 §3.4）：M1 `StageIndicator` + M2 `MaxPipelineBar` + M3 `AgenticPipeline` 三者可统一到同一 Timeline 渲染器（按 `MaxPipelineStage`/`DualLeaderStage`/`GoMePhase` 映射阶段序列），消除 3 套重复。

### 3.2 不应并入的部分（保持独立封装）

- **双领队左右分轨布局**（M1 `AgentTrack` 双列 + M2 浮动双面板）：是 MAX 的招牌 UX，**不属于通用 graph/timeline 渲染器**，作为 `OrchestrationRun` 之上的"双轨视图层"独立存在，仅消费适配器输出。
- **沉浸式全屏交互**（M2 `AgentFlowCanvas immersive` + 浮动委派/看板）：保留，仅把管线条换成 Timeline 变体。
- **分层安全扫描 drawer**（M1 `ScanDetailDrawer`）：与编排模型无关，保持独立。

### 3.3 状态词表归一（复用 D5 §3.3 `STATUS_META`）

- 统一 `pending|running|done|failed` 为 canonical；`completed` 在 `dualLeaderToGraph` 中归并为 `done`。
- M3 `GoMePhase` 的 `queued/analyzing/executing/reviewing/completed` 直映 D5 枚举（已完成 M3 已覆盖）。

---

## 4. 分阶段收敛计划（低行为风险，沿用 D5 A→D）

| 阶段 | 动作 | 行为变更 | 风险 |
|------|------|----------|------|
| **A** | 新增 `dualLeaderToGraph` 适配器（纯函数）+ 复用 D5 `types/orchestration-graph.ts` | 无（仅加代码） | 🟢 |
| **B** | 复用 D5 `OrchestrationTimeline` 渲染器变体，承载 M1/M2/M3 三套阶段条；统一 `STATUS_META` 配色；emoji→SVG（修 R17） | 视觉一致化，交互不变 | 🟢 |
| **C** | M1 改 `useDualLeaderStore`→`dualLeaderToGraph`+双轨视图层；M2 管线条换 Timeline；M3 `SwarmDAGView` 直接并入 D5 C3 改写 | 功能等价，视觉统一 | 🟡（保留双轨布局 + 沉浸式交互） |
| **D** | 适配器 round-trip 单测 + `tsc --noEmit` + emoji 正则扫描 + P0 复检（R19 清硬色） | 验证 | 🟢 |

---

## 5. 范围边界（诚实声明）

- **纳入本次 MAX recon 收敛**：M1 DualLeaderPanel、M2 MaxImmersive、M3 PipelineReview 三组件对 `OrchestrationRun` 模型的复用；阶段条统一到 `OrchestrationTimeline`；新增 `dualLeaderToGraph`；M3 的 `SwarmDAGView` 并入 D5 C3。
- **相邻但不在本 recon 范围**（已在 D5 / 其他 recon 单列）：
  - D5 四画布（OrchestrationCanvas / AgentFlowCanvas / SwarmDAGView / CockpitPanel）本身。
  - `MaxDualPanel.tsx` / `MaxModeBanner.tsx` / `MaxWorkbenchScreen.tsx`：MAX 入口/横幅层，未在本 recon 逐行核查（仅列名确认存在），建议后续补 recon。
  - `KnowledgeGraphView.tsx`：知识实体图，不归并。
- **P0 关联清理**：
  - **R17 emoji**：M1/M2 的 emoji 图标在本收敛同 PR 机械替换为项目锁定 SVG 集（需覆盖 🔍✕📁📋💡🔎🔬⏳📡🛡️⚡）。
  - **R19 硬编码色**：M2 `MaxImmersive` 的 MAX 红主题 `#ef4444`/`#f87171`/rgba(239,68,68,…) 及游离状态 hex（line 126/196-198/263-264）**属重债，本 PR 应清**；M1/M3 多为 `var(--nx-* , fallback)` 的 fallback 形式（治理项，可接受但建议逐步去 fallback）。

---

## 6. 风险再评

| 项 | 评 | 说明 |
|----|----|------|
| 外部阻塞 | 无 | 后端 `max_orchestrator` 已统一双领队阶段；SSE `/max/dual-leader/stream` 已存在 |
| 适配器回归 | 🟢 | `dualLeaderToGraph` 纯函数 + round-trip 单测可先行验收 |
| 双轨布局保留 | 🟡 | M1 左右轨是招牌 UX，须用"双轨视图层消费 OrchestrationRun"而非强改 UX |
| emoji 替换 | 🟢 | 锁定 SVG 集覆盖 11 个图标；机械替换 |
| R19 清硬色 | 🟡 | M2 红色主题硬编码量大，需决定 token 化（新增 `--wb-max-*` 变量）或保留品牌色豁免 |
| 工作量 | ~1–1.5 天 | 1 新增适配器 + 3 改写 + Timeline 复用，量级小于 D5 |

---

## 7. 决策待确认（唯一交互点）

MAX 子系统收敛的**核心决策**是：① 接受新增 `dualLeaderToGraph` 第 5 适配器；② `OrchestrationRun` 是否新增可选 `pipelineStage` 字段以承载 MAX 运行级阶段（还是把阶段 graph 化）；③ 双轨布局作为独立视图层是否认可其"保持分离"。请确认后进入实施（可派 worker 执行 A→D）。
