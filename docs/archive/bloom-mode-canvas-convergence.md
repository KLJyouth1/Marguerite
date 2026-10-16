# Bloom Mode — 编排画布收敛方案（D5 实证 recon）

> 路径：`D:\Downloads\Marguerite\bloom-mode-canvas-convergence.md`
> 日期：2026-08-05
> 关联：K2.4 迁移风险清单（D5 标识为 🔴 最高结构债）、Bloom Mode 工程标准 v2.0（R17 emoji 图标 / R18 紫粉渐变 / R19 硬编码色）
> 方法：consolidation-first（排雷先于重构，同 K2.2→K1.4 / K2.4→K1.5）

---

## 0. 执行摘要

D5 在 K2.4 中被标为「≥4 编排画布孤岛」。经实证排查（`ls`/`Grep`/`Read` 四个画布 + 四个底层 store），结论比预期更精确：

- **不是 4 个图形引擎，而是 3 个 ReactFlow DAG 画布 + 1 个非图形阶段/步骤时间线面板**，它们读的是 **4 套彼此独立的 store 模型**，描述的是**同一个「编排运行 / 智能体步骤」概念**。
- 真正的技术债 = **节点/边/状态词表 / 角色分类 / 布局算法 / 状态配色 在 4 处各写一遍**，且两套画布用 **emoji 作功能图标（违反 P0 R17）**。
- 收敛目标 = 统一 `OrchestrationRun` 数据模型（node/edge/run/status）+ 4 个纯函数适配器 + 1 个共享 `<OrchestrationGraph>` 渲染器（CockpitPanel 复用同一模型做时间线）。后端已统一（`app.event_bus`，见 K2.4 F1），本收敛纯前端、无外部阻塞。

**评级**：🟢 适配器层零行为风险可先行；🟡 共享渲染器重构需保留 AgentFlowCanvas 的 WASD/LOD/MiniMap 交互与 CockpitPanel 的独立 UX；**无外部阻塞，不依赖 C6**。

---

## 1. 实证盘点（已逐文件读源码确认）

### 1.1 四个画布清单（全部存在）

| # | 文件 | 技术 | 角色 |
|---|------|------|------|
| C1 | `components/orchestration/OrchestrationCanvas.tsx` | ReactFlow (@xyflow/react) | 编排 DAG（SSE 驱动） |
| C2 | `components/AgentFlowCanvas.tsx` | ReactFlow (@xyflow/react) | Agent 节点图（蜂群/Swarm 实时） |
| C3 | `components/orchestration/SwarmDAGView.tsx` | ReactFlow (@xyflow/react) | Swarm 定义 DAG（静态+实时着色） |
| C4 | `components/gome/CockpitPanel.tsx` | 纯 div 时间线（非 ReactFlow） | GoMe 任务驾驶舱（阶段/步骤/工具流） |

> 三者共用 `@xyflow/react` 技术栈，但各自独立实现 node/edge/model —— 这就是「孤岛」的本质：**同一库、四种方言**。

### 1.2 四个底层 store（根因）

| Store | 模型 | 关键字段 | 状态词表 |
|-------|------|----------|----------|
| `useOrchestrationStream` → `RunSnapshot` | 编排运行 SSE | `runId, goal, mode, layers:string[][], nodeStatus, nodeOutput, dag:{nodes:[{id,owner,deps:string[],priority}], edges:string[][]}` | 节点: pending/running/done/failed；运行: running/done/cache_hit |
| `swarmStore` → `SwarmStepView` | 蜂群步骤 | `nodes:Record<node_id, {node_id, agent_name, task, status, output, error, duration_ms, retry_count}>` | pending/running/done/failed |
| `agentStatusStore` → `AgentLive` | 全量心跳轮询 | `{id, name, cluster?, status, last_active}` + `AGENTS:AgentMeta[{name, cluster, role}]` | available/running/idle/failed |
| `gomeStore` → `GoMeTask`+`ToolCall` | GoMe 任务/工具流 | `GoMeTask{id,title,mode,phase,progress,...}`；`ToolCall{id,tool,args,result,durationMs,status,...}` | phase: queued/analyzing/executing/reviewing/completed/failed；tool: calling/done/error |

---

## 2. 分歧矩阵（逐字段对照，已核对源码）

### 2.1 节点标识（3 套互斥方案，无共享主键）

| 画布 | node.id 方案 | 含义 |
|------|-------------|------|
| C1 OrchestrationCanvas | 不透明 `nid` | 后端 `dag.nodes[].id` |
| C2 AgentFlowCanvas | `agent:${name}` | 前缀 + agent 名 |
| C3 SwarmDAGView | 不透明 `node_id` | 后端 swarm 节点 id |
| C4 CockpitPanel | `step.id`（`QuestStepLite`） | 步骤 id，无图边 |

→ 没有任何一个 id 能在另外三处直接引用。**跨画布无法对齐同一节点**。

### 2.2 节点数据模型（字段名/类型各写一遍）

| 字段 | C1 `data` | C2 `AgentNodeData` | C3 `data` | C4 `QuestStepLite` |
|------|-----------|-------------------|-----------|--------------------|
| 标识 | `label`(=nid) | `agentName` | `nodeId`/`agentName` | `id` |
| 角色 | `owner` | `role`(=cluster) | `role` | —（阶段在父级） |
| 状态 | `status` | `status` | `status` | `status` |
| 任务 | `output`(预览) | `task` | `task` | `title` |
| 优先级 | `priority` | — | — | — |
| 耗时 | — | `durationMs` | — | `started_at`/`completed_at` |
| 重试 | — | `retryCount` | — | — |
| 颜色 | — | `color`(自算) | — | — |
| 输出 | `output` | — | — | — |

### 2.3 边模型（两种表示法）

| 画布 | 边表示 | source/target 字段 | id 格式 |
|------|--------|--------------------|---------|
| C1 | **`string[][]` 元组**（无 id/type） | `dag.edges` = `[source,target][]` | 无（运行时拼 `${dep}→${nid}`） |
| C2 | 对象 | `source`/`target` | `e-${sId}-${tId}` |
| C3 | 对象 | `source`/`target` | `${s}->${t}` |

→ C1 的 `edges:string[][]` 与 C2/C3 的对象边**结构不兼容**，适配器必须做元组→对象转换。

### 2.4 状态词表（4 套枚举，部分重叠）

```
RunSnapshot 节点 : pending | running | done | failed
SwarmStepView    : pending | running | done | failed
AgentLive        : available | running | idle | failed
GoMeTask.phase   : queued | analyzing | executing | reviewing | completed | failed
CockpitPanel step: queued | running | completed | failed
```

→ 需归并为一个 **canonical 状态枚举**（见 §3）。`queued/analyzing/executing/reviewing` 仅 GoMe 用；`available/idle` 仅 AgentLive 用。

### 2.5 角色/簇分类（3 套分类法）

| 来源 | 分类法 |
|------|--------|
| C1 `owner` | researcher/architect/creator/reviewer/tester/optimizer |
| C2 `cluster` | reasoning/execution/orchestration |
| C3 `role` | researcher/architect/creator/reviewer/tester/optimizer/member |
| `AGENTS.role` | 同 C1 集 |

→ `owner` 与 `role` 实为同一概念两套名；`cluster` 是另一维度的分组（reasoning/execution/orchestration）。

### 2.6 重复实现（浪费 + 漂移风险）

| 重复项 | 出现处 | 说明 |
|--------|--------|------|
| 布局算法 | C1 层布局 / C2 网格-by-cluster / C3 Kahn 分层 | 3 套独立实现 |
| 状态→配色 | C1 `STATUS_COLORS` / C2 `STATUS_CONFIG`+`ROLE_COLORS`+`CLUSTER_HEX` / C3 `STATUS_COLORS` | 3+ 套，且 C2 用硬编码 rgba（R19 债） |
| 节点组件 | C1 `OrchNode` ≈ C3 `SwarmNode`（近同） / C2 `AgentNode`（3D 风） | 重复的卡片渲染 |
| **功能图标** | **C1 `OWNER_ICONS` emoji / C3 `ROLE_ICONS` emoji** | **违反 P0 R17** |

---

## 3. 统一数据模型（提案 · 决策点）

> 这是 D5 的核心设计决策，需用户确认后实施。

### 3.1 `types/orchestration-graph.ts`

```ts
export type GraphNodeStatus =
  | 'pending' | 'queued' | 'analyzing' | 'executing'
  | 'reviewing' | 'running' | 'done' | 'completed'
  | 'failed' | 'available' | 'idle';

export type AgentRole =
  | 'researcher' | 'architect' | 'creator' | 'reviewer'
  | 'tester' | 'optimizer' | 'coder' | 'analyst'
  | 'deployer' | 'office' | 'orchestrator' | 'member';

export interface GraphNode {
  id: string;          // 规范主键： `${runId}:${localId}`（localId 跨源各异，前缀消除歧义）
  runId: string;
  label: string;
  agentName?: string;
  role: AgentRole;
  status: GraphNodeStatus;
  task?: string;
  output?: string;
  error?: string;
  priority?: number;
  deps: string[];      // 依赖的 node id（统一存数组，C1 的 dep / C3 的 deps 归一）
  startedAt?: string;
  completedAt?: string;
  durationMs?: number;
  retryCount?: number;
  cluster?: string;    // reasoning/execution/orchestration（可选分组维度）
}

export interface GraphEdge {
  id: string;          // `${source}__${target}`
  source: string;
  target: string;
  kind?: 'dep' | 'sequence' | 'hub';
}

export interface OrchestrationRun {
  runId: string;
  goal: string;
  mode: string;        // hierarchical|swarm|consensus|solo|max|agent|expert_team
  status: GraphNodeStatus;
  nodes: GraphNode[];
  edges: GraphEdge[];
  aggregate?: string;
  metrics?: Record<string, number>;
  createdAt?: string;
  updatedAt?: string;
}
```

### 3.2 四个纯函数适配器（无副作用，可单测）

| 适配器 | 输入 | 输出 | 关键转换 |
|--------|------|------|----------|
| `runSnapshotToGraph` | `RunSnapshot` | `OrchestrationRun` | `edges:string[][]` → `GraphEdge[]`；`owner`→`role`；`nodeStatus` 注入 `status` |
| `swarmStoreToGraph` | `Record<node_id, SwarmStepView>` + 定义 DAG | `OrchestrationRun` | `node_id` 作主键；`deps` 已在定义 DAG |
| `agentStatusToGraph` | `AgentLive[]` + `AGENTS` | `OrchestrationRun` | `cluster`→`cluster`，`status` 直映；无边的「名册」视图 |
| `gomeTaskToGraph` | `GoMeTask` + `ToolCall[]` + steps | `OrchestrationRun` | `phase`→`status`；steps → 无依赖节点；tool calls → 可选子节点/注释 |

### 3.3 规范状态元信息（消除 3+ 套配色副本）

`lib/orchestrationStatus.ts` —— **单一真相源**，全部走 design token（非硬编码 rgba，修 R19）：

```ts
export const STATUS_META: Record<GraphNodeStatus, { label: string; token: string; pulse: boolean }> = {
  pending:   { label: '等待中',  token: 'var(--nx-color-status-muted)',   pulse: false },
  queued:    { label: '排队中',  token: 'var(--nx-color-status-muted)',   pulse: false },
  analyzing: { label: '分析中',  token: 'var(--nx-color-cat-cyan)',       pulse: true  },
  executing: { label: '执行中',  token: 'var(--nx-color-cat-cyan)',       pulse: true  },
  reviewing: { label: '复核中',  token: 'var(--nx-color-cat-purple)',     pulse: true  },
  running:   { label: '运行中',  token: 'var(--nx-color-cat-green)',      pulse: true  },
  done:      { label: '已完成',  token: 'var(--nx-color-cat-green)',      pulse: false },
  completed: { label: '已完成',  token: 'var(--nx-color-cat-green)',      pulse: false },
  failed:    { label: '失败',    token: 'var(--nx-color-status-danger)',  pulse: true  },
  available: { label: '在线就绪', token: 'var(--nx-color-cat-cyan)',       pulse: false },
  idle:      { label: '空闲',    token: 'var(--nx-color-status-muted)',   pulse: false },
};
```

### 3.4 共享渲染器

`components/orchestration/OrchestrationGraph.tsx`（ReactFlow）：
- Props：`run: OrchestrationRun`、`layout: 'layer' | 'kahn' | 'grid'`、`nodeVariant: 'orch' | 'swarm' | 'agent'`、`showMiniMap?`、`lodThreshold?`、`immersive?`、`onNodeClick?`。
- 内部：`lib/graphLayout.ts` 收敛 3 套布局算法为 `layoutGraph(run, strategy)`；单一 `GraphNodeView`（按 `nodeVariant` 切卡片/3D 风）；图标走项目锁定的 SVG 集（同 CockpitPanel 的 `SvgIcon`），**根除 emoji（修 R17）**。
- `components/orchestration/OrchestrationTimeline.tsx`（CockpitPanel 复用）：吃 `OrchestrationRun`，按 `status` 映射阶段/步骤，保留其独立竖向 UX。

---

## 4. 分阶段收敛计划（低行为风险）

| 阶段 | 动作 | 行为变更 | 风险 |
|------|------|----------|------|
| **A** | 新增 `types/orchestration-graph.ts` + `lib/orchestrationStatus.ts` + 4 适配器（纯函数） | 无（仅加代码，不接 UI） | 🟢 |
| **B** | 新增 `OrchestrationGraph` + `OrchestrationTimeline` + `graphLayout.ts`；状态配色统一；emoji→SVG | 视觉一致化，交互不变 | 🟢（emoji 机械替换） |
| **C** | 四个画布改写：C1/C3→`OrchestrationGraph` 薄封装；C2→合并 `agentStatusToGraph`+`swarmStoreToGraph`；C4→`gomeTaskToGraph`+时间线 | 功能等价，视觉统一 | 🟡（保留 C2 WASD/LOD/MiniMap、C4 UX） |
| **D** | 单测（适配器 round-trip）+ `tsc --noEmit` + emoji 正则扫描 + P0 复检 | 验证 | 🟢 |

> 实施顺序遵循 consolidation-first：A（排雷，可独立验收）→ B → C → D。

---

## 5. 范围边界（诚实声明）

- **纳入 D5**：C1/C2/C3 三个 ReactFlow DAG 画布 + C4 时间线面板，及其 4 个 store 模型的归一。
- **相邻但不在本 recon 范围**（已在战略计划 D4/D11 单列）：
  - `DualLeaderPanel.tsx` / `workbuddy/MaxImmersive.tsx` / `workbuddy/PipelineReview.tsx`（MAX 双领队/管线视图）—— 属 Phase 1 MAX 子系统（D4/D11），后续可复用 `OrchestrationRun` 模型。
  - `KnowledgeGraphView.tsx`（实体关系图）—— 不同概念（知识实体 vs 运行 DAG），不归并。
  - `SwarmCard.tsx`（蜂群摘要卡）—— 非图，不归并。
- **P0 关联清理**：C1/C3 的 emoji 图标（R17）在本收敛同 PR 机械替换为 SVG；C2 的 `linear-gradient(135deg, rgba(59,130,246,...))` + 硬编码 rgba（R19）**属既有债，本 PR 修 emoji 但不强制清硬编码色**（避免范围蔓延），列为持续治理项。

---

## 6. 风险再评

| 项 | 评 | 说明 |
|----|----|------|
| 外部阻塞 | 无 | 后端 `app.event_bus` 已统一（K2.4 F1）；不涉及 C6 账号单源 |
| 适配器回归 | 🟢 | 纯函数 + round-trip 单测可先行验收 |
| 共享渲染器 | 🟡 | 须保留 C2 交互特性与 C4 UX，用 variant prop 而非强改 UX |
| emoji 替换 | 🟢 | 锁定 SVG 集需覆盖 6 owner + 空态；机械替换 |
| 工作量 | ~1.5–2 天 | 4 新增 + 4 改写 + 4 适配器，量级同 K1.4/K1.5 |

---

## 7. 决策待确认（唯一交互点）

D5 收敛的**核心设计决策**是 §3 的统一 `OrchestrationRun` 模型 + 适配器模式。请在确认后进入实施（D5-impl，可派 worker 执行 A→D）。

若同意，下一轮直接落地 Phase A+B（模型+适配器+共享渲染器骨架），再做 C 改写与 D 验证。
