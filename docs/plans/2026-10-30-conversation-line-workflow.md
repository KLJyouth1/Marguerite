# 对话产线（Conversation Line）—— 协作对话的生成生产工程工作流

> 日期：2026-10-30 ｜ 状态：设计与第一批施工 ｜ 作者：主智能体
> 本文件是**契约源**（前端/后端两批施工以本文 JSON 契约为准，不得各自发明字段）。

## 0. 一句话

把「对话」从**一条文字流**改造成**一条有规格、有预演、有取证、有复盘、会把复盘写回先验的生产线**；
把「蜂群 / 专家团 / 子智能体 / 主智能体」从**五挡互斥开关**改造成**可编译、可组合的产线阶段**。

## 1. 第一性原理推导（为什么现在的产品必然漂移）

不是「换个 UI」，是对同一个物理事实重新建模：**一次协作对话 = 一张推理单元的有向图**。
每个单元吃进（上下文 + 简报）、产出（断言 / 工件），产物或者成为另一单元的输入（边），或者交给人。
「蜂群」「专家团」只是这张图的**形状名**，不是四种不同的东西。

### 1.1 5 Why · 第一条链（缺陷 DEF-10「并行规模选 3、实际只跑 1」）

| # | 追问 | 事实依据 |
|---|---|---|
| 1 | 为什么只跑 1 个？ | `dispatch_form` 当年取 `agents.first()`（`core/src/gateway/orchestrate.rs:406-413` 已修，注释仍在） |
| 2 | 为什么后端只取第一个？ | UI 收的是多选，后端契约是单槽语义 |
| 3 | 为什么两侧语义不同？ | `form` 是**枚举挡位**：声明面（`OrchestrationState`，前端）与执行面（`route_meta`，后端）是**两份独立手写的形状描述** |
| 4 | 为什么允许两份描述？ | 产品按「加旋钮」生长：每加一个 UI 旋钮就手写一次后端读取 |
| 5 | **根因** | **没有任何一个对象本身就是"这一轮的形状"**。挡位+旋钮是*描述*（description），不是*规格*（specification）。描述会漂移，规格会被编译 |

推论 → **设计定律一：形状必须是可编译的规格，不是可勾选的开关。**
UI 上任何不可见的旋钮，在这份规格里就不该存在；后端必须**从规格执行**，使「界面说的 ≠ 引擎跑的」不可表示。
同类缺陷（DEF-08 档位两处漂移连拒两次、DEF-09 浮层压住编排卡、泳道刷新即消失）与本轮新查出的
`capabilities` 假参数（`orchestrate.rs:460` `let _ = capabilities;`）都是同一条根因的不同表现。

### 1.2 5 Why · 第二条链（质量：多智能体为什么没产出更好的答案）

| # | 追问 | 事实依据 |
|---|---|---|
| 1 | 为什么协作没有更准？ | 成员输出被拼成一个 `String`（`"[子专家 N] {summary}"`，`orchestration/mod.rs:2591`）交给一次 lead 调用 |
| 2 | 为什么拼接弱？ | 重复、相关性错误、无异议附和都过滤不掉 |
| 3 | 为什么过滤不掉？ | 跃迁总纲已写下反极化判据（`cos>0.85 / Jaccard>0.6 / ROUGE>0.8 → 附和降权 ×0.2，按证据链而非人头收敛`），但**回传路径没有断言结构，判据无处安放** |
| 4 | 为什么没有断言结构？ | run 记录为「展示」而存（`summary ≤ 1200`、`reply ≤ 8000`），不为「验收」而存 |
| 5 | **根因** | **对话被当成文字流（说了什么），不是当成产品（断言了什么、凭据是什么、过没过验收）** |

推论 → **设计定律二：对话的产物是可验收的断言链，不是文字流。**

### 1.3 5 Why · 第三条链（计量室没人去）

`tactics/` 已实现完整的先验数学：`model.rs::Topology{Solo,Star,Panel,Swarm,Relay}`、`catalog()` 7 条战术、
`Prior{Measured,Declared,Unknown}`、`sim.rs::simulate`（确定性 LCG 蒙特卡洛，2000 次）、
`advise.rs::evaluate/premortem`、`replay.rs::replay`（读 `harness.db` 真实历史）——
**但它是纯咨询件**：`tactics.simulate/replay` 没有任何一处被 `Orchestrator` 消费，只在 TacticsView 自成一页。
一条生产线配了计量室却不去，预演与真实永远对不上账。

推论 → **设计定律三：先验必须出现在派单之前（预演），并被复盘之后（回写）。**
闭合环：**规格 → 预演 → 派单 → 取证 → 复盘（5 Why）→ 回写先验**。下一次预演就比这一次准。

## 2. 词汇统一（三份并存的说法归一）

现状三套词汇（同一个物理事实）：

| 出处 | 取值 |
|---|---|
| `core/src/domain/mod.rs:29-41` `AgentMode` | Solo / DualLeader / ExpertTeam / Cluster / Pipeline |
| `core/src/gateway/orchestrate.rs:34` `OrchestrateForm`（= 前端 `OrchestrationForm`） | single / subagent / team / expert / swarm |
| `core/src/tactics/model.rs:24` `Topology` | Solo / Star / Panel / Swarm / Relay |

归一口径：**`Topology` 是数学名（不动，NG1），`LinePattern` 是产品名，二者 1:1**。

| `LinePattern` | `Topology` | 含义 | 宽度语义 |
|---|---|---|---|
| `solo` | Solo | 单一推理单元（主智能体亲自做） | 恒 1 |
| `fanout` | Star | 并联子智能体，各自独立、结果并列 | 角色数 |
| `panel` | Panel | 专家团：定制作名册 + 一次 lead 综合 | 名册数 |
| `swarm` | Swarm | 蜂群：切面 × 规模，无依赖并行启动 | 规模数 |
| `relay` | Relay | 接力：按依赖串行，前者产物是后者输入 | 链长 |

旧挡位不是被删掉，而是**降级为一个阶段的简写**（编译结果，可被前端展示为"配方"）：

| 旧 `form` | 编译成的阶段序列 | 保持既有语义 |
|---|---|---|
| `single` | `[solo]` | 走 `chat.send` |
| `subagent` | `[fanout(roles)]` | **不做 lead 综合**（现状：`orchestrate.rs:422-435` 直接返回 `summaries[]`） |
| `team` | `[panel(合并名册)]` | `team.run` 内含一次综合，综合阶段隐式 |
| `expert` | `[panel(专家名册)]` | 同 `team`，差 `expertPick` 与标签 |
| `swarm` | `[swarm(facets×size)]` | 后台跑 + 心跳 |

**组合是新增能力**：阶段序列长度 ≤ `MAX_STAGES`，模式可重复 ⇒ 5 + 5² + 5³ = **155 种形状**。
用户要的「各种组合」由此可表示，例如旗舰配方：

```
[swarm(6) → panel(法务+安全+架构) → solo(lead 定稿)]
  蜂群发散   专家团对抗评审         主智能体签字
```

## 3. 规格对象（唯一事实源）

```jsonc
// LineSpec —— 前端可编辑、后端可编译、run 可回溯，三者同一个对象
{
  "specId": "ls-<ms>-<uuid8>",
  "goal": "这轮对话要解决什么",
  "acceptance": ["可判真的验收条款，逐条"],         // 设计定律二的落点
  "budget": { "tokens": 120000, "seconds": 600, "usd": 2.5 },
  "stages": [
    {
      "id": "s1",
      "pattern": "swarm",
      "width": 6,                                   // fanout/swarm：成员数；relay：链长；panel：名册数
      "roster": ["法务合规专家团"],                  // panel/relay：角色或团队 id；swarm：切面
      "facets": ["事实核查","风险反例"],             // 仅 swarm，缺省取 SWARM_FACETS 前 width 个
      "synthesize": false,                          // 本阶段末尾是否做一次 lead 综合
      "note": "为什么要这一段"                       // 人写的理由，进 run 记录供复盘
    }
  ],
  "criteria": { "maxFailProbability": 0.25, "budgetTokens": 120000, "deadlineMs": 600000 }
}
```

后端硬约束（`validate`，全部来自**已存在**的真实限制，不是新造的）：

| 约束 | 值 | 依据 |
|---|---|---|
| `MAX_STAGES` | 3 | 深度 × 嵌套：子智能体**嵌套深度=1 不可行**（`execute_subagent_task_spec:1065` 不接 `SubAgentManager`），阶段串行是当前唯一可靠的深度 |
| `width` 上限 | 16 | `swarm_role` 16 静态角色 + 编号切面（`orchestration/mod.rs:3071`） |
| 并发告警 | >5 并发成员 ⇒ 警告（不拒绝） | `budget_policy::subagent_max_concurrent()` 默认 5，`Semaphore` 排队（`subagent.rs:213,218`）——排队是真实吞吐瓶颈，预演必须说出来，否则重演 DEF-10 |
| `roster` 非空 | panel/relay 必须给名册 | 否则静默降级 |
| 自主档单调 | 阶段自主档取 max 后一次性授权 | 不变量「工具面单调扩展」（`docs/agents/03-invariants.md`） |
| 权限门 | 任一阶段 ∈ {fanout,panel,swarm,relay} ⇒ `HighAutonomy` 口径 | 复用 `gate_form`，不改矩阵 |

## 4. RPC 契约（4 个新方法 + 1 个复盘方法）

注册三处（缺一即契约测试红）：`gateway/protocol.rs:21 SUPPORTED_METHODS`、`gateway/methods.rs` 分发臂、
`methods.rs:3107-3171` 守卫测试。全部**原生 Rust**，不进 `PY_FIRST_METHODS`，不新增前端 `supportsMethod` 字面量。

| 方法 | 入参 | 出参 | 语义 |
|---|---|---|---|
| `line.catalog` | `{}` | `{patterns[], recipes[], roles[], teams[], facets[], limits{maxStages,maxWidth,subagentMaxConcurrent}}` | 展示层唯一事实源，替掉前端硬编码常量 |
| `line.compile` | `{goal, recipeId? \| stages[], overrides?}` | `{spec, errors[], warnings[]}` | 挡位/配方 → 规格；**只读、零 LLM**（离线不变量） |
| `line.preflight` | `{spec, criteria?}` | `{stages[{pattern,width,tactic,p50Ms,p90Ms,p50Tokens,p90Tokens,p90CostUsd,successRate,failProbability}], total{…}, rationale, premortem[], unknowns[], gate{autonomy,decision,reason}}` | 预演；**必须复用 `tactics::plan(scenario, criteria)`**（它已经是「目录→逐条推演→判据裁决→事前验尸」一条龙），不得自己重写 simulate/evaluate/premortem 的编排 |
| `line.plan` | `{spec}` | `{stages[{index,pattern,backend,args,roster,autonomy}]}` | 干跑；**复用 `route_meta`** 保证「测试覆盖的就是执行的」 |
| `line.run` | `{spec, message, sessionKey, mode, confirmed}` | `{runId, stages[{index,status,reply,claims[]}], verdict{ok,failures[],p90MsActual,p90TokensActual}}` | 顺序执行阶段（复用 `dispatch_form`），阶段间只**增量**追加上一阶段产物（红线#1） |
| `line.retro` | `{runId, priorWrite?{pattern,failureRate,samples}, humanWhy?}` | `{ok, whyChain[{answer,kind}], deltas[], gaps[], prior{source,value,samples}, nextPreflightHint}` | 复盘回写；**必须复用 `tactics::replay`**（`replay.rs:245 why_chain` 已有「5 Why 只问有哪些环节有证据」的实现，`WhyKind{Evidence,Gap,Inference}` 已导出），本方法只做 `LineRun → ObservedRun` 投影 + 可选先验回写 |

### 4.1 预演的两条语义（别把它们混成一条）

`tactics::plan` 的既有语义是「给一个任务 DAG，哪种**协作战术**最好」。产线的语义多一层：**人已经选好了形状**，
所以 `line.preflight` 必须同时回答两个问题，缺一不可：

1. **逐阶段**：把该阶段的宽度/名册折算成 `Scenario`（`work_k` 缺省 40，同 `model.rs:384`），跑 `plan()`，
   取「与该阶段 `Topology` 匹配」那条战术的结果作为**该形状**的成本/耗时/成功率（不是取冠军）。
2. **对照**：再跑一次整条 spec 折算出的 DAG，给出「你选的形状 vs 推荐形状」的差异行 ——
   这是产品此前完全没有的能力（形状选了以后无从知道选得好不好）。

汇总口径：阶段**串行**，所以 `total` 的耗时与 token 是**相加**，成功率是**相乘**；
绝不取平均（平均会把「三段各 90%」谎报成「整体 90%」，实际是 73%）。

诚实约束（沿用 `tactics_rpc.rs:10-16`）：`unknowns` 非空时前端**必须**显示「这些数字没有实测支撑」；
参数非法一律 `INVALID_REQUEST` 且校验在任何计算之前。

`claims[]`（设计定律二的落点，一条断言而非一段文字）：

```jsonc
{ "claim": "该方案在 Windows 上无法直接复用 POSIX 信号量",
  "kind": "finding|decision|risk|question",
  "evidence": ["activity/team-<s>.json#member-3", "harness.db#run-42"],
  "by": { "stage": "s1", "member": "蜂群 3/6 · 风险反例" },
  "dissent": true,                    // 与同阶段多数结论的余弦 > 0.85 ⇒ 反极化保留
  "accepted": null }                  // lead 综合后回填
```

## 5. 落盘（必须带生即有界，别把 W4 的债再欠一遍）

`<data>/line/<runId>.json`：`{spec, verdict, claims[], retro?}`，原子写（`.tmp` + rename，同 `member_registry` 口径），
`RUN_KEEP = 100` 终态裁剪，目录字节预算 `LINE_DIR_BUDGET_BYTES = 32 MiB`，
并计入 `observability::io_report` 的 `line` 段（W7 的口径，面板自动可见）。

## 6. 对话页面与全部窗体的再设计

现有形状：输入框 3 个浮层（`ChatSwarmPopover` / `ChatTeamPicker` / `ChatAgentPicker`）+ 能力面板 +
`ConfirmModal`（z1110）+ `OrchestrationRunCard`。它们各自描述形状的一次，正是 1.1 的根因。

新形状（五件，逐件给出「创新在哪」和「为什么不能沿用旧的」）：

| 组件 | 位置 | 创新点 | 替代了什么 |
|---|---|---|---|
| **形状轨 `ShapeRail.vue`** | 输入框上方一条轨 | 形状**画出来**：阶段=节点，宽度=节点上的滑块，名册=节点上的芯片。**看不见的旋钮就不属于规格**，与旧浮层「设置了但没生效」互斥 | 3 个浮层 + 挡位下拉 |
| **预演卡 `PreflightCard.vue`** | **对话流内的一次回合**（不是弹窗） | 人批准的是**一个预测**：p50/p90 成本/耗时/成功率 + 逐条 kill-check + 推荐替代形状 + premortem 风险清单 + 权限门原文 | `ConfirmModal`（本仓已明文反对"Q&A 以弹窗为主形态"，确认疲劳） |
| **产线卡 `LineRunCard.vue`** | 对话流内 | 阶段泳道 + **断言链** + 一条「预测 vs 实测」差异条（首次让产品能说「预演 62k token，实测 91k」）+ 异议徽标 | `OrchestrationRunCard` 的文字流 |
| **复盘梯 `FiveWhyCard.vue`** | 终态失败的产线卡内 | 每一层的原因**从本轮真实证据里抽**（`tactics::replay` 的 `whyChain` 已按 `Evidence/Gap/Inference` 分类，凑不满五层就少给几层，绝不拿推测填满），选定后可一键 `line.retro` 回写先验 | 没有——产品此前没有复盘这件事 |
| **名册编辑器 / 判据编辑器**（`RosterSheet` `CriteriaSheet`） | `ContentSheet`（z300，抽屉，非模态） | 编辑器的校验**就是**后端的校验（同一份 `limits` 来自 `line.catalog`），杜绝两处漂移 | 分散在浮层里的常量 |

排版与设计的硬约束（每个新窗体都要过，门禁会跑）：
只用 `--sp-* / --fs-* / --radius-* / --dur-* / --ease-*` 与 L1 主题角色，禁原始 hex/rgb/hsl、禁裸 ms/`ease`；
≤2 强调色（蓝 `#1A56DB` / 橙 `#F97316`）；禁 emoji 图标；对比度 4 条基线；
z-index 只落在既有分层里（轨 0 层、卡 0 层、抽屉 z300）；`meta.group` 必填否则导航不可见；
新页面不得进 `composables/chat/*` 的 live-turn 围栏（`check-architecture.mjs`）。

## 7. 批次与验收

| 批 | 内容 | 门禁口径 |
|---|---|---|
| B1 后端 `core/src/line/` | 类型 + `compile`/`validate`/`estimate`/`plan`（纯函数、零 LLM、离线） | 155 形状可编译；非法形状给出**原因**；>5 并发出警告；与 `route_meta` 对账 |
| B2 后端网关 | 6 个 RPC + `line.run` 阶段执行 + 有界落盘 | 注册三处齐全；断言链落盘；复盘回写 `Prior` |
| B3 前端契约与轨 | `composables/line/` + `ShapeRail` + i18n `line.*` × 6 | 契约测试镜像 §3/§4；`check-rpc-contract` 绿 |
| B4 前端产线 | `PreflightCard` + `LineRunCard` + `FiveWhyCard` + 两个 sheet | 真机可见；无模态滥用；令牌/对比度/动效守卫绿 |
| 收口 | `bash scripts/verify.sh --clean --all` | 每条 `[PASS]` + exit 0；`[SKIP]` 不算通过 |

### 7.1 挂载缝（B4 现场实测，别猜）

- `ChatComposer.vue` 的编排入口是一条**props + emit** 缝：`props.orchestration?: OrchestrationState`（:559）
  + `emit('updateOrchestration', patch)`（:880），四个锚点分别是
  `ChatAgentPicker`（:232）/ `ChatTeamPicker`（:255）/ `ChatSwarmPopover`（:276）/ `ChatModeCapabilityPanel`（:298），
  全部包在 `v-if="props.orchestration"` 里，且各自带 `data-testid="chat-orchestration-btn-*"`。
  ⇒ 挂 `ShapeRail` 只需再加一条可选 prop + 一个 emit，**不需要重写 5180 行的 ChatView**；
  旧四锚点先共存（`form` 走旧缝、`spec` 走新缝），迁移期用 `fromOrchestrationState` 保证两边说的同一件事。
- e2e 侧已有 `data-testid` 依赖：删旧锚点会打掉 Playwright 用例，所以本批**只加不减**。
- 编排进度帧的面：`session.event.progress`（`kind:"orchestration"`）+ `orchestration.status` 对账；
  `LineRunCard` 消费同一入口，不新增事件名（新增事件名必须先补进 `types/rpc.ts:1005 RpcEventMap`，
  那里已经落后于真实订阅面 —— 已知缺陷，本批不扩大它）。

**不在本批**（NG，沿用既有非目标）：不重写编排 DAG 算法、不做子智能体嵌套（深度 1 是硬事实）、
不新增第五个前端、不把 `capabilities` 假参数洗白（它仍是待办，本设计只是**不再新增**同类漂移）。

### 7.2 落地实测（B4 现场，逐条回冲 §7.1 的预测）

§7.1 是动工**前**读到的形状，保留原文不改；只有一条预测被现实推翻，且推翻在**口径**上：

- §7.1 说「`LineRunCard` 消费同一入口（进度帧）」。实际**不是**：`line.run` 同步返回整份回执
  （`ASYNC_ONLY_METHODS` 只改调度方式，不改返回契约），产线卡吃的是**响应体**，帧只是给
  `member_registry` / 实时看板用的。于是后端复用 `kind:"orchestration"` 时必须同时钉一个
  `engine:"line"`（`core/src/gateway/line.rs::publish_frame`），前端在
  `composables/chat/useOrchestrationRuns.ts::handleFrame` 的**入口**按引擎排除。
  不排除的后果是**双重呈现 + 标签说谎**：一条 panel 产线会一边长出「表单=swarm」的编排泳道卡
  （`form` 缺省兜底就是 `swarm`），一边长出它自己的回执卡。
  `useTeamBoard` 侧**故意不过滤** —— 同一批帧也喂进后端 `member_registry::observe`，
  只滤事件流会让实时看板比它的快照还空（那才是说谎）。仍然没有新增事件名。
- 对话台账这条是「完整」而不是「装饰」：挂线上膛后发送缝**绕开了** `chat.send`，所以 `line.run`
  自己落两轮 —— 入口 `record_turn_persisted(user, goal)`（与 `orchestrate.run` 同口径），
  终态帧之后 `record_turn_persisted(assistant, 末阶段正文)`；半路失败如实写「产线未跑完：<阶段>：<错误>」，
  不把上一段的产物冒充结论。少了 assistant 轮的版本等于**重载会话只剩问题没有答案**
  （回执卡只活在当前会话的内存里，切走即清，不能当历史用）。

「停止」对产线是真的（`line_run` 登记进与编排同一张取消表：`chat.abort` → `cancel::cancel_session(session)`
按会话命中）。取消是**阶段边界**粒度：正在跑的那一段没有中断点（阶段内部是同步的子智能体/会诊调用），
所以回执如实写「已在阶段边界停止（用户取消）」，不假装把那段掐断了。
守卫以源码断言固化在 `gateway::line::tests::line_run_records_both_turns_into_the_transcript`
（`record_turn_persisted` 会写真实会话库，这条不变量在单测里跑不了，只能钉在源码上——同 `chat.rs` 的
`persist_session` 守卫口径）。

真实挂载缝（`ChatView.vue` 5422 行，一行未重写；全部是加标签 + 加状态块）：

| 位置 | 内容 |
|---|---|
| `views/ChatView.vue:913` | 导入 `LineDock` / `LineRunPanel` / `useLineWorkflow` / `lineCodeText` |
| `:3745` | `const line = useLineWorkflow(sessionKey, gearKey)` —— 形状、闸门、回执、复盘**只有这一份** |
| `:636` | `<LineDock>` 挂输入框上方；props 传 `line` + 旧挡位（`gearKey`/`form`/knobs），**无 emit**（dock 写的就是同一份状态） |
| `:259` | `<LineRunPanel v-if="lineRunVisible">` 在消息流末尾、编排泳道之后（同一条理由：执行属于这一轮对话） |
| `:3199` | 发送缝：`line.armed` **优先于**旧五挡；正在跑则拒绝重发；跑不成则**先说原因**再回落普通对话 |
| `:2504` | `lineRunningCount` 并入 `sessionHasActiveWork`，且**按归属会话**计数：在 A 会话跑线不该锁住 B |
| 回执归属 | `lineReceiptKey` + 草稿重绑豁免一次 + `discardForeignLineReceipt`：切走后到达的回执不摆进这一段对话，也不清这一段的输入框 |

### 7.3 与 §6 设计的偏差（如实记录，不改口径假装一致）

| 偏差 | §6 的设想 | 实际落地 | 是代价还是取舍 |
|---|---|---|---|
| 预演卡的位置 | **对话流内的一次回合**（人批准的是一个预测） | 住在 `LineComposer` 里、由停靠条展开 | **代价**：形状每改一次宽度就重算一次预演，放进对话流会把中间态钉成历史；但批准后预演卡随收起消失，可对账的只剩产线卡。要迁移得先定义「预演轮」的持久化与去重，本批不假装已经做了 |
| 形状轨 | 输入框上方一条常驻轨 | `LineDock` 收起态一行（形状+闸门+上膛），展开即装配台 | 取舍：常驻整条轨会把聊天首屏顶走；挂线状态必须常驻，细节按需展开 |
| 旧五挡 | 迁移期共存 | 共存，且删除为零（e2e 依赖 `data-testid`） | 对账**单向**：挡位→线有 `armNextFromLegacy`，线→挡位没有。反向不做的理由是同一轮只能有一个裁决入口 |
| `line.plan` | 六条 RPC 都有消费者 | **本批补上**：`useLineWorkflow.plan` 与预演同批取（共用 `preflightSeq` 守卫，晚到的响应不能盖过新形状），`PreflightCard` 的「执行映射」段逐阶段显示 `backend / 名册 / autonomy / note`；`store.planAvailable` 为 false 时**只有这一段消失**，预演表照常站着 | 取舍：先验算与后端同源（`route_meta`），界面上"声明的形状"与"真正会被调的东西"第一次是同一份；`line.plan` 缺席不该把主表带下水 |
| 复盘回写 `Prior` | §7 B2 的验收口径写着「复盘回写 `Prior`」 | **未落地**：`line.retro` 只读（投影真实运行 → `tactics::replay` 出 5-Why 链），`tactics::catalog()` 仍是厂定先验，预演数字不随本机实测移动 | 有意**不做半截**：回写要让 `line::preflight` 覆盖 `Prior`，界面上就必须同时标出这条数字的来源从「厂定」变成「本机 n=X 实测」；只接后端不接来源标注，等于让用户看到两次预演对不上却说不出为什么——正是本设计要消灭的口径漂移。下一批做「覆盖 + `PriorSource` 下发 + 预演卡来源徽标」三件一起 |

### 7.4 两处「没有任何用户动作能满足」的死结（真机才撞得出来）

B4 真机走查连着撞上两个同类缺陷：**判据本身不可满足**，界面只给得出「永远灰的按钮」或「点了却没回执」。
单测都测不到——夹具是手填的，权限矩阵只在库级断言，两边都天然合法。

| 死结 | 判据在哪 | 为什么无解 | 修法 | 守卫 |
|---|---|---|---|---|
| 产线没有目标就发不出去 | `line/mod.rs:400` `goal 不可为空：产线没有目标就退化成文字流` | `spec.goal` 只在**派单成功后**写回，而对话输入框那句话此前**根本不进校验** ⇒ `canRun` 恒 false | `useLineWorkflow.ts` 的 `draftGoal`（:163，永不落盘）+ `specWithDraft`（:164）：`validateLocal`/`blockingErrors`/`refreshPreflight` 统一改读「形状 + 本轮草稿」的合成视图，派出后写回 `spec`（:412，输入框清空时回落已存形状，预演数字不归零） | `useLineWorkflow.test.ts` 的草稿组用例 |
| 确认弹窗是安慰剂 | `orchestrate.rs::gate_form` → `PermissionMatrix::check` | 矩阵的 `Plan × Mutating`、`MinimalDoIt × Mutating` **无视 `trusted`** 恒回 `Confirm`；阶段→形态映射把 `solo` 判成 `SubAgent` = `Mutating` ⇒ **默认档 `standard_plan` 下任何产线永远发不出去**，`confirmed=true` 也拒 | 钥匙落在 **RPC 边界**（`gate_form` 新增 `Confirm { .. } if confirmed => Ok(())`）而不是改矩阵：矩阵的 `trusted` 是 `guard::trust` 的会话记忆，动它会把工具派发面一起放宽。`Deny` 格**不吃**这把钥匙（档位纪律不由一次确认改写） | `gate_form_confirmation_is_a_key_not_a_placebo` + `line.rs::confirmation_opens_the_confirm_cells_but_never_a_deny`（后者还断言理由必须点名「阶段 s1」） |

另有一条同批修掉的**握手竞态**：`stores/rpc.ts` 在 `_state` 里把 `state` 置 `connected` 时先清 `methods=[]`，
方法表要等**随后一帧 hello** 的 `features.methods`；`catalog.ts` 的 `available` 是 `computed(() => rpc.supportsMethod(...))`，
setup 期那次 `store.load()` 扑空后**没有响应式依赖会再叫它一次** ⇒ 同一条产线刷新两次，一次目录齐全、一次整套空掉
还挂着「目录不可用」，而 `line.preflight`/`line.run` 照常工作（局部可用掩盖整块缺失）。
✅ `useLineWorkflow.ts:527` 补 `watch(catalogAvailable, …)` 重载 + 补预演；测这条要求 rpc mock 的
`supportsMethod` **返回真 `ref`**（返回 `vi.fn()` 常量会让 computed 缓存成常量，用例恒绿而什么都没测）——
本轮把 `useLineWorkflow.test.ts` 的 mock 改成共享响应式源，并给 `effectScope` + `afterEach` 停作用域，
使「方法表晚到」第一次成为可测行为（新增 `hello 帧晚到…` 用例）。

### 7.5 真机验收证据（2026-10-31，非截图自述）

网关：`MARGUERITE_DATA_DIR=E:/temp/marguerite-line-verify` + `./target/debug/marguerite-core.exe gateway --port 18797`，
浏览器 `chrome-devtools` 真点。链路走完：**芯片挂载 → 上膛 → 预演数字出现 → 确认弹窗 → 真实执行 → 回执 → 5-Why 复盘 → 刷新后仍在**。

- 回执逐字（关键处）：`产线跑完 … line-ac2311c6-3534-446b-9422-704e589b32ea … 1 solo×1 成功 SubAgentManager.spawn_with_spec 4m 10s …
  耗时（预演 p90）11m 00s … 实测与预演偏差 -62% … 运行记录：E:/temp/marguerite-line-verify\line\line-ac2311c6-….json（7014 B）5-Why 复盘`
- 无 toast、`lastRunError` 为空（修前这里就是那句闸门理由）；「实测与预演偏差」如实给出 −62%，
  即**预演是量级而非承诺**——这一条正是 §4.1 想要的诚实口径。
- 刷新后两段对话气泡（产线目标 + 回执）都在，说明落盘与投影双向对齐。
- ⚠️ 探针隔离不彻底（已记 §4 坑）：那次只设了 `MARGUERITE_DATA_DIR`，产线记录进了临时目录，
  但 `usage.db` 属 `config_dir()` 侧，真实 `~/.marguerite/usage.db` mtime 同步变 10-31 10:53。
  手工起网关两个 env 都要带。

同批离线证据：`npx vitest run src/composables/line src/components/chat/line` → **4 files / 51 tests passed**
（PreflightCard 4、useLineWorkflow 26、LineDock 15、LineRunPanel 6）；`npm run typecheck` → exit 0
（`check-rpc-contract` 470 条声明全有实现、硬编码 CJK 380 基线无增长、i18n 六语齐全）；
`cargo test -p marguerite-core --lib -- gate_form confirmation_opens` → **5 passed / 0 failed**。
统一门禁 `bash scripts/verify.sh --clean --all` 仍按 §7 收口口径另行取新鲜证据。
