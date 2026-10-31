# Marguerite 工作台整体性跃迁总纲（0.2.20 → 0.3.x）

> **日期**：2026-10-29 ｜ **基线**：0.2.19（含发行链路修复）｜ **性质**：总纲计划，不是实施记录
> **纪律**：不重写引擎、不新增第五个前端、不绕过门禁、不在没有新鲜证据时声称完成（`AGENTS.md` §8）。
> **本文所有路径均指本仓内真实存在的文件**；`artifacts/` 下的调研记录**不在此引用**（该目录不随仓库发布，引用会变成死链）。

---

## 0. 一句话结论

工作台（引擎 + 四入口）已经"能用"，下一步不是再加视图，而是把**上下文、记忆、知识库、智能体、协同**这五层从"各自有模块"变成"围绕同一个用户与同一个项目互相供料"，并在其上做出**两层助理**：全局助理管跨项目的历史与偏好，项目助理管当前项目的拆解与并行。两层之间只传"不可推导的东西"，越用越省解释。

---

## 1. 现状盘点（只写有代码证据的）

### 1.1 已经存在的骨架（不必重造）

| 层 | 已有模块（`core/src/`） | 已暴露 RPC（`gateway/protocol.rs`） |
|---|---|---|
| 上下文 | `context/{pressure,spool,compact_state,middle_truncate}.rs`（M1 已落地） | `sessions.messages.*`、`usage.status` |
| 记忆 | `memory/{layers,tiered,sensory,short_term,long_term,recall,scoring,synapse,cross_project,behavior,experience,pages,home_memory,federated}.rs` | `memory.search/save/list`、`memory.layers.*`、`memory.recall.search`、`memory.cross_project.search/expand`、`memory.behavior.*`、`memory.synapse.status`、`memory.tiered.status`、`memory.inject.last` |
| 知识库 | `kb/{store,graph,project_graph,hybrid,integrity,denoiser}.rs` + `codeintel/` | 由 kb 网关与工具面消费 |
| 智能体 | `agent/{runtime,stream_loop,multi_strategy,subagent,team,orchestration,capability_cards,capability_guard,tool_schema,tool_catalog,task_checkpoint,trellis,profile,role_context,expertise,budget_policy,turn_budget}.rs` + `orchestration/`、`plan/`、`goal/`、`harness/` | `team.*`、`chat.*`、`tools.*` |
| 协同 | `collab/`（SSH）、`team.rs`、`harness/`、webui `TeamBoard` | `collab.ssh.*`、`team.members` |

### 1.2 真实欠账（已定位，不是猜测）

1. **`sessions.fork` / `resume` / `replay` 在 Rust 侧仍是 0 实现**：前端 `forkTransition.ts` 已上线，纯 Rust 部署点"分支会话"只会得到"需要 Python 侧车"的提示。
2. **上下文接续只做了一半**：`CompactState` 会被写出，但**新会话不会自动载入最近 N 份**，也没有开场注入 `git status / diff --stat`。
3. **两层助理只有零件没有接口**：`memory/cross_project.rs` 已能跨项目检索 `project_memory.md` / `topics.md`，但没有"全局助理"这个概念，也没有"只传不可推导信息"的注入契约。
4. **记忆只进不出**：写入路径齐全，缺**晋升/遗忘/冲突消解**的统一策略；`memory.behavior.*` 有采集，没有回灌成偏好。
5. **知识库没有增量**：`kb` 有能力做图与混合检索，但没有"文件变更 → 局部重建"的增量链路。
6. **协同停在单体**：`team.members` 能列出成员，但没有"多任务并行 + 依赖 + 回滚"的统筹层。

---

## 2. 目标形态（一页纸）

```
┌──────────────────────────── 用户 ────────────────────────────┐
│  目标 / 偏好 / 历史（跨项目）        当前项目 / 当前任务        │
└──────────┬────────────────────────────────────┬──────────────┘
           ▼                                    ▼
   ┌───────────────┐   只传「不可推导」  ┌───────────────────┐
   │ 全局助理 L1    │ ─────────────────▶ │ 项目助理 L2        │
   │ GlobalAssistant│ ◀───────────────── │ ProjectAssistant   │
   └───────┬───────┘   只回写「可复用」  └─────────┬─────────┘
           │                                      │
   跨项目记忆 · 用户画像 · 长期目标        项目记忆 · 项目图谱 · 任务树
   （memory/cross_project · profile）    （kb/project_graph · plan · goal）
           │                                      │
           └──────────────┬───────────────────────┘
                          ▼
        ┌─────────────────────────────────────────────┐
        │  统一的 上下文层 / 记忆层 / 知识库层（现有）  │
        └─────────────────────────────────────────────┘
```

**两条硬红线**（否则会互相污染，越用越乱）：

- **L1 → L2 只传不可推导的东西**：偏好、长期约束、跨项目已确认的事实。不传原始对话、不传代码片段。
- **L2 → L1 只回写可复用的结论**：一句话结论 + 出处（项目 + 文件/会话 id）。不整段搬运上下文。

---

## 3. 五层架构的增补设计

### 3.1 上下文层（Context）—— 补完 M1 的"接续"

| 项 | 落点 | 验收 |
|---|---|---|
| 新会话自动载入最近 N 份 `CompactState` | `context/compact_state.rs` + `gateway/sessions.rs` | 单测：伪造 3 份状态，新会话的系统提示含最近 1 份的 `{branch, dirty, test_status}` |
| 开场注入 `git status --short` + `diff --stat` | `agent/role_context.rs`（已有角色上下文装配点） | 单测：脏工作区 → 注入行非空；干净工作区 → 不注入（避免噪声） |
| Prompt Scaffold 重建 | `agent/tool_schema.rs` + `agent/prefix_scope.rs` | 现有"稳定半区只增不减"守卫不得变红 |

### 3.2 记忆层（Memory）—— 从"能存能搜"到"会取舍"

新增**写入闸门**（`memory/scoring.rs` 已有打分，补策略层）：

- **晋升**：`sensory → short_term → long_term` 的阈值不再由调用方各写一套，统一 `memory::promote::decide(entry) -> Promote|Keep|Drop`。
- **遗忘**：长期层按"最后命中时间 × 重要度"衰减，超过阈值的降级而不是删除（可回溯）。
- **冲突消解**：同 key 不同值时，保留新的并把旧的标 `superseded_by`，检索只返回最新一条。
- **画像**：新增 `memory/profile.rs`（持久化 `~/.marguerite/user_profile.md`），只存**稳定偏好**（语言、技术栈、禁忌），不存一次性任务。

验收：`memory.stats` 增加 `promoted/dropped/superseded` 三个计数；`memory.recall.search` 在注入内容里附"为什么召回"（来源 + 分值），不允许黑箱。

### 3.3 知识库层（Knowledge Base）—— 增量与边界

- **增量索引**：`kb/store.rs` 增加 `apply_changes(paths)`，只重建受影响文件的图节点；触发点接 `tools/fs` 写入与 `worktree` 变更事件。
- **边界**：知识库只放**可公开推导的代码/文档事实**；用户偏好与跨项目结论一律走记忆层。两边不互相写。
- **完整性**：`kb/integrity.rs` 增加"索引指针 vs 磁盘 mtime"校验，`kb.graph` 启动时自检，坏索引直接重建而不是静默返回空。

### 3.4 智能体层（Agent）—— 缺的是"可中断、可回滚、不极化"

| 批次 | 内容 | 落点 |
|---|---|---|
| A-1 | `sessions.fork/resume/replay`（M2）：fork 时复制 `CompactState` + 消息前缀，`resume` 从 checkpoint 续 | `session/`、`gateway/sessions.rs`、`persistence/` |
| A-2 | 长任务熔断：连续 5 错 / 60s 错率 >15% / 同参数循环 3 步 → 暂停并可回滚 | `agent/task_checkpoint.rs`、`agent/tool_cycle.rs`、`agent/worktree.rs` |
| A-3 | 辩论防极化：论点独立性校验（cos>0.85 / 引用 Jaccard>0.6 / ROUGE>0.8 判附和，权重 ×0.2）、每轮强制新证据、按证据链强度而非人头收敛 | `agent/team.rs`、`orchestration/` |
| A-4 | 工具面纪律：首轮工具面固定（已有 `FIXED_FACE`），新增能力只能"发现即追加" | `agent/tool_schema.rs`（已有守卫，勿破） |

### 3.5 协同层（Collaboration）—— 从"列成员"到"统筹"

- **任务树**：`plan/` 已有计划模型，补 `TaskNode{deps, owner, status, checkpoint}`，支持 DAG 并行与依赖等待。
- **并行调度**：`orchestration/` 按 DAG 派发，单节点失败不 abort 全局（已有子专家韧性的经验，复用同一策略）。
- **可见性**：webui `TeamBoard` 已有进度帧合并逻辑，接真实 `TaskNode` 状态；新增 `tasks.tree` / `tasks.control` 两个 RPC。

---

## 4. 两层助理：接口与契约

### 4.1 新的 RPC 面（最小集）

```
assistant.scope.get            → { scope: "global"|"project", workspace, since }
assistant.scope.set            → 切换；切到 project 时必须带 workspace
assistant.brief.get            → L1/L2 的注入简报（结构化，不是自由文本）
assistant.brief.pin / unpin    → 用户手动钉住的偏好/约束（最高优先级，永不被自动覆盖）
```

### 4.2 `assistant.brief` 的结构（这是"越用越懂你"的载体）

```json
{
  "scope": "project",
  "pinned":     [{"text": "所有文档用中文", "source": "user", "at": "..."}],
  "preferences":[{"key": "package_manager", "value": "npm", "confidence": 0.9, "evidence": 3}],
  "facts":      [{"text": "0.2.19 修了 updater 缺 pubkey 导致的启动 panic", "source": "project:marguerite#0.2.19", "score": 0.82}],
  "open_loops": [{"text": "sessions.fork 仍未实现", "source": "plan:m2", "at": "..."}],
  "budget":     {"max_chars": 1200, "used": 640}
}
```

规则：

- **`pinned` 永远排在最前且不参与截断**。
- 每条 `facts` 必须带 `source` 与 `score`，前端可展开看"凭什么说这句"。
- 超预算按"pinned → preferences → facts（按分值）→ open_loops"的顺序裁剪，**裁剪要记日志**（`memory.inject.last` 已有先例）。

### 4.3 全局助理（L1）

- **数据源**：`memory.cross_project.*`（已存在）+ `~/.marguerite/user_profile.md` + `usage.db`（频次）+ `behavior` 汇总。
- **职责**：① 新任务冷启动时回答"这个人在别的项目里怎么做事"；② 跨项目事实召回；③ 维护长期目标清单。
- **禁止**：读原始会话正文做"性格分析"；把 A 项目的代码片段带进 B 项目。

### 4.4 项目助理（L2）

- **数据源**：workspace 的 `AGENTS.md`、`kb/project_graph`、`context/compact_state`、`plan/`、`goal/`。
- **职责**：① 需求拆解成 `TaskNode` DAG；② 并行派发与收口；③ 项目内记忆读写。
- **禁止**：在未读项目规范前就拆任务（`AGENTS.md` 必须先读，且超 32 KB 要截断告警）。

### 4.5 两层怎么"协同而不打架"

| 场景 | L1 做什么 | L2 做什么 |
|---|---|---|
| 新开一个陌生项目 | 给出"这个人惯用什么技术栈、哪些坑踩过" | 读项目规范，产出第一批任务树 |
| 项目内做到一半 | 不介入 | 从 `CompactState` + checkpoint 续跑 |
| 跨项目复用经验 | 提供带出处的结论 | 判断是否适用于本项目，适用才落记忆 |

---

## 5. 路线图（按批次，每批都有机械验收）

### P0 —— 发行链路与冻结项（0.2.19 ~ 0.2.20）

| 批次 | 内容 | 验收 |
|---|---|---|
| P0-1 | 发行链路：updater `pubkey`（已修，启动 panic 消失）、`latest.json` + `.sig`、下载器/安装器版本对齐 | 装完双击能开；`update.check` 返回 200 而不是 panic |
| P0-2 | `sessions.fork/resume/replay`（M2） | `cargo test -p marguerite-core --lib` 新增用例；纯 Rust 部署点分支会话有真实结果 |
| P0-3 | 上下文接续（§3.1 两行） | 见 §3.1 验收 |

### P1 —— 两层助理 MVP（0.2.21）

| 批次 | 内容 |
|---|---|
| P1-1 | `assistant.scope` + `assistant.brief` 四个 RPC（后端先行，前端后接） |
| P1-2 | L1 简报：`cross_project` + `user_profile` 合成，带 `source`/`score` |
| P1-3 | L2 简报：`AGENTS.md` + `project_graph` 摘要 + `CompactState` |
| P1-4 | 前端：设置页"助理"分区（钉住/查看/删除学到的偏好）——**可撤销**是硬要求 |

### P2 —— 记忆与知识库（0.2.22）

| 批次 | 内容 |
|---|---|
| P2-1 | 记忆写入闸门（晋升/遗忘/冲突消解）+ `memory.stats` 三个新计数 |
| P2-2 | 知识库增量索引 + 启动自检重建 |
| P2-3 | 召回可解释：注入内容带来源与分值 |

### P3 —— 智能体与协同（0.3.x）

| 批次 | 内容 |
|---|---|
| P3-1 | 长任务熔断 + 回滚 |
| P3-2 | 辩论防极化 |
| P3-3 | 任务树 DAG + 并行调度 + `TeamBoard` 真实状态 |

**每批的完成定义**：

```bash
bash scripts/verify.sh --clean --all      # 26 项全 PASS，exit 0
python scripts/check-readme-claims.py     # 文档与实测对账
python scripts/check-website-claims.py
```

外加：该批次涉及的用户可见行为必须有单测或 e2e；`[SKIP]` 不算通过。

---

## 6. 红线与不做清单

1. **不重写引擎**：所有新增都挂在现有模块上（`memory/`、`kb/`、`agent/`、`context/`、`orchestration/`）。
2. **不新增第五个前端**：前端只在 `webui/` 维护。
3. **不跨层写数据**：知识库不写偏好，记忆不写代码事实。
4. **不做不可撤销的"学习"**：任何自动学到的偏好，用户必须能在设置里看到并删除。
5. **不用"全绿"当结论**：本机 Windows 全绿不等于 Linux/macOS 能编译 —— 0.2.18 就是这么翻车的（`#[cfg(target_os="linux")]` 的代码在 Windows 上永不编译）。
6. **不引用 `artifacts/`**：该目录不随仓库发布。

---

## 7. 与用户原始诉求的对照

| 诉求 | 落在哪 |
|---|---|
| 智能体架构革命性加强 | §3.4（A-1~A-4） |
| 记忆架构 | §3.2 + §4.3 |
| 知识库架构 | §3.3 |
| 协同架构 | §3.5 |
| 上下文架构 | §3.1 |
| 全局助理跨项目梳理历史 | §4.3（`memory.cross_project.*` 已存在，补 brief 契约） |
| 项目助理精通项目、自动拆解、并行统筹 | §4.4 + §3.5 |
| 两层协同、越用越懂你 | §4.2 `assistant.brief` + §4.5 协同表 |

---

## 8. 实施状态（2026-10-30）

> 只记录**已落盘并跑过测试**的东西；RPC 尚未接线的部分单独标出，不混进「已完成」。

| 批次 | 状态 | 落点 | 证据 |
|---|---|---|---|
| P0-0 CI 转绿 | **完成** | 9 个测试文件平台化（`gateway/{workspace,workspaces,config,market_remote}`、`doctor/router_weights`、`kb/project_graph`、`plugins/mcp_launch`、`sandbox/{whitelist,policy_store}`）+ `.github/workflows/ci.yml` 显式装 numpy | `cargo test -p marguerite-core --lib` 4380 passed / 0 failed / 25 ignored；`clippy --all-targets -D warnings` exit 0 |
| P0-2 会话分支 | **已上线（RPC + 库）** | `core/src/session/fork.rs` + `gateway/assistant.rs` 的 `sessions.fork/resume/replay` | 7 条库测试 + 3 条网关测试 |
| P0-3 上下文接续 | **已上线（注入提示词）** | `context/continuity.rs` + `role_context.rs` 的 `continuity_block` / `assistant_block` | 4 条 continuity + 3 条 assistant_block 测试 |
| P1 两层助理 | **后端 + 设置面板 + 聊天注入** | `assistant/{scope,profile,brief}.rs` + 10 条 RPC + `AssistantPanel.vue` + `role_context::assistant_block` | 5 条 brief + 3 条 scope + 2 条 profile + 13 条网关 + 6 条前端 + 3 条注入 |
| P1.1 L1→L2 投影 | **已落地（RPC 简报 + 聊天注入同一路径）** | `assistant/projection.rs`（`project` 纯函数）；`brief::build_brief` 先投影再进 `compose_brief` 预算；丢弃明细进 `budget.notes` | 8 条投影测试 + 2 条预算备注测试（`cargo test -p marguerite-core --lib assistant::`） |
| P2-1 记忆闸门 | **已接生产写路径** | `memory/promote.rs` + `memory/writeback.rs::remember_fact`（先闸门后落盘） | 4 条闸门 + 4 条写回测试 |
| P2-2 知识库增量 | **库级完成** | `kb/store.rs` `apply_changes`、`kb/graph.rs` `remove_document`、`kb/integrity.rs` `audit_sources` | 4 条测试；`kb::` 50 passed |
| P3-1 长任务熔断 | **已接入运行时** | `agent/tool_cycle.rs`（CircuitBreaker）+ `agent/task_checkpoint.rs`（pause_for_rollback）+ `runtime.rs`/`stream_loop.rs` 收口点 | 3 条熔断单测 + 3 条接线单测 |
| P3-2 辩论防极化 | **库级完成** | `agent/team.rs` `converge_debate`（附和降权 ×0.2、证据链定胜负、保留少数派） | 3 条测试 |
| P4 桌面端双轨（注意力 + 溯源） | **已落地（前端；零新增 RPC）** | `components/ui/AttentionShelf.vue` + `composables/ui/{attentionQueue,motionGrammar}.ts` + `components/shell/{CommandDeck,ProvenanceDrawer}.vue`；设计见 `docs/design/2026-10-31-attention-first-workbench.md`、`docs/design/2026-10-31-memory-provenance.md` | 注意力 33 例 + 命令台 3 例 + 溯源 13 例；`vue-tsc` / motion / i18n 守卫 exit 0 |
| P3-3 任务树 | **库级完成** | `core/src/orchestration/task_state.rs`（就绪集/派发/失败只阻塞下游/并行批） | 5 条测试 |

**下一步（尚未做，别当成已完成）**：

1. **CI 验证**：P0-0 的 9 处平台化只在 Windows 本地跑过；Linux/macOS 由 CI 独立验证（这正是 0.2.18 翻车过的地方）。
2. **`assistant.facts` 的删除入口**：写回闸门（`memory::remember_fact`）已就绪，但用户只能删
   `pinned`，删不掉「自动学到的事实」—— 可撤销性还差这一环。
3. **RPC 计数纪律**：`SUPPORTED_METHODS` 的当前值以 `node webui/scripts/check-rpc-contract.mjs`
   的输出为准（2026-10-31：**470** 条；未剥注释 473）。每加一条都要同步官网三处，
   否则 `check-website-claims.py` 会判红。
