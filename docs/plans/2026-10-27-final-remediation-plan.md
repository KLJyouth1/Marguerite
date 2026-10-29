# Marguerite 最终改进计划（Lancer 范式 × Personal Agent 研究 × 既有 PRD 汇流）

> 日期：2026-10-27 ｜ 状态：**执行中**（本文档即施工总纲；每节标注执行状态）
> 输入源：① Lancer 视频范式（文件工作区驱动交接 / 角色看板 / 任务节点串联 / 人保决策权）；
> ② `deliverables/personal-agent-gap-research-2026-10-27.md`（九层对照，P0/P1/P2）；
> ③ `docs/prd/2026-10-27-workspace-session-architecture.md`（会话页架构 P0/P1/P2）；④ `.tmp/task-backlog.md`。

## 0. 核心判断：三条输入指向同一件事

Lancer 的差异化是「**会话只做调度，产物全部落盘共享，人保审核**」。Marguerite 已有的资产恰好是它的地基：
产物台账（artifacts.*）、任务工作台（/task 聚合）、交付流水线 DAG（orchestration 交付流水线）、多智能体编排（agents/team/swarm）、团队看板（team-board）、审批面（approval/escalation）。
**缺的不是能力，是「交接协议」**：模型/会话/角色之间没有成文的"落盘-承接"约定，导致跨模型协作退化为用户手动传文件或长会话堆上下文——这正是用户"盲控电脑/传文件"痛点的协作面根源，与研究报告 P1-4（Connector 阶梯）同根。

## 1. 执行总纲（四条线，按序推进）

### A 线：工作区交接协议（Lancer 核心范式）
| 步骤 | 内容 | 状态（2026-10-27 二次核实） |
|---|---|---|
| A1 | `/handoff` 交接卡：把会话产物清单 + 交接说明写成工作区内 `handoff/<时间戳>.md`，并登记产物台账——任何下一个模型/会话/角色直接读工作区承接，不经对话传文件 | **已实现**：`webui/src/utils/chat/handoff.ts` + `useChatSlashCommands.ts` 的 `handoff.write` |
| A2 | 交接卡的"承接"半边：新会话/切换模型时，检测工作区 `handoff/` 最新交接单，在聊天流提示「发现交接单，一键作为上下文引用」 | **已实现**：`webui/src/components/chat/HandoffNoticeCard.vue`（用既有 `sandbox.path.list` 读 `<workspace>/handoff`，取字典序最新 `.md`，失败静默），挂载 `ChatView.vue:140`，处理器 `ChatView.vue:1749 applyHandoffNotice` / `1756 openHandoffInEditor`，i18n `chat.handoff.notice*` |
| A3 | team-board 角色看板补流转状态列（待办/进行中/待审查/完成），与交付流水线 DAG 的 step 事件对齐；卡片显示产物数 | **已实现**：`webui/src/views/TeamBoardView.vue:54` 的 `kanbanColumns` 泳道（`data-column`），守卫 `TeamBoardView.test.ts:503`「kanban lanes (A3)」 |
| A4 | 人保决策权落进流水线：交付流水线每步完成 → 产物落盘 → **待审查门**（用户确认才流转下一步） | **已实现**：`webui/src/components/task/StepList.vue:5` 审查条 + `composables/task/useStepReview.ts`（纯函数层，含单测） |

> ⚠️ 本表上一版把 A2/A3/A4 全部标成「待做」是**错的**（2026-10-27 二次核实发现三条都已落盘）。
> 后果不是文档难看：按过期状态施工会重复造一遍——本轮就曾为 A2 新增了一条
> `workspace.handoff.latest` RPC，核实后整条撤回。**改这一节前先 grep 上面列出的文件。**

### B 线：Personal Agent 研究 P0（信任与安全，core 侧，需与写者协调窗口）
B1 间接注入防线（spotlighting 包裹 browser/DOM/外部内容）；B2 Credential 迁 Vault；B3 allow-always 参数级绑定+过期。——**本轮未动 core**，等打包窗口/写者确认后单独施工。

### C 线：会话页架构 PRD P0（已立项）
隔离契约测试、清空/归档文案与 UI 入口、产物分类表扩展。与 A 线共享同一批 UI 变更窗口。

### D 线：拥挤感治理推广（上轮已开头的模式复制）
`control-stage--focus` 专注模式推广到 ComputerUse/Fleet/Editor 等工作面视图；49 视图半实现巡检（每批 5-8 个视图，带证据销项）。

## 2. 本轮已执行（A1 交接卡）

- 新增 `webui/src/utils/chat/handoff.ts`：`composeHandoffMarkdown()` 纯函数（交接单组装：会话键/时间/产物表/交接说明/承接指引）。
- `useChatSlashCommands.ts`：注册 `/handoff`（catalog 缺失时前端兜底，同 `/plan` 模式）；执行链 = `artifacts.list` → 组装 → `workspace.file.write`（越界拒绝，写 `handoff/<ts>.md`）→ `artifacts.register`（kind=handoff，进台账与聊天流）→ toast 反馈。
- i18n 六语言 `chat.handoff.*`；新单测 `utils/chat/handoff.test.ts`。
- 验证：typecheck + 定向单测（结果见本轮回复）。

## 3. 排期与验收

| 批次 | 内容 | 验收门禁 |
|---|---|---|
| 第 1 批（本轮） | A1 | typecheck + 定向单测绿 |
| 第 2 批 | A2 承接提示 + C 线 P0 | 同上 + e2e 冒烟 |
| 第 3 批 | B 线 core 三项（与写者协调窗口） | `verify.sh --clean --all` 全绿 |
| 第 4 批 | A3/A4 看板与待审查门 | 同上 + 真机走查 |

## 4. 诚实边界

- Lancer 视频内容为二手转述，其实现细节未核对源码；本文只吸收**范式**（工作区交接/看板/人保决策），不承诺复刻其具体实现。
- B 线涉及 core 与审批面，属并发写者活跃区，未在本轮动。
