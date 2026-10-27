# Agent 工作台竞品与开源实践研究（2026-10-24）

> 目的：为「玛格丽特」桌面端的工作台、远程执行、预算控制与任务质量门禁提供可证伪的外部参照。
> 方法：官方文档 / 变更日志 / 论文优先（P0–P1），社区文章仅作补充（P2）。每条关键结论标注来源。
> 局限：本文只回答「别人怎么做、为什么」，不代替本项目的产品决策（见 `docs/product/2026-10-24-agent-workbench-prd.md`）。

---

## 1. 研究问题

1. Agent-first IDE 的信息架构长什么样？工作区、对话、编辑器、终端、文件树如何共存？
2. 头部产品如何限制「轮数 / 超时 / 重试 / 花费」？哪些是硬限制，哪些交给模型或预算？
3. 任务「做完没有」由谁判定？如何避免模型自述成功？
4. 远程执行的产品形态是什么？为什么不是「装一个 VPN」？
5. 终端与编辑器的技术选型边界在哪？

---

## 2. 竞品与开源实践

### 2.1 Cursor 3：Agent 优先，但保留完整开发环境

Cursor 3（2026-04-02 变更日志）把界面改成以 Agent 为中心，同时保留 IDE 深度：

- **Agents Window**：并行运行多个 agent，覆盖本地、worktree、云端与 **remote SSH** 四种环境；Agent 标签可并排/网格查看。
- **隔离与对比**：`/worktree` 为任务创建独立 git worktree；`/best-of-n` 用多个模型并行跑同一任务，各自隔离再比较结果。
- **可观测性修补**：新增「滚动到底部」按钮、todo 卡片完成后不再消失、流式期间思考块可展开/折叠、大文件 diff 渲染更快更省内存。
- **MCP Apps 支持结构化内容**，工具输出不再只有文本。

来源：[Cursor Changelog 2026-04-02](https://cursor.com/changelog/04-02-26)、[Cursor 3 发布文](https://cursor.com/en-US/blog/cursor-3)

**对我们的启示**

- 「对话」与「工作台」不是二选一：Agent 面板与编辑器/终端必须能**同时可见**，且互不遮挡（我们本轮真机测出的右侧栏遮挡工作台标题栏，正是反例）。
- 「隔离」是质量手段：worktree / best-of-n 让并行任务互不污染，也让结果可比较。
- 可观测性修补都是小交互，但直接决定「能不能信任这轮结果」。

### 2.2 Claude Agent SDK：轮次、预算与压缩的职责分离

Claude Agent SDK 的 agent loop 文档给出了清晰的职责划分：

- **轮次**：`max_turns` / `maxTurns` 统计「含工具调用的轮次」，**默认不设上限**；用 `max_budget_usd` 按花费封顶。触顶时返回 `error_max_turns` / `error_max_budget_usd`。
- **预算语义**：预算在执行后按 token usage 结算，因此允许**超出上限一个响应的成本**，官方建议预留 headroom；子代理花费计入总预算。
- **推理强度**：`effort`（low/medium/high）独立于轮数，控制每轮推理 token。
- **上下文**：接近窗口上限时自动压缩，流里发出 `compact_boundary`；只读工具可并行，写工具串行。
- **钩子**：`hooks` 可在工具执行前拦截/修改/阻断，是策略层而非硬编码在循环里。

来源：[Claude Agent SDK · How the agent loop works](https://code.claude.com/docs/en/agent-sdk/agent-loop)、[Compaction at a token threshold](https://platform.claude.com/docs/en/build-with-claude/compaction-threshold)

**对我们的启示**

- 限制应当是**多维预算**（轮次 + 花费 + 上下文 + 推理强度），而不是单一「最大轮数」。
- 「默认不设上限 + 可配置预算 + 触顶原因可读」比「默认 8 轮」更符合长任务。
- 压缩是运行时事件，要能在 UI 里被看见（我们已有 compaction 进度，但缺少「为什么压缩、压缩了什么」的摘要）。

### 2.3 OpenHands：用独立裁判判定「目标完成」

OpenHands 的 `/goal` 不是「模型说完成就完成」：

- 每轮结束后由**独立 judge LLM** 审计 transcript 中的**权威证据**（文件内容、命令输出、测试结果），不满足就带着反馈继续跑。
- `max_iterations` 默认 **10**，触顶返回 `status="capped"`；经典运行时 `MAX_ITERATIONS` 默认 100；任务还可设 `max_budget_per_task`。
- 适合「可验证目标」：测试通过、CLI 可运行、迁移可发布。

来源：[OpenHands · Goal Completion Loop](https://docs.openhands.dev/sdk/guides/convo-goal)、[OpenHands · Conversation Goals](https://docs.openhands.dev/sdk/guides/agent-server/conversation-goals)、[Environment Variables](https://docs.openhands.dev/openhands/usage/environment-variables)

**对我们的启示**

- 我们已有 `governance/verifier.rs`（LLM + 校验命令）与 `goal` 轮次护栏，但**没有接到聊天完成路径**，也没有「未通过 → 带反馈重试」的闭环。
- judge 必须看**可执行证据**，而不是只看模型自述；这与我们的 `CompletionGate` 方向一致，但门禁目前只在个别路径被调用。

### 2.4 预算感知与自适应停止（研究侧）

- **BATS**：运行时持续追踪「已用 / 剩余预算」，预算感知会改变规划、工具使用与验证行为。
- **BAGEN**：agent 是否 budget-aware 本身可测；「预测不可行则提前停止」能省 token，但要付「错误中止」的代价。
- **Turn-Adaptive Budgeting**：把多轮推理视为序列算力分配问题，按轮次难度分配思考预算，比固定预算更省。
- 保底结论：**预算要显式、持续可见、可被模型读取**，而不是隐藏在代码常量里。

来源：[Budget-Aware Tool-Use Enables Effective Agent Scaling](https://arxiv.org/html/2511.17006v1/)、[BAGEN: Are LLM Agents Budget-Aware?](https://arxiv.org/html/2606.00198)、[Adaptive Thinking Budgets for Multi-Turn Reasoning](https://arxiv.org/pdf/2604.05164v3)

### 2.5 任务质量与验收：证据优先，裁判其次

- **SWE-bench Verified** 是人工清洗过的子集（原始集约 8–12% 实例被标为有歧义/不可解），前沿实验室普遍只引用 Verified。
- **Agentic Rubrics** 把评测拆成功能正确性、视觉质量、布局、UX 等维度，并可用「交互式裁判」在真实浏览器里操作产物；在 SWE-Bench Verified 上取得 54.2%（BEST@K 协议）。
- 但 **LLM-as-judge 并不可靠**：RuVerBench 专门测「裁判能否可靠验证 rubric」，说明裁判本身需要被校准；也有工程文章指出流行 verifier 会「自信地撒谎」。
- Inspect Evals 的 SWE-bench 评分器明确区分「模型补丁失败」与「基础设施失败」，而不是从脚本退出码推断成功。

来源：[SWE-bench in 2026](https://www.callsphere.ai/blog/swe-bench-evaluating-agentic-coding-agents)、[Agentic Rubrics as Contextual Verifiers](https://arxiv.org/pdf/2601.04171v1)、[Can LLM-as-a-Judge Reliably Verify Rubrics?](https://arxiv.org/html/2606.29920v2)、[Inspect Evals · SWE-bench Verified](https://ukgovernmentbeis.github.io/inspect_evals/evals/swe_bench/)、[Your Verifier Is a Confident Liar](https://dev.to/max_quimby/your-verifier-is-a-confident-liar-26pc)

**对我们的启示**

- 验收顺序应是：**可执行证据（测试/构建/运行输出）> 结构证据（diff、产物清单）> LLM 裁判**。裁判只解释证据，不替代证据。
- 必须有「无法验证」状态，而不是把「没跑测试」当成通过。

### 2.6 远程执行：VS Code Remote-SSH 的架构参照

VS Code 的远程开发不是 VPN：

- 本地只保留 UI；连接后会在远端安装 **VS Code Server**，扩展在远端运行，因此补全、导航、调试都是「本地级体验」。
- 端口转发通过 `asExternalUri` 等 API 管理，本地 webview 可安全访问远端 localhost 服务。

来源：[VS Code Remote Development using SSH](https://code.visualstudio.com/docs/remote/ssh)、[Supporting Remote Development](https://code.visualstudio.com/api/advanced-topics/remote-extensions)、[Remote Development FAQ](https://code.visualstudio.com/docs/remote/faq)

**对我们的启示**

- 「工作位置：远程」的合理语义是：**把执行端放到远端**（远端跑我们的 gateway/core），而不是让用户装 Radmin VPN 组网。
- SSH 是**连接手段**，不是产品形态；产品形态应是「选择远端 → 建立连接 → 全功能可用」。
- 需要处理首次连接（安装/校验远端组件）、认证、端口与失败恢复。

### 2.7 编辑器与终端技术边界

- **CodeMirror 6**：核心约 300KB，模块化、按需加载，适合嵌入面板；大文件表现稳定。
- **Monaco**：5–10MB，功能全但重，diff 需要 web worker 配置；适合「完整 IDE」而非嵌入式小面板。
- **xterm.js**：Web 终端事实标准；`FitAddon` 负责把像素尺寸换算成 cols/rows，**必须把 cols/rows 同步给后端 PTY**，并对 resize 做去抖，否则换行错乱；WebGL addon 可提升吞吐。

来源：[CodeMirror vs Monaco](https://agenthicks.com/research/codemirror-vs-monaco-editor-comparison)、[xterm.js · Using addons](https://xtermjs.org/docs/guides/using-addons/)、[xterm.js 高性能终端实践](https://azmx.ai/blog/xterm-js-tutorial-web-terminal-guide)

**对我们的启示**

- 我们的「代码编辑器」目前是 textarea + 行号，不具备语法高亮、选区编辑、diff；若要「IDE 一样」，CodeMirror 6 是更合适的第一阶段选择（体积、嵌入性、可按语言懒加载）。
- 终端要「像终端」，必须有 PTY + cols/rows；纯管道 + 轮询只能做命令回放。

---

## 3. 与本项目的差距对照（有代码证据）

| 能力 | 外部实践 | 我们的现状 | 差距性质 |
|---|---|---|---|
| 工作台布局 | Agent 面板与编辑器/终端可并存且互不遮挡 | 工作台与右侧栏重叠，右侧栏（z-index 95）盖住工作台标题栏（本轮真机复现） | 布局缺陷 |
| 入口可达性 | 入口常驻、面板可随时开关 | 工作台右缘「+」在 `items.length > 0` 时消失；关闭面板后一度找不到入口 | 状态机缺陷 |
| 文件树 | 可展开、双击打开、右键菜单、重命名/删除/定位 | 旧实现是「进入目录/返回」的浏览器式列表，文件不可打开、无右键菜单 | 功能缺失（本轮已补） |
| 编辑器 | 语法高亮、多标签、脏状态、diff | `EditorPanel` 为 textarea + 行号；无高亮/diff | 技术代差 |
| 终端 | PTY + xterm + FitAddon，cols/rows 同步 | `terminal/mod.rs` 自述「行模式 v1」；`sandbox.terminal.*` 为管道 + 轮询，输出每 400ms 覆盖（本轮修了累积与 ANSI） | 架构代差 |
| 远程 | 远端执行端 + 一键连接 | 仅「本地/云端/远程」置灰；SSH 协同存在，但设置页引导安装 Radmin VPN | 产品形态错误（本轮改为远程网关直连，去掉 VPN 引导） |
| 预算 | 轮次 + 花费 + 上下文 + 推理强度，多头预算且可观测 | 已有 `budget_policy` 四档 + 模型声明 + 硬护栏；但 plan runner 仍有 `STEP_TOTAL_TIMEOUT=120`、`STEP_MAX_RETRIES=3`、`max_steps=12` | 覆盖不全 |
| 完成判定 | 独立裁判 + 可执行证据 + 未通过重试 | `CompletionGate` 已改为「证据优先」，`governance/verifier.rs` 已存在但**未接入聊天完成路径** | 接线缺失 |
| 质量可见性 | 产物清单、diff、验证回执、成本 | `harness` 台账、`patch_evidence_ledger`、`final_diff_contract` 已存在，前端未形成统一「交付回执」 | 呈现缺失 |
| 并行与隔离 | worktree / best-of-n | 有 `agent/worktree.rs` 与编排团队，但没有「同一任务多方案对比」入口 | 能力有、产品无 |

---

## 4. 提炼出的设计原则

1. **双栏共存，不互相遮挡**：对话是主视图，工作台是并置工具；任何浮层/面板都必须做视口碰撞与 z-index 预算。
2. **入口幂等**：入口的可见性只由「能力是否可用」决定，不由「是否已有标签」决定。
3. **限制是预算，不是禁令**：默认给足，触顶要能说明原因并可调；硬护栏只用于不可逆风险。
4. **证据优先于自述**：测试/构建/运行输出 > diff/产物 > 裁判解释；无法验证要显式说「未验证」。
5. **远程 = 远端执行端**：SSH 只是传输；产品要负责连接、安装、认证、恢复，绝不让用户先装 VPN。
6. **隔离即质量**：worktree/临时副本让并行与试错不污染主工作区。
7. **压缩要可见**：上下文压缩是运行时事件，必须能在 UI 里看到边界与摘要。
8. **终端要真**：PTY + 尺寸同步 + 流式增量；没有 PTY 就不要假装是终端。
9. **编辑器按需加载**：嵌入式用 CodeMirror 6，重能力再考虑 Monaco。
10. **每个「小交互」都是信任的一部分**：滚动到底、todo 不消失、思考块可展开——这些是竞品专门修的 bug。

---

## 5. 对我们最有价值的 5 个借鉴

1. **Agent 工作台与右侧栏共享宽度模型**（Cursor Agents Window / VS Code 分栏）：用统一 inset 变量而不是各自 fixed。
2. **OpenHands 式「验收官」闭环**：独立 judge + 可执行证据 + 未通过带反馈重试 + 触顶标记 `capped`。
3. **Claude 式多维预算**：轮次/花费/上下文/推理强度分开配置，默认不设死上限，触顶原因可读。
4. **VS Code Remote-SSH 式远程体验**：远端执行端 + 一键连接 + 端口/认证/恢复，而不是 VPN。
5. **xterm + PTY + CodeMirror**：把「像 IDE」落到技术选型，而不是只改样式。

---

## 6. 来源清单

- [Cursor Changelog 2026-04-02](https://cursor.com/changelog/04-02-26)
- [Cursor 3 发布文](https://cursor.com/en-US/blog/cursor-3)
- [Claude Agent SDK · How the agent loop works](https://code.claude.com/docs/en/agent-sdk/agent-loop)
- [Claude · Compaction at a token threshold](https://platform.claude.com/docs/en/build-with-claude/compaction-threshold)
- [OpenHands · Goal Completion Loop](https://docs.openhands.dev/sdk/guides/convo-goal)
- [OpenHands · Environment Variables](https://docs.openhands.dev/openhands/usage/environment-variables)
- [VS Code Remote Development using SSH](https://code.visualstudio.com/docs/remote/ssh)
- [VS Code · Supporting Remote Development](https://code.visualstudio.com/api/advanced-topics/remote-extensions)
- [xterm.js · Using addons](https://xtermjs.org/docs/guides/using-addons/)
- [CodeMirror vs Monaco](https://agenthicks.com/research/codemirror-vs-monaco-editor-comparison)
- [SWE-bench in 2026](https://www.callsphere.ai/blog/swe-bench-evaluating-agentic-coding-agents)
- [Agentic Rubrics as Contextual Verifiers](https://arxiv.org/pdf/2601.04171v1)
- [Can LLM-as-a-Judge Reliably Verify Rubrics?](https://arxiv.org/html/2606.29920v2)
- [Inspect Evals · SWE-bench Verified](https://ukgovernmentbeis.github.io/inspect_evals/evals/swe_bench/)
- [BATS: Budget-Aware Tool-Use](https://arxiv.org/html/2511.17006v1/)
- [BAGEN: Are LLM Agents Budget-Aware?](https://arxiv.org/html/2606.00198)
