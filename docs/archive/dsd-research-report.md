# DSD / DeepSeek Harness 调研报告（面向 marguerite-core 的能力/截图强化）

> 调研日期：2026-08-18
> 调研员：general-purpose-3（web/code research）
> 目标：复核 dsh-desktop（DeepSeek Harness Desktop）的“修复了很多问题、更新了官方 Discussions、验证过的 fork 修复分支”说法，提取可用于 `marguerite-core`（Rust axum 网关 + Tauri GUI + Vue3 webui）的能力与截图（screenshot）实现参考。
> 重要前置结论：**dsh-desktop 本质是一个把 `deepseek-harness` 包成 Electron 桌面壳的社区项目，它本身不含 screenshot / computer_use / RPC 网关的具体实现**。用户提到的“基于现有代码实现截图”在 Marguerite 中**已经领先于 dsh-desktop**（见 §4）。本报告如实区分了“dsh-desktop 实际做了什么”与“上游 deepseek-harness 架构里有哪些可借鉴点”。

---

## 1. 摘要（Summary）

1. **dsh-desktop（dataelement）是桌面封装壳，不是 Agent 内核。** 它的真实修复都是桌面工程问题（Windows 进程启动崩溃、驱动器根目录安装、插件市场安装器运行时、跨版本数据保留），与“截图/RPC/computer_use 内核”无关。PR 清单见 §3.1。
2. **“官方 Discussions + 验证过的 fork 修复分支”这一说法在 dsh-desktop 仓库内无法证实。** 该仓库没有 external fork，所有分支都是自身的（如 `codex/windows-root-install`、`codex/market-installer`）。但上游生态确实存在活跃修复流：deepseek-harness 的 `worktree/fix-*` 分支、社区第三方 fork（tokscale / Clawdmeter / viberank 等 token 计量修复）以及一篇 dev.to 深度文（指出了 token projection 双计数、fork 陷阱等“应最先修的 bug”）。详见 §3.5。
3. **真正可借鉴的架构来自上游 `deepseek-ai/deepseek-harness`**（MIT，TypeScript monorepo）：Typert RPC（白名单式 `@Remote`/`@RemoteScope` + request-id 关联 + 取消）、Agent Inbox 转向（steering）、Turn/Step 模型、append-only 事件溯源会话日志 + “model-visible == logged” 不变量、按平台沙箱 failover（bubblewrap→Landlock / Seatbelt / Restricted Token+ACL）。这些与 `marguerite-core` 的网关、agent 运行时、会话、沙箱高度同构，可直接对照强化。
4. **截图/computer_use 在 Marguerite 中已经实现且更完整：** `core/src/browser/mod.rs`（`screenshot_png` / `a11y_nodes` / `box_center` / `click_at` / `relay` 连真实 Chrome）与 `core/src/tools/computer_use.rs`（三层 OS 原生感知/控制/grounding + VLMM `locate` 闭环）。dsh-desktop 没有提供可抄的截图实现。可“做极致”的方向是：统一截图/感知通道 + 借鉴 dsh-market 的截图隐私门控 + 借鉴 harness 的取消/超时/重试中间件。
5. **最高价值的可落地项**（按优先级）：① 网关 RPC 增加“作用域解析（Agent/Session）+ 一等公民取消（aborted/timedOut/signal 区分）”；② 会话日志强制“任何 model-visible 内容必须落事件”的不变式（防重放不一致）；③ 沙箱按平台 failover 且 `SANDBOX_UNAVAILABLE` 显式上报、不静默降级；④ 截图功能加隐私门控与静默降级（来自 dsh-market）；⑤ 桌面端进程生命周期健壮性（来自 dsh-desktop PR #39/#59）。

---

## 2. 调研对象（Source Repos Inspected）

| 仓库 | URL | 性质 | 本次调研结论 |
|---|---|---|---|
| dsh-desktop | https://github.com/dataelement/dsh-desktop | Electron 桌面壳（封装 deepseek-harness） | 仅桌面工程修复，无内核能力；无 fork 修复分支。最新提交 2026-08-17。 |
| dsh-market | https://github.com/dsh-market/dsh-market | DeepSeek Harness 插件市场 | 含“AppStore 风格截图”隐私门控实现（双层源 + 仅 GitHub 托管 + no-referrer + 懒加载 + E12 防追踪像素 + 静默降级）。无 computer_use/RPC 产品代码（仅 playwright e2e）。 |
| deepseek-harness（上游） | https://github.com/deepseek-ai/deepseek-harness | 官方 Agent 框架（Cordis 插件内核，TypeScript monorepo，MIT） | 提供 Typert RPC、Agent Loop/Inbox、capability seam、append-only 会话日志、按平台沙箱。文档/博客未显示其自带 screenshot/computer_use 工具。 |
| 深度架构文（第三方） | https://www.sshipanoo.com/blog/ai/DeepSeek-Harness架构全解 | 基于 0.1.0-rc.5 源码的六层架构拆解 | RPC/Host-Client、权限审批、沙箱 failover、会话恢复的核心参考。 |
| dev.to token 文（第三方） | https://dev.to/lizhuojunx86/...token-projection... | 社区审计 deepseek-harness 的 token 计量 bug | 指出“每个 usage 写两遍导致 2.00× 双计”“fork 陷阱（父历史被复制导致 23× 虚高）”等应最先修的 bug。代表“社区验证过的 fork 修复”精神。 |

Marguerite 侧已读代码（用于适用性锚定）：
- `core/src/gateway/methods.rs`（方法路由、async/sync 分派、`browser.*` / `computer_use.action`）
- `core/src/gateway/protocol.rs`、`core/src/gateway/server.rs`
- `core/src/browser/mod.rs`、`core/src/browser/cdp.rs`、`core/src/browser/gateway.rs`
- `core/src/tools/computer_use.rs`
- `core/src/security/{exec,os_sandbox,command_guard,preflight}.rs`、`core/src/sandbox/rpc.rs`
- `core/src/agent/{runtime,tool_cycle}.rs`、`core/src/persistence/session.rs`

---

## 3. 具体修复 / 功能发现（Concrete Fixes / Features Found）

> 每条含：**what（是什么）/ where（PR·commit·模块）/ why（为何重要）/ applicability（对 marguerite-core 的适用性）**。

### 3.1 dsh-desktop 的真实修复 PR（桌面工程类，非内核）
这些 PR 是 dsh-desktop 仓库里“修复了很多问题”的真实来源，但都围绕**桌面壳 + 子进程生命周期**，不是截图/RPC 内核。

| PR | 标题 | 修复点 | 对 Marguerite 的适用性 |
|---|---|---|---|
| #39（merged 2026-08-15） | Fix Windows Harness process startup | Windows 下 Harness 子进程启动失败 | **高**：Marguerite 桌面端（gui/ Tauri）启动 `marguerite-core` 也需稳定的子进程拉起 + 失败重试。可借鉴其“启动失败→重试/查看日志/退出”的 UX。 |
| #59（merged 2026-08-17） | Fix intermittent Windows Harness workspace crash | Windows 下 Harness 工作区间歇性崩溃 | **高**：进程崩溃恢复。Marguerite `pyengine/sidecar.rs` 侧车有类似崩溃风险，应加心跳 + 自动重启 + crash 上报（对齐 core-optimization-top10 的健壮性精神）。 |
| #63（merged 2026-08-17） | allow Windows drive-root installation | 允许装在 Windows 盘符根目录 | **低**：纯路径处理，Marguerite 安装器可规避（用 `%LOCALAPPDATA%`）。 |
| #61（merged 2026-08-17） | market-installer: ensure bundled Node/pnpm runtimes used | 插件安装必须走捆绑的 Node/pnpm 运行时 | **中**：Marguerite 的 skills/plugins 远程安装同理——必须用自带运行时，避免宿主环境 Node 版本不一致。对齐 `skills/loader.rs` 的签名校验思路。 |
| #34（merged 2026-08-15） | Preserve desktop data across upgrades | 升级不丢用户数据（数据放在安装目录外） | **高**：Marguerite 桌面端打包应保证 `~/.marguerite` 配置/会话在覆盖安装后保留（当前 `persistence/config.rs` 已读 `$1marguerite/config.yaml`，需在打包层保证不被清掉）。 |
| #45 / #47 | LAN mobile remote / qrcode for lan-mobile-bridge | 局域网手机遥控 + 二维码 | **可选**：Marguerite webui 已可通过 WS 网关远程访问，可加二维码快速配对。 |

### 3.2 Typert RPC：白名单式远程调用 + request-id 关联 + 取消（来自 deepseek-harness）
- **what**：构建期生成 Invocation Descriptor（端点/参数/返回值/编解码）；Host 方法须显式标注 `@Remote` / `@RemoteScope` 才暴露给浏览器；`dsh-api-remotes` 决定 Web 端能访问哪些 Host 能力（白名单，非自动暴露全部 JS 对象）；`Connection` 负责请求关联（request id）、取消、帧、连接生命周期。错误分类把 `aborted` 与 `timedOut` / `signal` 分开。
- **where**：`packages/api`（Typert Gateway + Web BFF）、`dsh-api-remotes`、`Connection`、`packages/typert`（构建期生成）。
- **why**：白名单式暴露天然缩小攻击面；request-id 关联让取消/超时/并发可定位；错误类型细分便于上层区分“用户取消”vs“超时”vs“信号中断”。
- **applicability（对 marguerite-core）**：`core/src/gateway/methods.rs` 已是“前缀代理 + async/sync 分派”的白名单式，但未显式做 **Agent/Session 作用域解析** 与 **取消令牌传播**。`chat.abort` 已有，但建议：① 把 `browser.*` / `computer_use.action` 等 IO 方法的调用绑定到具体 session 作用域（防止跨会话越权）；② 在 `RpcError`（`protocol.rs`）中区分 `aborted` / `timeout` / `cancelled`；③ 参考 harness “tools/execute waterfall 含 timeout/retry/metrics” 的思路，为 `gateway/server.rs` 的 WS 读循环与 `pyengine/client.rs::call` 统一加 `tokio::time::timeout`（core-optimization-top10 ⑩已列入下一阶段，建议提前）。

### 3.3 Agent Inbox 转向 + Turn/Step 模型（来自 deepseek-harness）
- **what**：Agent 收到的消息先进 **Inbox**（输入队列），负责：唤醒空闲 Agent、执行中到达的 follow-up、执行中 **steering**（调整后续方向）、injected context、以及“由驱动器原子认领本步输入（claim）”。**Turn** = 一次从输入到无待办工作的完整回合；**Step** = 一次模型请求 + 其产生的全部工具执行。区分 Turn/Step 让取消/重放/预算更精确。
- **where**：`packages/core`（agent / agent-loop）、`agent/inbox/*` 活事件、Session Event 持久化。
- **why**：steering 让长任务中途改方向不中断；claim 避免并发重复消费；Turn/Step 让“取消”“压缩”“预算”语义清晰。
- **applicability（对 marguerite-core）**：`core/src/agent/runtime.rs` 已有状态机（Init→Ready→Running→Paused→Done→Error）+ `AtomicBool` 中断/恢复；`tool_cycle.rs` 有工具循环 + Semaphore 并行 + 30s 超时。建议补：① Inbox 语义——执行中新消息作为 steering 注入而非排队丢弃；② 显式 Turn/Step 计数（对齐 harness，便于 Trajectory 回放与计费）；③ 执行中 follow-up 的“认领”机制（避免并发工具循环重复消费同一输入）。

### 3.4 append-only 会话日志 + “model-visible == logged” 不变量（来自 deepseek-harness）
- **what**：Session 是仅追加的事件溯源日志（事件含 `turn/start`、`user/message`、`assistant/chunk`、`tool/call`、`tool/result`）。核心不变量：**任何进入模型请求的动态内容都必须能从 Session Log 重建**，否则会出现“页面显示一套历史、恢复后模型看到另一套”。`deriveMessages()` 从日志派生下一次模型请求历史；seed/live history 分界、投影缓存、崩溃恢复识别完整/不完整生命周期区间。
- **where**：`packages/core/session`、`packages/session`（持久化/投影）、`deriveMessages()`、`ctx.systemPrompt`。
- **why**：保证可审计（Trajectory 逐帧回放）、断点续跑一致、压缩不悄悄替换内存消息（Compaction 也写 Session Event）。
- **applicability（对 marguerite-core）**：`core/src/persistence/session.rs` 已有会话快照/恢复。建议强化为 harness 式**强不变式**：① 任何注入模型的动态内容（技能输出、`AGENTS.md`、注入上下文、工具结果）都必须先落事件再进模型，禁止“临时拼文本进 prompt 但不记日志”；② `deriveMessages()` 同源自一处（避免页面与恢复后不一致，这正是 harness 明确点名的坑）；③ 压缩/重放同样走事件流（当前 `context/` 压缩若直接改内存数组需补事件）。

### 3.5 社区“验证过的修复分支”精神（无 dsh-desktop fork，但上游生态活跃）
- **what**：用户说“验证过的 fork 修复分支”，在 dsh-desktop 内**找不到 external fork**（其分支均为自身）。但在更广生态可印证“社区验证修复”的存在：
  - 上游 `deepseek-harness` 的修复分支：`worktree/fix-1463-rich-content-bridge`（PR #2252，桥接 MCP/ACP 富图片内容）、`release/dsh-0.1.0-rc.7`（PR #2620）。
  - 内测组织 `dsh-external` 下 251 个仓库、反馈板 545 条 issue（社区生态已长成）。
  - 第三方 token 计量修复 fork：tokscale、Clawdmeter、viberank、splitrail（dev.to 文作者自述在 4 个 usage tracker 上提过 10 个上游修复）。
  - dev.to 审计文指出的“应最先修的 bug”：① 每个 usage 事件被写两遍导致 token 总数精确 **2.00× 双计**（input/output/cache-read 全中招，cache-read 占某 route 81%）；② **fork 陷阱**：fork 子会话日志复制父历史前缀，`seedLength` 未扣除，导致一个 fork 报告 263,790 tokens 仅 11,442 为自己工作（23.05× 虚高）；③ compaction 事件（48,895 tokens）不被任何 projection 计数。
- **why**：这些是“社区验证 + 可复现”的真实修复点，比 dsh-desktop 的桌面修复更接近 marguerite-core 的**计量/会话/压缩**子系统。
- **applicability（对 marguerite-core）**：① Marguerite 若有 token 计量（`context/budget.rs`、`model/llm.rs` 流式 usage），必须**每个 step 只记一次**且区分 cache 桶，避免 2× 双计；② 子 Agent / 会话 fork（`agent/subagent.rs`）要带 `seedLength` 语义，合并统计时扣除继承前缀；③ compaction 事件必须进入计量 projection。这与 harness 的 append-only 设计互为印证。

### 3.6 沙箱按平台 failover + 显式“不可用”不上报静默降级（来自 deepseek-harness）
- **what**：本地沙箱 `dsh-sandbox-local` 按平台选 runner：macOS=Seatbelt（`sandbox-exec`）；Linux=bubblewrap（`bwrap`），**失败回退 Landlock**；Windows=Restricted Token + NTFS ACL（报告 partial enforcement）。**关键点**：若平台 runner 不可用，返回 `SANDBOX_UNAVAILABLE`，**绝不静默改成无约束执行**；launcher 启动失败与命令返回 127 不同，携带 `runner failure signature`。当前本地 profile 不限制网络出口/CPU/内存。
- **where**：`dsh-sandbox-local`、`dsh-bash-sandbox`、`dsh-subprocess-local`、`native/`（Linux Landlock 原生 runner）。
- **read**：Maguerrite 已有 `core/src/security/{exec,os_sandbox,command_guard}.rs` 与 `core/src/sandbox/rpc.rs`。建议对齐：① 多平台沙箱 failover 链（如 Linux：bwrap→landlock→显式不可用）；② 沙箱不可用时返回明确错误码并阻断，而非放宽到无约束（harness 明确反对静默降级）；③ 权限审批要求模型给出 justification（`ctx.approval`），与 Marguerite 的 `CommandGuard` 默认超时/workspace 约束互补。

### 3.7 dsh-market 的“AppStore 风格截图”隐私门控（双层源 + 静默降级）
- **what**：安装对话框内展示截图，采用**双层数据源**：① 策划层（`data/screenshots.json`，作者维护）；② 回退层（打开对话框时从 repo README 用 `raw.githubusercontent` CORS `*` 提取图片）。**隐私约束**：截图 URL 携带用户 IP，仅允许 GitHub 图片托管 HTTPS，对话框打开后才请求，`img` 带 `no-referrer` + 懒加载；快照校验器加 **E12 检查防追踪像素**；无 README/图片/断链均**静默不渲染**。
- **where**：`dsh-market` 仓库，commit `feat: AppStore-style screenshots in the install dialog (#61) (#88)`；`data/screenshots.json`；隐私校验器 E12。
- **why**：截图可能泄漏用户工作区/隐私，必须控制来源与加载时机，并防追踪像素回传。
- **applicability（对 marguerite-core 截图能力）**：**这是本次调研里与“截图”最相关的可落地参考**。Maguerrite 的 `browser.screenshot` / `computer_use` 截图若要在 webui 展示或上传，应：① 仅允许受信/本地来源；② 展示时 `no-referrer` + 懒加载；③ 加隐私/追踪像素检测；④ 任何加载失败**静默降级**而非报错崩溃。这是对“截图做极致”的直接加固点。

---

## 4. 截图实现参考（Screenshot Implementation References）

**核心事实：dsh-desktop 没有 screenshot 实现；Maguerrite 已有的截图能力比 dsh-desktop 强。** 下面锚定 Marguerite 现有代码，给出“基于现有代码做极致”的具体路径。

### 4.1 Marguerite 已有的截图/屏幕感知实现（无需从零，只需强化）
| 能力 | 文件 / 函数 | 现状 |
|---|---|---|
| CDP 截图 | `core/src/browser/mod.rs::CdpSession::screenshot_png` → `page_capture_screenshot`（`cdp.rs`） | 已用 `Page.captureScreenshot` 返回 PNG，带 30s 超时。 |
| a11y 非视觉感知 | `browser/mod.rs::a11y_nodes`（`Accessibility.getFullAXTree`）+ `box_center`（`DOM.getBoxModel`）+ `click_at`（`Input.dispatchMouseEvent`） | 已实现“截图→a11y 快照→ref 坐标→真实点击”非视觉闭环。 |
| 连真实 Chrome（Relay） | `browser/mod.rs::BrowserBridge::relay`（`GET /json/version` 发现 + `target_create/attach`） | 已复用用户登录态，headless 通道补充。 |
| OS 原生截图 | `core/src/tools/computer_use.rs::build_capture_cmd`（Windows `System.Windows.Forms` / macOS `screencapture` / Linux `scrot`） | 支持主屏/多屏/活动窗口 + `max_edge` 降采样（对齐 OpenCUA 降采样策略）。 |
| VLMM grounding 闭环 | `computer_use.rs::dispatch` 的 `locate` 分支（截图→`VisionCapability::locate`→坐标对齐→可选 `auto_click`） | 已有“截图→VLMM 坐标→执行”闭环 + 低置信度降级。 |
| 网关暴露 | `core/src/gateway/methods.rs`：`browser.open/navigate/evaluate/screenshot/a11y/click/close`、`computer_use.action` 均已在 `ASYNC_ONLY_METHODS` | 已通过 WS 异步 RPC 可达。 |

### 4.2 可借鉴的外部截图/感知范式（用于“做极致”）
1. **统一截图/感知通道**（来自 Marguerite 自身 + harness 的 capability seam 思想）：把 `browser.screenshot`（CDP 视觉）与 `computer_use` 的 OS 原生截图 + a11y 合并为一个语义化 `perceive` 动作，由 `VisionCapability` 选择视觉/非视觉/坐标三种输出，避免工具碎片化（harness 的“consumer 调用 `ctx.tools`”同构）。
2. **截图隐私门控**（来自 dsh-market §3.7）：webui 展示截图时加 `no-referrer`、懒加载、追踪像素检测、静默降级。
3. **截图可靠性**（来自 harness §3.2 的 timeout/retry 中间件 + Marguerite 已有的 30s CDP 超时）：在 `browser/mod.rs::send_command` 已有 30s 超时基础上，为 `screenshot_png` 加失败重试（如首帧空白/页面未 ready 时等待 `wait_page_ready` 后重试一次）。
4. **坐标空间对齐稳健化**（harness 未做，Maguerrite 已做 `box_center` 的 viewport CSS 映射）：保留并补充 devicePixelRatio 处理，避免 HiDPI 下坐标偏差（可参考 `viewport_size()` 返回的 innerWidth/Height 与 `image_metadata` 的像素比做换算）。
5. **截图即事件**（来自 harness §3.4 不变量）：agent 触发的截图若进入模型（视觉理解），必须作为 `tool/result` 或 `assistant/chunk` 类事件落会话日志，保证重放/审计一致。

---

## 5. 优先级建议（Prioritized Recommendation）

按“对 marguerite-core 价值 × 与 dsh/harness 调研的关联度 × 可落地性”排序：

1. **【P0】网关 RPC 作用域解析 + 取消/超时一等公民**（对应 §3.2、core-optimization-top10 ⑩）
   - 在 `gateway/methods.rs` + `protocol.rs`：IO 方法（`browser.*`、`computer_use.action`）绑定 session 作用域；`RpcError` 区分 `aborted`/`timeout`/`cancelled`；为 `pyengine/client.rs::call` 与 WS 读循环统一加 `tokio::time::timeout`。
   - 价值：直接提升“网关/RPC 层健壮性”，对应任务第一条。

2. **【P0】会话日志“model-visible == logged”强不变式 + fork/seedLength 语义**（对应 §3.4、§3.5）
   - 强化 `persistence/session.rs` 与 `context/`：所有进模型的动态内容先落事件；子 Agent/fork 带继承长度扣除；compaction 进入计量。
   - 价值：防重放不一致 + 防 token 双计/虚高（社区已验证的真实 bug）。

3. **【P1】沙箱按平台 failover + 显式不可用不上报静默降级**（对应 §3.6）
   - 扩展 `security/os_sandbox.rs` 为多平台失败回退链；不可用时返回明确错误码并阻断。
   - 价值：安全加固，对齐 harness 的 `SANDBOX_UNAVAILABLE` 原则。

4. **【P1】截图能力做极致：统一感知通道 + 隐私门控 + 重试**（对应 §4）
   - 合并 `browser.screenshot` 与 `computer_use` 截图为统一 `perceive`；webui 展示加 `no-referrer`/懒加载/追踪像素检测（来自 dsh-market）；`screenshot_png` 加空白首帧重试。
   - 价值：直接回应“基于现有基础实现截图”的需求，且 Marguerite 已具备地基。

5. **【P2】Agent Inbox steering + Turn/Step 显式化**（对应 §3.3）
   - 在 `agent/runtime.rs` + `tool_cycle.rs` 增加执行中 follow-up steering 与 Turn/Step 计数。
   - 价值：长任务可控性，对齐 harness 的 Inbox/steering。

6. **【P2】桌面端进程生命周期健壮性**（对应 §3.1 的 dsh-desktop PR #39/#59/#34）
   - gui/ Tauri 启动 `marguerite-core` 加启动失败重试/日志查看/退出；`pyengine/sidecar.rs` 加心跳 + 自动重启；安装器保证 `~/.marguerite` 跨升级保留。
   - 价值：对标 dsh-desktop 真实修复，提升 Marguerite 桌面端稳定性。

7. **【P3，可选】LAN 手机遥控 + 二维码配对**（对应 §3.1 PR #45/#47）
   - webui 已可远程访问，加二维码快速配对即可，成本低收益中等。

---

## 6. 关键澄清（给 team-lead）

- 用户原话“dsh-desktop 修复了很多问题、更新官方 Discussions、验证过的 fork 修复分支”**在 dsh-desktop 仓库中无法证实**：它是 Electron 壳，无 external fork，无 screenshot/computer_use/RPC 内核代码。
- 真正可借鉴的是**上游 deepseek-harness 的架构模式**（RPC/Inbox/会话日志/沙箱）与**社区审计出的真实 bug**（token 双计/fork 陷阱），以及 **dsh-market 的截图隐私门控**。
- Marguerite 的截图/computer_use 能力**已经领先于 dsh-desktop**；本次调研的价值在于用 harness 的架构原则把现有能力“做极致”并补上健壮性与隐私短板，而非从 dsh-desktop 抄截图代码（它没有提供）。
- 所有引用均来自公开网页/仓库（WebFetch + WebSearch，2026-08-17/18）。涉及 deepseek-harness 内部文件路径为社区架构文/仓库导航页所披露，部分为推断（已在正文标注“未提及/推断”），Maguerrite 侧路径为本地代码实测。
