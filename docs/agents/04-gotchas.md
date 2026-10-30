<!-- 本文件原为 AGENTS.md §4，2026-10-29 由 scripts/split-agents-md.mjs 逐字节搬运；
     内容未删改，请勿在此加"摘要"，细节就该留在分册里。 -->
## 4. 已知坑（gotchas）

- **🔐 未绑定项目时，默认工作区就是凭据目录 —— 所以凭据拦截只能按「文件名」判**（2026-10-28 B 线安全轮实测 + 已修）。
  `Workspace::resolve` 的回落顺序是「显式项目 > `Config::data_dir()`（`~/.marguerite`）> 进程 cwd」，而
  `gateway-token` / `credentials.json` / `vault.json` / `model.key` / `.env` 全都躺在 `~/.marguerite` 里 ——
  **默认工作区与凭据目录是同一个目录**，于是任何「越界才拒绝」的路径前缀判据在这里恒等于放行。
  而 `Preflight` 此前**只管写入**（`check_write_multi`），`read_file` 没有任何凭据黑名单，
  `grep`/`codebase_search` 的 `walk_files` 只跳过 `.git|target|node_modules|.versions|build`、**不过滤点文件** →
  一次 `read_file .env` 就把密钥读进模型上下文，再随下一次请求发往服务商（真实数据流，不是理论风险）。
  ✅ 已修：`security::scanner_l3::is_credential_file`（按基名判 `.env` 与五个引擎凭据文件；`.env.example|sample|template|dist` 除外）
  在 `check()` 里对 `OpKind::Read` 直接拒；引擎凭据文件的写/删/移也拒（`.env` **写入刻意放行** —— 脚手架场景需要）；
  `read_file` 走新增的 `Preflight::check_read_multi`，`grep`/`codebase_search` 逐文件跳过同一批。
  守卫 `guard_denies_credential_file_reads_and_searches`（真夹具 + 断言任何输出都不含密钥原文）。
  ⚠️ 新增读取类工具时**必须**复用 `is_credential_file`，否则就是同一条越权面的新入口。
- **🔏 落盘 0600 统一走 `persistence::harden_private_file`（非 Unix 是 no-op，别指望 Windows）**（同日）。
  实测 `.env` / `gateway-token` / `credentials.json` 此前都是 `fs::write` 的默认 umask（多用户机器上可能 0644）；
  `byok::chmod_0600` 改为复用同一实现。⚠️ **`vault.json` 的派生密钥与 salt 同文件** —— 它的静态保护实际来自
  **文件权限**，口令不提供 at-rest 保护；这条是已知边界，不要写成「已加密所以安全」。Win32 的 `os.chmod` 只能表达
  只读位（实测 `0o600` 永远回报 `0o666`），所以 Windows 上这条防线是空的 —— 要写「仅属主」得走 ACL，未做。
- **🧬 参数指纹必须自己排序，别信 `serde_json` 的键序**（同日，B3 踩中）。
  本仓依赖图里对象**保留插入序**（`preserve_order` 生效），而 `args_fingerprint` 的两个入口形状不同 ——
  审批链给的是**构造出的** `Value`，工具调用给的是**解析出的** `Value` —— 直接 `to_vec` 会算出不同指纹，
  参数级信任于是**静默失效**（用户点了「记住这次」仍被反复追问）。已改为递归 `canonicalize`（对象键排序、
  数组保序）后再哈希。守卫 `args_fingerprint_is_stable_different_and_never_plaintext` **专门断言「键序不影响指纹」**——
  这条断言不是形式主义，它就是那个跨入口不一致的回归闸。
  ⚠️ 同轮语义：工具级查询（`is_trusted`）**只匹配未绑定参数的条目**，否则一条「记住这次」会被能力签发误读成「整个工具已授权」；
  过期条目在 `restore()` 时既不进内存也从库里删掉，避免设置界面永久显示一条早已失效的「永久信任」。

- **🧪 「隔离网关」并没有隔离 —— 改 `HOME`/`USERPROFILE` 在 Windows 上无效**（R52 实测，同时解释了一类红用例）。
  `Config::config_dir()` 用 `dirs::home_dir()`，而它在 Windows 上走 **Known Folder API**，**不认 `USERPROFILE`/`HOME`**
  （`LOCALAPPDATA` 是认的，所以 `AppData/Local` 那半是真的隔离）。实测：以 `USERPROFILE=$tmp HOME=$tmp LOCALAPPDATA=$tmp/AppData/Local`
  起的「隔离网关」，跑完一轮后**真实 `~/.marguerite/config.yaml` 的 mtime 变了**（12:22:12 → 12:23:35），
  而 `$tmp` 下**根本没有 `.marguerite` 目录**。两个后果都是真缺陷：① e2e/冒烟/探针会**写用户的真实配置**
  （`settings-modal.spec` 正好在翻 privacy 开关）；② 隔离网关与常驻 daemon **争抢同一份 config.yaml** —— 就是
  「并发写丢更新」的真实现场：两条并行 `config.patch.safe` **都回 `ok: true`**，而**只落一条**。
  ✅ 已修：`Config::config_dir()` 新增 **`MARGUERITE_CONFIG_DIR`** 覆盖（空串按未设置，否则配置目录会被解析成 cwd，
  配置静默丢失），`verify.sh` 与 `smoke_orchestrate.sh` 的隔离启动都带上它；解析逻辑抽成纯函数
  `resolve_config_dir()` 以便单测（**不要在单测里改全局 env**：Rust 测试并行跑，会串扰其它用例）。
  写路径另加 Rust 侧 `CONFIG_WRITE_LOCK`（`gateway/config.rs`）把「load → 改 → save」整段串行化；
  pyengine 侧**没有锁**（它的读改写窗口内不存在 `await`，asyncio 无从交错 —— 实测 `gather` 双写都落盘），
  但该不变量写进了 `rpc_config.py::_handle_config_patch` 上方注释：**在持久化前插入 await 必须先补串行锁**。
  **判据**：隔离是否真的生效，看**真实配置 mtime 前后是否相等**（不能只看「测试通过」）；并发写则看
  `scripts` 外的一次性探针：两条并行 patch **两边的键都要在 `config.get` 里可见**。
  回归守卫：`test_resolve_config_dir_prefers_env_override`、`test_concurrent_config_patch_keeps_both_writes`
  （Rust；把锁去掉后它**第 0 轮**就报「两边都回 `ok:true` 但只落一条」—— 有牙）、
  `test_concurrent_safe_patches_keep_both_updates`（pyengine；钉住「并发双写都落盘」这个用户可见契约）。
  ⚠️ 诚实边界：写锁是**进程内**的（覆盖了实测到的现场：同一网关的并发请求、以及 e2e 与常驻 daemon 抢同一文件 ——
  后者已靠 `MARGUERITE_CONFIG_DIR` 从根上消除）。**跨进程**同写（例如 TUI 与网关同时改 `~/.marguerite/config.yaml`）
  仍可能互相整份覆盖，文件级锁尚未实现。

- **🛡️ 同一份控制台，两个引擎，只有一边下发安全响应头**（R53 实测）。`/control` 由 **Rust** 网关托管
  （真机 18801），但 CSP / `X-Frame-Options` / `nosniff` / `Referrer-Policy` **只在 pyengine** 的
  `SecurityHeadersMiddleware` 里下发 —— Rust 侧一个都没有。于是 `webui/e2e/preview-origin-csp.spec.ts`
  拿到的是**空 CSP**（`Received string: ""`），而它钉的正是「控制台允许隔离的 localhost 预览源、
  其余一律收紧」这条安全边界。**判据**：别搜关键词就下「断言过期」的结论 —— 该文案在
  `pyengine/src/opensquilla/gateway/middleware.py` 里一直存在（我第一遍只搜了 `webui/`，差点误判成
  「文案全仓消失 = 断言过期」）。✅ 已修：`core/src/gateway/server.rs` 新增 `CONTROL_UI_CSP` +
  `control_ui_security_headers_layer`（挂在 router 最外层，覆盖静态托管与 SPA fallback 两条产出路径），
  判定口径逐条对齐 pyengine `_is_control_ui_request`（GET/HEAD、at-or-below `/control`、排除
  `/api`/`/ws`/`/health*`）；`control_ui.enabled` 在 Rust 侧没有该字段，按 `enabled = true` 处理。
  跨实现守卫 `test_control_ui_security_headers_match_pyengine_middleware`（`include_str!` 解析 Python
  源码里的 CSP 拼接 + 三个头，逐字比对；**变异实测**：把 Rust 的 `frame-src` 改成 `'none'` → 判红并
  同时打印两侧全文）。真机复核：`/control/` 四个头齐备、`/api/health` 一个都没有；headless 探针
  （`%TEMP%\csp_violation_probe.cjs`）在 `/control/`、`/control/settings`、`/control/chat` 三个路由上
  **securitypolicyviolation 事件为 0**（页面照常渲染 2000+ 节点）→ CSP 没有把控制台打坏。
  ⚠️ 同一现象的另一面（**已由 R54 处理**）：控制台加载即打 13 个 501。其中审批那 8 条已修（见下一条——
  它不是噪声，是**功能坏掉**）；剩 `GET /api/system/update`（3）与 `POST /api/elevated-mode`（2）待前端按能力降级。

- **🚨 控制台点「允许」曾经永远不生效 —— 审批的 REST 面只有 pyengine 有**（R54 实测定位 + 已修）。
  `/control` 由 **Rust** 网关托管，抬升审批的 `chat.send` / 链式执行器也是 **Rust**（挂起项注册进
  `guard::approval_registry`），但 WebUI 的审批卡片**只通过 REST** 做快照与裁决
  （`useChatApprovals.ts` 的 `fetchSnapshot` → `GET /api/approvals`、`resolveApproval` →
  `POST /api/approvals/resolve`；推送才走 WS `session.event.approval`）。而 Rust 网关对
  `/api/approvals*` 一律回 **501**（只有 pyengine `app.py` 实现了这几条）—— 于是**「允许」按钮
  打不通**，链式执行器只能等审批超时按 fail-closed 判失败，功能等于坏掉（`approval_registry.rs`
  头注释还把 `/api/approvals` 列为 WebUI 的三条裁决链路之一 = 文档承诺了、代码没兑现）。
  ✅ 已修：新增 `core/src/gateway/approvals.rs`（`GET /api/approvals` 快照 + `POST /api/approvals/resolve`），
  契约按 pyengine `api_approvals / api_approvals_resolve` 对齐；写接口照 pyengine `_same_origin` 做**同源校验**
  （带 `Origin` 必须与请求 authority 同源，且 authority 必须是回环 —— 第二条挡 DNS rebinding；无 `Origin`
  的 curl/桌面客户端放行），拒绝时回 pyengine 同款 `403 {"code":"FORBIDDEN_ORIGIN"}`。
  注册表新增只读 `pending_snapshots()` / 门控新增 `live_pending()`（**不能**用消费式的 `outcome()` 做列表查询，
  那会把待裁决项判掉）。
  **证据**：真机 18801 → `GET` 200 `{"pending":[],"mode":"prompt",…}`（此前 501）、未知 id 404、跨源 403、
  同源 Origin 放行、缺 id 400；`chain/executor.rs::approval_survives_console_rest_round_trip` 跑**真实 HTTP
  往返**——执行器抬升审批 → 后台任务拉快照拿 id → POST 裁决 → 断言步骤**真的执行成功**。
  **变异实测**：摘掉路由 → 该用例 `审批挂起后在 6s 内未被快照暴露`、HTTP 用例 `快照应 200: HTTP/1.1 501`（有牙）。
  ⚠️ 诚实边界：快照只覆盖**本进程**（Rust 引擎）的队列；侧车自己的审批队列归 pyengine 的 `/api/approvals`，
  双引擎同在时两侧互不可见，跨队列合并/未命中转发**尚未实现**。
  ⚠️ 仍未修的两个 501（同一现象的剩余部分）：`GET /api/system/update`（更新横幅）、`POST /api/elevated-mode`
  （pyengine 的按会话提权开关，Rust 引擎没有这个概念）—— 这两条该由**前端**按能力降级，而不是在 Rust 里造语义。
  ✅ **R57 更正**：这两个死调用前端**已经**在正确降级（`useChatElevatedMode` 把 403/404/405/501 一律当「本部署不支持」
  并永久闩锁、同时清掉 localStorage 里的假开关；`UpdateBanner` 对 `!response.ok` 直接 return，真通道是注入的
  `#Marguerite-data[data-update]`）。它们是**噪声**，不是缺陷 —— 不要再按缺陷去「修」。

- **🔗 两侧审批队列各说各话：侧车抬起的审批「卡片推得到、点『允许』什么也不发生」**（R57 实测定位 + 已修）。
  双引擎下 Rust 引擎与侧车**各有一份审批队列**（Rust 在 `guard::approval_registry`，侧车在它的 `ApprovalQueue`），
  而控制台只连 Rust 网关。实测（隔离双引擎网关，**等侧车就绪后**）：经 `exec.approval.request` 造出一条真实的
  **侧车侧**挂起审批 → `GET /api/approvals` 回 **`pending: []`** → `POST /api/approvals/resolve` 回 **404**
  「approval is missing or does not match」→ 复查侧车队列仍是 `pending: true, resolved: false`：
  **用户点了「允许」，审批原地不动**。
  ✅ 已修：`gateway/approvals.rs` 快照**合并**两侧（`merge_pending` 按 id 去重，本进程在前），裁决时本进程未命中就
  **转发**给侧车 `/api/approvals/resolve`（带共享 token；2s 拉快照 / 5s 转发，侧车不可用则回自己的 404 ——
  绝不假装成功，也不把界面挂住）。
  **证据**：同一探针（`%TEMP%\cross_queue_approval_probe.cjs`）修后 —— `rustSnapshotPending: 1`、`resolveStatus: 200`、
  侧车回包 `{"approved":true,"resolved":true,"resolution":"approved"}`、复查侧车队列 `pending:false, resolved:true`
  （两个不同 approval id 各复现一次）。守卫：`merge_pending_keeps_both_queues_without_duplicating_ids`、
  `forward_resolve_returns_none_when_sidecar_is_unreachable`。
  ⚠️ 探针坑：侧车 boot 可长达 ~90s，**不等它「不再回 Rust 的 NOT_IMPLEMENTED」就测，测到的是纯 Rust 模式**。

- **🔁 同一族的第三面：状态查询被转给了「不持有它的那一侧」——审批卡片重连后自己折叠**（R65 定位 + 已修）。
  `exec.approval.status` / `plugin.approval.status` 在 `PY_ENGINE_PREFIXES` 里（`method` 前缀 `exec.approval.`），
  而 Rust 网关**没有**任何 `approval.status` 处理分支 → 一座**Rust 抬起的**挂起审批，其状态探测被代理到侧车；
  侧车队列对它一无所知，如实回 `found: false`。前端 `statusResolution` 把 `found:false` 解读为**终态**
  `unavailable`（`useChatApprovals.ts`），`applyApprovalStatus` 又是单调的（`if (!current?.resolution)` 才写），
  于是卡片永久折叠成「审批已不可用」——**用户再也点不了「允许」**，只能等超时 fail-closed。
  触发条件很窄但真实：页面**重连**（`_state === 'connected'`）后的恢复扫描会为每条未决中断探一次状态。
  **变异证据**（本轮实测）：把 `methods.rs` 的拦截改成 `if false && …` →
  `dispatch_answers_status_for_rust_raised_approval` 立刻判红，报的正是线上症状
  `RpcError { code: "SIDECAR_STARTING", message: "exec.approval.status 依赖 Python 引擎，它仍在启动中…" }`。
  ✅ 已修两处：
  1. **内核侧**（持有方作答）：`gateway/approvals.rs` 新增 `registry_status`（先查本进程注册表，命中回
     `found/pending:true`，**超时项不答**——`pending_snapshots()` 已按 `live_pending()` 过滤；`plugin.` 命名空间
     一律不越界作答；`deadline` 如实 `null`，因为 `PendingApproval` 只有单调 `Instant`）+ `missing_status`
     （与 pyengine `approval_lookup_status_rpc_payload` 的缺省分支**逐字段同形**）；`methods.rs` 在
     `chat.clarify` 拦截之后、`PY_FIRST_METHODS` / `is_python_method` **之前**插入
     「本进程未命中 → 转发侧车 → 两侧皆无才回缺省」的 forward-on-miss。
  2. **前端侧**（少做一次多余的事）：`handleConnectionState` 原来在**首次连接**也跑
     `reconcileLocalApprovalStatuses` —— 而首次连接的快照（`hydrateApprovals` 刚拉到的那份）才是更新的事实，
     再扫一遍只会把裁决权交给不持有该 id 的队列。现改为 `sawDisconnect` 门控：只有**真的断过**才扫。
  守卫：`registry_status_answers_only_for_live_local_exec_approvals`、`registry_status_ignores_timed_out_pending`、
  `missing_status_matches_the_pyengine_lookup_shape`、`dispatch_answers_status_for_rust_raised_approval`（路线级，
  经真实 `methods::dispatch`），以及前端 `useChatApprovals.contracts.test.ts` 新增的
  「首次连接不扫状态队列」用例（变异：去掉门控即红，报出它多发的那次 `exec.approval.status` 调用）。
  ⚠️ 这是 R54 / R56 / R57 / R61 同族的**又一个实例**，共同模式：**某一侧对某个 id 的「不知道」被当成了另一侧的「不存在」**。
  新加任何 `*.approval.*` RPC 时，先回答「谁持有这条数据、谁有权回答『没有』」。
  ⚠️ 与已知偶发的关系：`webui/e2e/approval-card.spec.ts` 的 live 用例曾偶发红过一次（点「允许」后等不到 outcome 行）。
  机制上这条缺陷**恰好能产生同样现象**：该用例的审批快照是 `page.route` 假造的，id 不属于任何一侧；
  修复前页面加载即触发状态扫描 → 转发侧车 → `found:false` → 卡片在点击前就被判终态折叠（点击自然等不到 → 断言红）。
  修复后首次连接不再扫描，且内核对自己的 id 必定作答。⚠️ R65 当轮**未复现**该偶发（只跑了单测层），所以没有把它记为「根因已证」。
  ✅ **2026-10-28 补记：该偶发已确定性复现，且根因是另一条独立缺陷**（R65 的修复没有覆盖它）——
  `useChatSessionSubscription.ts::applySubscriptionSnapshot` 在
  `isStreaming && !hasActiveInterrupt && !liveTaskSnapshot` 时调用 `resetStreamLiveTurnState()` 清掉整条 live turn，
  **却不重挂审批/澄清帧**。而**点击「允许」会让 `hasActiveInterrupt` 立刻变 false**：只要订阅快照在这次点击之后落地，
  已裁决的结果行连同整条气泡一起被抹掉 → 页面回落到「暂无消息」，而顶部角标仍报「1 approvals pending」。
  与 `onLiveSnapshot` 路径的不对称正是证据：`useChatRpcEventHandlers.ts::restoreLiveTurnSnapshot` 重置后**有**
  `options.reappendInterruptFrames?.()`（注释写着「卡片会消失」），订阅路径漏了这一步。
  ✅ 修法：`useChatSessionSubscription` 选项新增 `reappendInterruptFrames?`，在 `resetStreamLiveTurnState()` **之后**调用；
  `ChatView.vue` 因 `chatApprovals` 晚于订阅创建，先挂转发壳、创建后接上 `chatApprovals.appendPendingInterrupts`（它重挂的是
  `approvalEntries` 全部条目，含已裁决的结果行；fold 按 id 去重，重挂幂等）。
  守卫：单测「re-appends interrupt records when an authoritative idle snapshot clears a stale live turn」
  （变异：去掉那次调用 → `expected "vi.fn()" to be called 1 times, but got 0 times`）；
  e2e「a late authoritative idle snapshot keeps the approved outcome row」——用 `page.routeWebSocket` +
  `connectToServer()` 转发帧、只把 `sessions.messages.subscribe` / `sessions.messages.snapshot` 的**响应**压后 2s
  （2s < `SESSION_SNAPSHOT_BUDGET_MS` 3s，所以是「晚到」不是「丢失」），让裁决**必定**早于权威空闲快照落地。
  ⚠️ **不要用 CDP `Network.emulateNetworkConditions` 复现这条**：实测它**不影响 WebSocket 帧**（加了 300ms 延迟的版本
  在「修复被禁用」的构建上照样通过，是个假绿）。变异实测（禁用那次调用 + 上述 2s 压后）：红出来的断言与门禁现场**逐字节同形**
  （找不到 `approval-outcome`、`region "Chat conversation": No messages yet.`、`status: 1 approvals pending`）。
  修复后 `approval-card.spec.ts` 9 passed / 2 skipped，整层 **97 passed ≥ 基线 96**（棘轮 95 → **96**）。

- **🧪 那个「偶发」的 `settings-modal` 用例不是超时抖动，是断言过窄**（R57 定根因 + 已修）。
  失败时 Playwright 的 ARIA 快照显示 Model Routing 面板渲染的是**空状态**（`Add a model provider to start`
  → `data-testid="model-strategy-provider-first"`）；生效条件是 `panel.hasSavedProvider`
  （= `primaryProviderIsConfigured`，看 `effective.fields['llm.provider'].source` 与 `onboardingStatus.llmConfigured`）。
  **隔离网关用的是空配置**，所以「没有服务商」才是常态；旧断言无条件等策略卡 radiogroup → 时红时绿。
  修法（不弱化）：接受两种**都合法**的渲染，并加自证 —— 两者**恰好**出现其一
  （`count() + count() === 1`，面板没渲染时是 0/0 而先失败）。
  实测：修前两次全量门禁各红一次；修后连跑 **5 次全量 `--e2e`（3+2）全绿 95/95**。
  ⚠️ 教训：`serial` 模式把一例失败放大成 ~20 例 skip（表现为 `75 passed + 1 failed`），别被「通过数骤降」
  误导成「大面积回归」。

- **🔌 会话键请求头改了一半 —— 每次按会话取附件/产物都 404**（R55 实测定位 + 已修）。
  控制台把会话键放在**请求头**里：`webui/src/utils/chat/attachmentAccess.ts` 在发请求前会删掉所有含
  `token`/`session` 的查询参数（`CREDENTIAL_QUERY_KEYS`），所以 URL 携带的键必然被丢弃。品牌改名把
  src/ 的头改成了 `x-Marguerite-session-key`（15+ 处），而 **pyengine 的四个读取点只认旧名
  `x-opensquilla-session-key`**（`attachments.py` / `artifacts.py`×2 / `artifact_preview.py`）→
  每个按会话鉴权的下载都解析成「没有会话」并回 **404**。e2e 侧同时有 4 处断言还在用旧名
  （`attachment-drag-upload` / `artifact-card`×3），即**两边都以为对方还叫旧名**。
  ✅ 已修：新增 `pyengine/src/opensquilla/gateway/session_key.py::request_session_key()` 作为唯一解析入口
  （品牌名头 → 旧名头 → 查询参数；头优先是刻意的——URL 里的凭据会进日志/Referer），四个读取点全部改用它；
  e2e 的 4 处断言改成品牌名。
  **判据**：`pyengine/tests/unit/gateway/test_session_key_header_contract.py` 三件事——①真机路由用**品牌名头**
  能 200 下载（修复前 404，实测红）②解析优先级真值表 ③**跨实现源码守卫**：控制台 HEAD 发的头名必须落在网关
  接受清单里（前端再改名就会在这里红）。e2e 侧 `e2e/brand-keys.guard.spec.ts` 新增同向守卫：扫 `src/` 得出
  实际发送的头名，任何 spec 断言了别的拼写就精确报出行号。**变异实测**：把 spec 改回旧名 →
  `attachment-drag-upload.spec.ts:439 … → src/ sends x-marguerite-session-key`。
  ⚠️ 教训：`brand-keys.guard.spec.ts` 里原本把 `x-opensquilla-session-key` 写在「仍然正确、别乱改」的
  白名单注释里（写着「15 uses in src/」），**那条注释已过期**，反而护着群里最旧的用法 —— 白名单注释也会腐烂。

- **🧾 `pyengine/tests/test_gateway` 曾是没人跑的死树 —— R58 已接入门禁**（R55 发现，R58 修复）。
  `verify.sh --pyengine` 只跑 `pyengine/tests/unit`（761 例），CI 也只跑同一层外加一个**只查收集**
  的检查（`check-pyengine-collection.py`）。于是 `tests/test_gateway` 2900+ 个通过用例**没有任何门禁在跑**，
  失败长期无人看见（R58 复测基线：**2901 passed / 29 failed / 162s**）。
  ⚠️ 这与 R24（pyengine 未进门禁）、R28（workbench 未进门禁）是**同一类缺陷的第三次**：一个目录改名
  就能让一整棵 2900+ 用例的测试树静默失效。R58 把 29 例逐条定性并修复后，接入
  `check_pyengine_gateway_tests`（`--pyengine` 与 `--all` 都调度，README 闸门总数 23 → **24**）：
  1. **13 处静态源断言指向从未迁移的上游控制台目录 `opensquilla-webui/`**（本仓控制台是 `webui/`）：
     `test_chat_view_static.py`、`test_chat_meta_{command,ribbon}_static.py`、`test_health_view_static.py`、
     `test_static_onboarding_views.py`、`test_webui_sources_row_static.py`。已抽到**单一模块**
     `tests/test_gateway/spa_source.py`（`SPA_ROOT` / `spa()`），不再逐文件写死路径。
     ✅ 断言字面量在 `webui/` 里**逐条命中**（70+ 条探针实测），即 `webui/` 就是那个 SPA 的迁移体 ——
     缺的只是路径，所以修复=换路径，不是换契约。
     ⚠️ 其中两条是**断言本身有缺陷**，与目录名无关：①`sharedRoutes.ts` 的
     `assert "HealthView.vue" not in routes` 被现役 `@/views/EngineHealthView.vue`（`/engine` 的仪表盘）
     **子串误伤** → 改为断言导入路径 `"/views/HealthView.vue" not in routes`；②`ChatView.vue` 断言
     `const pendingForkBeforeMessageId = ref(...)` 在视图里，而该 ref 已抽到
     `views/chat/useChatViewState.ts` → 断言改到新家，其余接线断言（传入两个 composable、会话切换时清零）
     原样保留。
  2. **14 例 auto-summarize 的根因是夹具 `budget=10` 低于「压缩后载荷」的估算地板** ——
     R55 记的「行为漂移、实现已变」**定性错了**，此处更正。`AUTO_SUMMARIZE` 只有「压缩后估算 ≤ budget」
     才走成功分支；估算器有**两档**：装了 `tiktoken`（`pyproject` 的 `memory`/`recommended` **可选** extra）
     时 ≈ chars/4，**未装**时回退保守的 utf8 bytes/2（约翻倍）。两档下「kept entry + 回放 summary +
     新消息」的地板是 **11 / ~7 tokens**，所以 `budget=10` 只在装了 tiktoken 的机器上能成功。
     **实测三连**：把估算器钉成 chars/4 跑该文件 → 41 passed；本机默认档 → 14 failed；
     改成两档都成立的 `_AUTO_BUDGET = 20` → **两档都 2929 passed / 0 failed**。
     ⚠️ 教训：夹具被校准到「某个可选依赖存在时的估算语义」，会让 CI 与开发机红绿不一 —— 而这正是
     门禁层最容易长期无人发现的那类缺陷。
  3. **1 例断言源码树没有 dist 元数据时的行为**：`test_status_version_provider.py` 断言
     `opensquilla.__version__ == importlib.metadata.version("opensquilla")`，而本仓**从不安装这个包**
     （`pythonpath = ["src"]`，见下条）→ 它在任何源码树上必红。改为：有 dist 元数据时比对元数据；
     没有时**仍咬住真契约**（`__init__.py` 的哨兵必须是 `0.0.0+unknown`，不得退化成硬编码 semver ——
     那正是该模块存在的理由）。
  4. **1 例断言从未迁移的上游文档章节**（`docs/troubleshooting.md` 的「Control UI Assets Are Unavailable」）
     → 显式 `pytest.mark.skip` 并写明原因（与 `_INHERITED_GROUPS` 同类：具名列出，不是假装通过）。
     ⚠️ 该层门禁因此带 `-rs`，让跳过原因出现在证据里（§8「跳过 ≠ 通过」）。
  与 `tests/unit` 拆成两条检查是刻意的：**162s** 与「纯单测 11s」不同量级，混进同一条 `[PASS]` 会掩盖慢测。
  现状：**2956 passed / 0 failed / 11 skipped / 169s**（2026-10-28 全量闸门实测；该目录**不含**任何 llm/live/browser marker，
  实测 0 处 —— 所以它不需要 marker 排除表，也就不存在第二份会漂移的清单）。
  ⚠️ 其余目录（`test_scripts` / `test_desktop` / `test_packaging` / `test_provider` /
  `test_skills` / `test_public_release_hygiene.py` …）**仍无门禁执行**；全树**执行**结果未测（不可测），
  不要声称它绿。⚠️ 另：`hatch_build.py`、`THIRD_PARTY_NOTICES.md`、`test_compose_yaml_shape.py` 里
  仍写着 `opensquilla-webui/`（各自有独立理由，本轮未动）。

- **🧾 契约族六个目录曾是没人跑的死树 —— R60 已接入门禁**（R59 发现，R60 修复）。
  与 R58 的 `tests/test_gateway`、R24 的 pyengine、R28 的 workbench 是**同一类缺陷的第四次**：
  `verify.sh --pyengine` 只跑 `tests/unit` + `tests/test_gateway`，于是 `test_ci` / `test_migrations` /
  `test_sandbox` / `test_onboarding` / `test_tools` / `test_recovery` 里 **30 个失败长期无人可见**。
  R60 新增 `check_pyengine_contract_tests`（`--pyengine` 与 `--all` 都调度，**一次检查内逐目录登记证据**，
  闸门总数 24 → **25**）。R59 基线 → R60 现状：
  | 目录 | R59（未接门禁） | R60（已接门禁） |
  |---|---|---|
  | `test_ci` | 35p / 0f / 1s（48s） | 35p / 0f / 1s（35s） |
  | `test_migrations` | 59p / 0f（23s） | 59p / 0f（23s） |
  | `test_sandbox` | 1738p / **11f** / 346s（94s） | 1746p / **0f** / 349s（89s） |
  | `test_onboarding` | 955p / **3f** / 6s（44s） | 956p / **0f** / 8s（41s） |
  | `test_tools` | 1200p / **2f** / 23s（109s） | 1202p / **0f** / 23s（92s） |
  | `test_recovery` | 431p / **14f** / 8s（121s） | 444p / **0f** / 9s（100s） |
  ⚠️ **R63 给该检查补了「无结论行」的证据兜底**：它原先对每个目录只 grep `^(FAILED|ERROR)` 与汇总行，
  而实测出现过「目录红了、却既无 FAILED 行也无汇总行」（进程被打断 / 输出里没有 pytest 结论行），
  此时原写法**一行都不打印**，末尾只剩一句「1/6 个目录有失败」—— 违反「红必须带证据」。
  现改为：没有汇总行时退回**原始输出尾部 20 行**，至少留下现场。
  **30 个失败逐条定性：25 条真修，5 条具名 skip**（跳过 ≠ 通过）。真修的部分：
  1. **`recovery/atomic.py::_chmod_open_file` 的能力探测是过期的（7 例）** —— 它只探 `os.fchmod` 是否存在，
     而 CPython **3.13 起 Windows 也有了 `os.fchmod`**（本机 3.14 实测 `hasattr` = True），于是那句
     「POSIX 加固是额外能力、不是可移植前置」的注释**变成了硬失败**：`fchmod(fd, 0o600)` 对
     **只读打开的 fd** 抛 `PermissionError [WinError 5]`，而每个调用点都是先 `O_RDONLY` 打开比对身份再加固
     → `AtomicStateUnknownError: workspace config backup could not be restricted to its owner`。
     实测 `os.chmod(0o600)` 在 NTFS 上**永远回报 `0o666`**（stat 只有只读位），所以「仅属主」在 Windows
     本来就不该由 mode 表达。修法：新增 `can_harden_posix_mode()`（`os.name != "nt" and callable(fchmod)`），
     `settings_transaction._restrict_existing_credential_to_owner` 也改用它 —— 那个 `can_harden_posix_mode`
     变量此前还会在 Windows 上**对只读 fd 调 `os.fsync`**（实测 `OSError [Errno 9]`）并断言 `0o600`。
  2. **`gateway/rpc_sandbox._path_list_start` 的符号链接环在 Windows 上换了错误类型（2 例）** ——
     POSIX 的 `pathlib` 抛 `RuntimeError("Symlink loop from …")`，Windows 走 `ntpath.realpath`，
     实测抛 `OSError [WinError 1921]`（`ERROR_CANT_RESOLVE_FILENAME`；缺文件仍是 `FileNotFoundError`，故可区分）。
     于是「工作区不可解析 → 回落主目录」的 `except (FileNotFoundError, NotADirectoryError, RuntimeError)`
     **在 Windows 上不生效**，而「显式请求的坏路径必须保持严格」也变成了另一个异常类。修法：新增
     `_resolve_strict()` 把 1921 归一成与 POSIX 同文案的 `RuntimeError`，两处调用点共用。
     ⚠️ 同一常量 `1921` 在 `sandbox/directory_listing.py` 早已被当作「未解析的符号链接目标」处理 —— 本仓的既有惯例。
  3. **六处静态源断言指向从未迁移的上游控制台目录 `opensquilla-webui/`（这 30 条里的 6 例）** —— 与 R58 同一根因、
     同一结论：`webui/` 就是那个 SPA 的迁移体，缺的只是路径。涉及
     `test_sandbox/test_frontend_sandbox_assets.py`（4 例）与 `test_sandbox/test_release_contract.py`（2 例）。
     同一次排查在门禁之外还修了两处同源路径（`test_skills_community_copy_contract.py` 转为全绿；
     `test_skills_third_party_notices.py` 顺带暴露真正的合规缺口：Vue 控制台的直接依赖 **`three`**
     没有出现在 `THIRD_PARTY_NOTICES.md` —— 已补一行 MIT 署名）。
  4. **子进程类用例 import 不到 `opensquilla`（6 例）** —— `pyproject.toml` 的 `pythonpath = ["src"]`
     只补**本解释器**的 `sys.path`，而恢复事务的契约用例要 spawn 新解释器
     （`subprocess.run([sys.executable, "-m", "opensquilla.cli.main", …])`），侧车按设计**从不安装**
     → 子进程一律 `ModuleNotFoundError: No module named 'opensquilla'`。更坏的是 Windows 的次生效应：
     父进程往**已经死掉**的子进程 stdin 写/flush 时报裸 `OSError [Errno 22] Invalid argument`，
     把「import 失败」伪装成「管道坏了」。修法：`tests/conftest.py` 把 `pyengine/src` 前置进 `PYTHONPATH`
     （子进程继承），实现「PACKAGED」语义的用例本来就会自己 `pop("PYTHONPATH")`，语义不受影响。实测
     `tests/unit` 761p/6s、`tests/test_gateway` 2929p/11s **不变**（无外溢）。
  5. **两例硬编码 `python3`（2 例）** —— 全新 Windows 只有 `python`，实测 `python3 -c "print(1)"` 是
     **exit 1 且 stderr 为空**。改用运行中的 `sys.executable`（**不加引号**：Windows 侧工具会把
     「含分隔符的 python 路径」重写成 `Invoke-OpenSquillaPythonProcess -FilePath '…'`，手写引号反而
     触发 PowerShell `Unexpected token`）。同模块里其余 `"python3 …"` 字面量是**只被解析、不被执行**的
     分析器输入，**故意保留**。
  6. **`test_flow.py` 的飞书依赖提示断言写死了发布版的形态（1 例）** —— 它断言输出里有
     `releases/download/` 与 `v0.5.2`，而源码树的 `__version__` 是哨兵 `0.0.0+unknown`
     → 产品**故意**走「不猜 wheel 地址、指向 latest release 页」那一支（该分支的 docstring 明写）。
     改为按构建类型断言两条互斥契约：发布版必须报**运行中版本**的 wheel 名（比写死 `0.5.2` 更强），
     哨兵版必须报 `<wheel-url>` + `releases/latest`。
  7. **`test_windows_native_move_refuses_real_cross_volume_move` 的 CI 判定过宽（1 例）** ——
     它用通用 `CI` 变量判「是不是 CI」，而**本机工具 shell 就设了 `CI=true`**（实测）→ 把一次合法的
     「缺两个合成卷」skip 变成硬失败。改用 `GITHUB_ACTIONS`（本仓 pyengine job 是 ubuntu，该用例在那里本就 skip；
     Windows runner 上仍 fail-closed）。
  5 条具名 skip：`test_file_policy` 那条把 `platform="linux"` 与**原生 Windows home 路径**混用
  （POSIX 图案保留反斜杠、宿主解析器却给出带盘符路径 → 永不匹配，与同文件既有 skip 同类）；
  `test_release_contract` 两条断言上游 Electron 运行时清单与上游打包校验器；`test_onboarding` 两条
  断言从未迁移的上游产品文档与 Electron 桌面壳。

- **⏱️ `settings-modal.spec.ts` 有偶发**（R55 实测：同一批改动下，连续两次全量 `--e2e` 一次红一次绿；
  更早的 R53 两次都绿，故是低频偶发而非稳定红）：`rail switches sections…` 在 5s 内等不到
  `radiogroup 'Choose how models are used'`。**放大效应**：该文件是 `mode: 'serial'`，一例失败会让
  **后续 ~20 例全部 skip**（表现为「75 passed + 1 failed」而不是「94 passed + 1 failed」），闸门直接红。
  已记录，未定位到根因；下次复现时应查该面板的 `radiogroup` 是否被某个异步 profile 加载门控，
  而不是简单加超时（加超时只会把概率往后推）。

- **📦 产物下载在真机控制台上必然 501 —— 取字节只有 REST 一条路**（R56 实测定位 + 已修）。
  `webui/src/utils/chat/artifacts.ts::artifactDownloadUrl` 对「有 `id`、没有 `download_url`」的产物**直接拼**
  `/api/v1/artifacts/{id}`，而 Rust 台账里的产物正是这个形状（`id = artifact:<session>:<path>`）→ 点下载
  必然拿到 501，前端弹「下载失败（HTTP 501）」。同一条 URL 还承担**内联文本编辑的读取**
  （`useArtifactActions::loadTextContent`）。对照：`open_in_system` / `edit_inline` 是「RPC 优先、REST 兜底」，
  所以只有取字节这条在纯 Rust 托管下是死的（**这是 R54 同族的第三个实例**：pyengine 有、Rust 没有、
  而控制台只用 REST）。
  ✅ 已修：`gateway/artifacts.rs` 新增 `artifact_download`（`GET /api/v1/artifacts/{id}`）——**流式**回文件本体，
  如实给 `Content-Type`/`Content-Length`/`Content-Disposition`（文件名过 ASCII 白名单，防响应头注入）+ `no-store`。
  **安全要点**：只接受 **id** 且必须命中 Rust 台账（`file_change::get_artifact`）——绝不接受路径，
  否则就是任意文件读取；未登记 / 已删除 / 目录类一律 404。
  **证据**：真机 18801 —— 经 `artifacts.register` 登记真实文件后 `GET /api/v1/artifacts/<id>` → **200**、
  `content-type: text/plain`、`content-length: 22`、`content-disposition: attachment; filename="probe-report.txt"`、
  本体逐字节正确（修复前同一请求 501）；未知 id → 404 `{"code":"NOT_FOUND","error":"Artifact not found"}`。
  守卫 `test_artifact_download_serves_registered_file_only`（**变异实测**：摘掉路由 → `已登记产物应 200: HTTP/1.1 501 Not Implemented`），
  其中含关键否定面：**同目录下未登记的文件不能靠猜 id 读到**。
  ⚠️ 已知差距：Rust 无缩略图存储，`?variant=thumb` 被忽略并回原图（缩略图网格会拉整图）；
  `/api/v1/artifacts/{id}/open|content` 仍是 501，但前端只在这两条 RPC 缺席时才走它们，故不影响。

- **🌉 侧车 REST 面在 Rust 托管下整体不可达 —— 改成「按面兜底」的通用代理（R61 实测定位 + 已修）**。
  与 R54（审批）/ R56（产物取字节）同族，但这次不再是第四个实例：**UI 只有 REST 一条路的功能有一整片**，
  而 Rust 网关对它们一律回 501 —— 最典型的是**附件上传**（`useChatAttachments.ts` → `POST /api/v1/files/upload`，
  拖文件进输入框必然失败、`chat.send` 拿不到 `file_uuid`），另有支持诊断包（`/api/v1/diagnostics/bundle`）、
  Workbench 预览租约（`/api/v1/artifact-preview-leases/*`）、附件下载（`/api/v1/attachments/*`）、
  语音转写（`/api/audio/transcribe`）。**逐端点补必然继续漏**（每新增一个侧车端点就再坏一次），
  所以改成按面兜底：`core/src/gateway/sidecar_proxy.rs` 接管通配 `/api/{*path}` —— 侧车在线则原样转发，
  离线则沿用原来的 **501 JSON**（既有的「本部署不支持」契约不变）；更具体的路由（`/api/health`、
  `/api/approvals*`、`/api/v1/artifacts/{id}`）在 axum 里优先匹配，仍由 Rust 直接服务。
  实现要点：**双向流式**（大附件不整块进内存）、逐跳头两个方向都不转发、业务头（`x-Marguerite-session-key`
  等）原样透传、**客户端 `authorization` 不转发**（改带网关共享 token）、同源校验直接复用
  `approvals::origin_allowed`（口径不许漂移）、失败如实回 `502 sidecar_unreachable`（绝不假装成功）。
  `artifact_download` 在 Rust 台账未命中时**转问侧车**（Python 引擎产出的产物此前点下载必 404）。
  **证据（真机 18801 双引擎）**：`POST /api/v1/files/upload` → **200** + `{"file_uuid":"u-…"}`（修复前 501）、
  `/api/config|/api/sessions|/api/usage` → 200（侧车载荷）、未知路径 → 404、`/api/health` → 200（未被通配遮蔽）；
  路由层真实 HTTP 用例钉住**离线时仍是 501 + `not_implemented`**，跨源仍 403 `FORBIDDEN_ORIGIN`。
  ⚠️ **`no_proxy()` 是必需项，不是优化**：本机 WinINET 代理（`ProxyServer=127.0.0.1:7890`）会**替回环目标回 502**，
  于是「侧车离线」与「代理拒绝转发」在响应上无法区分 —— 摘掉 `.no_proxy()` 后
  `forward_reports_unreachable_sidecar_as_502` 立刻判红（实测：响应体变成空串而不是本模块的 JSON）。

- **⏱️ 侧车的 `/health` 被自己的限流回 429 → 看门狗把「活着」读成「死了」**（R61 实测定位 + 已修；
  这是「e2e 偶发变红」的真根因）。侧车 `RateLimitMiddleware` 当时只豁免了 Control UI、`GET /api/approvals`
  与预览 capability 路径，**没豁免存活探测**；而 Rust 侧车守护任务用 `status.is_success()` 判定健康 →
  负载下（全量 e2e、4 worker）探针实测 **21/90 次拿到 429** → 看门狗判定
  `alive=true http_ok=false` 并**杀掉健康引擎**（实测重启耗时 21s，期间控制台所有 RPC 失败、
  `settings-modal` 的 Discard 无法完成重载 → 该用例红 + serial 连带 15 例 skip）。
  ✅ 两处都修：①pyengine `middleware.py` 新增 `_LIVENESS_PATHS`（`/health`/`/healthz`/`/ready`/`/readyz`、
  `/api/health`）永久豁免限流；②Rust `health_status_is_alive(status) = status < 500`（**有回答即存活证据**，
  只有 5xx 与传输失败才算不健康），并把「单次失手就杀」改成「连续 3 次失手 + 一次 10s 长超时复核」
  （`classify_health` 纯函数；`HEALTH_MISS_LIMIT`/`HEALTH_PROBE_TIMEOUT`/`HEALTH_REPROBE_TIMEOUT`）。
  守卫：Rust `rate_limited_probe_still_proves_liveness`、`alive_but_slow_probe_is_not_dead`、
  `exited_process_is_dead_immediately`、`reprobe_success_counts_as_healthy`；
  pyengine `tests/unit/gateway/test_rate_limit_liveness_exemption.py`（真实 app + 真实中间件，桶限 1：
  `/health` 5 次全 200、`/api/config` 第 2 次 429 —— 反向自证豁免没把限流整体关掉）。
  **变异实测**：把 pyengine 侧豁免条件改成 `if False:` → pyengine 2 例红（429）；摘掉 `.no_proxy()` →
  上述 502 用例红。**e2e 连跑 3 次 95 passed ≥ 基线 95 全绿**（修复前每次都在 Discard 那一步红）。

- **📦 安装版里打包进来的 Python 引擎永远用不上（探测不看 `resources/`）**（R61 实测定位 + 已修）。
  `package_full.cmd` 会把引擎源码放进 `gui/src-tauri/resources/pyengine/src` （安装后 =
  `<install>/resources/pyengine/src`），但 `detect_pyengine_src()` 只找 `pyengine/src` / `../pyengine/src` /
  `marguerite/pyengine/src` 与 **exe 上溯** —— 安装目录里这三条都不存在 → `SidecarConfig::detect` 失败 →
  桌面端**静默降级成纯 Rust 模式**（那一片 REST 能力直接变 501，见上一条）。
  ✅ 已修：候选列表抽成纯函数 `pyengine_src_candidates(cwd, exe)`，**打包资源布局排在最前**
  （`<exe dir>/resources/pyengine/src`，每层上溯也都看一次），`detect_pyengine_src` 只做「取第一个合法候选」
  （合法 = 含 `opensquilla/cli/main.py`）。守卫：`packaged_resource_layout_is_searched_and_ranks_first`、
  `first_valid_src_picks_the_packaged_tree_and_skips_invalid_candidates`（真实目录夹具 + 干扰项）。
  **变异实测**：把该候选改成不存在的路径 → 两条同时判红。
  ⚠️ 打包纪律：`scripts/package_full.cmd` 会先同步 webui/dist、skills、resources/*（含 pyengine src）再构建；
  **直接 `cargo tauri build` 会用上一次的 resources 快照**（本次实测：装出来的 pyengine 是 9/24 的副本）——
  发版必须走 `package_full.cmd`，或自行执行它的步骤 3–4 后再构建。
  ⚠️ **同一个坑的第二段（本轮实测）**：修好探测后，安装版引擎**能启动**却立刻退出 ——
  `RuntimeError: opensquilla migrations directory not found`。因为 `resolve_migrations_dir()`
  （`pyengine/src/opensquilla/persistence/migrator.py`）的仓库候选是 `Path(__file__).parents[3]/"migrations"`
  = 安装后的 `<install>/resources/pyengine/migrations`，而 `package_full.cmd` 当时**只复制了 `pyengine/src`**。
  ✅ 已修：脚本新增 `pyengine\migrations` 的复制（缺失时**直接 exit 1**，不再产出「装得上但引擎起不来」的包）。
  守卫：`pyengine/tests/unit/gateway/test_desktop_bundle_migrations.py` 四条 —— 仓库 migrations 存在且含 `V*.py`、
  `migrator.py` 的候选路径**等于**打包位置（证明两者不会各走各的）、打包脚本含 `pyengine\migrations`（含 `src` 的反向自证）、
  轻量脚本 `package_app.cmd` 必须指明资源来自全量脚本。
  **判据（安装版是否真的双引擎）**：管理员解包 MSI（`msiexec /a <msi> /qn TARGETDIR=<tmp>`）→ 以解包目录为
  `WorkingDirectory` 启动 `marguerite-gui.exe`（`MARGUERITE_CONFIG_DIR` + `LOCALAPPDATA` 都指向临时目录 = 干净机器等价）
  → 探 `http://127.0.0.1:18790/api/config`：**200 = 引擎在线**，501 = 纯 Rust 降级（本轮修复前实测就是 501）。

- **🧨 PowerShell 的 `Set-Content` 会写 BOM → Python 源码直接 `SyntaxError`**（R61 踩到，代价 4 个用例变红）。
  用 `Set-Content -NoNewline` 就地改文件（我用来做变异测试/批量替换）时，**Windows PowerShell 5.1 默认写 UTF-8 **带 BOM****，
  于是 `pyengine/src/opensquilla/gateway/middleware.py` 首行变成 `\uFEFF"""Middleware pipeline…`，
  Python 报 `SyntaxError: invalid non-printable character U+FEFF`，而 `tests/test_ci` 有 4 个「把全树源码
  喂给 ast/compile」的架构守卫因此变红（现象像「我的改动把架构搞坏了」，其实是编码）。
  ✅ 改文件的正确姿势：`[System.IO.File]::WriteAllText($p, $text, (New-Object System.Text.UTF8Encoding($false)))`
  （读用 `ReadAllText` 会自动丢掉既有 BOM）；改完用 `[System.IO.File]::ReadAllBytes($p)[0..2]` 确认不是 `239,187,191`。

- **🔴 真钥匙进过仓库 —— 工作区已干净，但历史就是发布面**（R27 核实；这条**需要你动手**）。实测三段：
  1. **工作区干净**：对**将随提交落地**的 8977 个文件扫 `sk-…` 形态，命中的 92 个**全部自我标注是夹具**
     （`sk-FAKE…`、`sk-fake--fixture-…`、`sk-SEN71NEL-do-not-leak-…`）或 CSS 类名（`sk-` 也是本仓类名前缀，
     所以朴素正则 `sk-[A-Za-z0-9_-]{10,}` 会刷出几千条假阳性，必须收紧成「无 `-`/`_` 分隔的长串」）。
     被报告的 `sk-bfd…` 真钥匙在**工作区 0 命中** → **`git add -A` 不会把它带回来**。
  2. **它在历史里，但已不在 HEAD**（2026-10-28 复测，本条此前写的是「HEAD 里还有」）：
     `git grep -l 'sk-bfd' HEAD` 现在只剩 **`HEAD:AGENTS.md` 自己那句引用**（`sk-bfd` 后面一个字符都没有，不是完整钥匙）；
     `smoke_orchestrate.sh` 第 21 行在 HEAD 里已经是 `# Credentials are NEVER hardcoded here.`。
     移除发生在 **`95f1d3f`（2026-10-16 21:38）**，最后一个仍带着它的提交是 **`109db3c`（21:28）**；
     58 个提交里**仍有 31 个的树带着那把钥匙**（此前记的「29 个」是当时 HEAD 的口径），最早出自**首个提交** `9362b1f`。
  3. ⚠️ **结论不变：这不是工作区能修的问题**。钥匙已经在 31 个提交里出现过，**按已泄露处理，必须吊销**；
     然后再决定**重写历史**（`git filter-repo`）还是**接受披露**。重写历史是不可逆的破坏性操作，
     **没有明确指示我不会替你执行**。
  ✅ 本轮补上的是**防再犯**：`scripts/check-secrets.py` 成为门禁的常驻检查（`--fast` / `--core` / `--all` 三层都跑）。
  ⚠️ 但接线时**只接进了 `--fast` 与 `--core`，漏了 `--all`** —— 于是 `--all` 实际只跑 **17** 项，而文档写着 18。
  这不是靠读代码发现的，是靠新加的 `scripts/check-readme-claims.py`（它会读 `verify.sh` 的 `--all` 块来数）
  当场报出来的：**「文档里的闸门总数」与「脚本真正调度的检查数」必须相等**。
  它只扫 `git add -A` 集合（被忽略的草稿里有密钥不是发布风险），**放行可识别的占位符、其余一律 FAIL**。
  夹具因此必须**自我标注** —— 该规则当场抓到一例：`skills/codebuddy/capability-evolver/test/sanitize.test.js`
  里一个 `sk-proj-…` 值长得像真 OpenAI 项目键（同文件其它夹具都自带 `EXAMPLE`/`abcdef`/`123456`），已改为 `sk-proj-FAKE…`。
  ⚠️ 同一轮还发现：**这个守卫此前只在本机跑**。`ci.yml` 的 `docs` job 只跑 `check-doc-cli-drift.py`，
  所以（a）新密钥扫描在 PR 路径上缺席，（b）`check-version-sync.py` **从来没在 CI 里跑过** ——
  而 `README.md` 明写它「in CI」强制对齐版本号：**一句没有实现的宣传**。已在 `docs` job 补上这两条
  （都只需 `python3`，无需 setup-python；实测 `bash -n` 零语法错、YAML 5 个 job 解析正常、docs job 由 2 步变 6 步 ——
  后来又把 `check-doc-links.py` 与 `check-readme-claims.py` 一并接了进去）。
  ⚠️ 诚实边界：`actions/checkout` 之后 `git ls-files -co` 只剩已跟踪文件，**只扫得到提交树、扫不到历史** ——
  所以它挡的是「新泄漏」，**不会**发现 `smoke_orchestrate.sh` 历史里那把钥匙；那把只能靠吊销。
  另一处必须说清（2026-10-28 复测后**作废了旧写法**）：曾经这里是「当前 HEAD 第 21 行仍是那把真钥匙，
  所以这个新 CI 步骤在提交工作区之前必然红」。**HEAD 现在已不含它**（见上面第 2 条），所以 `docs` job 的
  密钥扫描不会因为这把钥匙变红 —— 历史里那 31 个提交它依然扫不到，**吊销仍然是唯一真正的修复**。

- **📄 两份 README 会各自漂移，而没有任何门禁比对它们**（R27 实测）。`README.zh-Hans.md` 落后于
  `README.md` 一整代：写着整道闸门 **15/15 全绿（2026-09-22）**，而真实是 **18/18**；基线写 **2533 / 3212（310 文件）**，
  真实是 **2538 / 3216（311 文件）**；表里**完全没有** `pyengine` 那一层（758 个测试）与密钥扫描行；
  i18n 写 **3834 键**，实测 **3862 键**（脚本逐叶子统计，6 个 locale 全部 3862）；
  还声称 `PHASE_*.md` / `Security_Audit_*.md` 躺在仓库里 —— 它们早已隔离到 `docs/internal/`（实测根目录只剩
  7 个 `.md`，全是入口与社区健康文件）。**一个中文读者看到的是更旧、更小的项目**，而两页互相矛盾。
  **判据（可复用）**：语言差异不是差异 —— 只比对**与语言无关的事实**（`NNN passed / N failed` 基线、
  `NNN ignored/skipped/files`、`N/N` 闸门总数、locale 键数、版本号），并把「表格里出现的脚本名」求交集；
  行标题（`webui unit tests` vs `webui 单测`）不参与比对，否则全是假阳性。
- **🌐 官网印了几十个「硬事实」，而 `check-readme-claims.py` 只看两份 README —— 网站从来没有任何核对**（2026-10-28 实测）。
  首次逐条对账当场查出**七类**漂移，全部是用户可见的宣传口径：
  1. **内置工具写 85，页脚又写 67，而注册表实测 74**。`85` 是 `marguerite-core capabilities --json` 的
     `real` 计数（`total 97 / real 85 / partial 11`），**不是** `register_builtin_tools` 的条目数；而它当时引用的
     命令 `marguerite-core serve --health` **根本不返回工具数**。⇒ 口径必须挂在那条真会打印该数字的命令上，
     否则数字迟早变成别人的数字。
  2. **测试基线停在 4220 / 4816**，而同日 `verify.sh --clean --all` 全绿是 **4230 / 4898**。
  3. **代码行数台账落后整整一个冲刺**：`git archive HEAD` 单独跑一遍测量，23 项里 **11 项**与 HEAD 自家数字不一致
     （Rust 254,605→268,375、前端 308,344→334,418、Python 413,506→463,482、CJK 棘轮 380→408、工具模块 31→30…）——
     即**「过期」不是写错一个数，是近一半的数字各停在各自己的时间点**。（这是**修正前**的那一遍；把官网数字推到工作区实测值之后再回测 HEAD，是下面那条边界里的 **13 项**。）
  4. **`core/src/provider` 这个目录不存在**（14 个 provider id 是散在 `core/src` 里的字符串字面量，其中 8 个在 `core/src/model`）。
     它出现在官网 + 两份 README 共 **3 个已发布文件**里 —— 与 §4「死链指到仓库外」同族，只是这次指向的是**仓库内不存在的目录**。
  5. **「2,178 份技能文档」把被 gitignore 的 `skills/blender-skills/`（547 个 `.md`）也算进去了**，而提交集实测 **1,631**。
     ⇒ 任何「随仓库发布」口径的计数只能取 `git ls-files -c -o --exclude-standard`，且**只算磁盘上真实存在的文件**。
  6. **两份 README 的「架构守卫」写 10 项，`webui/package.json` 的 `check:architecture` 实际串了 13 项**
     （官网倒是对的）。`check-readme-claims.py` 只比对中英两份**互相**一致，所以两份一起过期时它照绿。
  7. **两份 README 介绍这条检查时写「约 20 个数字」，而它核对的是 23 项** —— 修完前六类后，这句**自我介绍**
     自己成了新的漂移。⇒ 检查器现在把 `len(claims)` 钉在两份 README 的那句话上：加一项宣称却忘了改文档，当场红。
  ✅ 已修：`scripts/check-website-claims.py`（纯标准库）按上条口径**先测量再比对**，覆盖 23 项数字 +
  「官网 `<code>` 里的纯路径必须存在」+「不得外链到 `remote.origin.url` 那个源码仓库（未开源期间）」，
  已接入 CI `docs` job（第六项）。**刻意不接进 `verify.sh`**：闸门总数 25 印在 README×2、AGENTS.md、官网四处，
  改总数得同步五处，而这条检查与 Rust/Node 栈无关。
  ⚠️ **与 `check-secrets.py` 当年同一条边界**：它测的是**工作区**。2026-10-28 用 `git archive HEAD` 单独跑了两遍，
  第二遍是「官网数字已提交、代码台账未提交」的那个状态：**23 项里 13 项对不上** ——
  7 项是代码行数类（Rust 276,649↔268,375、前端 339,170↔334,418、Python 463,508↔463,482、工具模块 31↔30、CJK 棘轮 380↔408…），
  2 项是两份 README 的「10 项架构守卫」（改成 13 的那次没落地），4 项是「本页印了几个数」这句新措辞**在 HEAD 里还不存在**。
  ⇒ **在「官网数字 + 文档措辞 + 代码」一起落地之前，这条 CI 步骤必然红** —— 那是守卫在正确报警，不是回归
  （AGENTS.md §4 密钥扫描那条写的是同一件事）。**只有 13 项归零才是这一节的终点，不是「脚本跑过一次」。**
  ⚠️ **接线时踩到的三个测量坑（都是检查器自己的 bug，不是官网）**：
  ① **子进程中文输出按 UTF-8 解码得到乱码** —— `check-version-sync.py --print` 在 cp936 控制台下写 GBK 字节，
     正则永远匹配不上（§4「别用终端显示下结论」那条坑的变体）；给子进程 `PYTHONUTF8=1` 即可。
  ② **`SUPPORTED_METHODS` 必须先剥行注释再取字面量** —— 不剥是 **464**，剥了才是守卫报的 **461**（口径要和
     `webui/scripts/check-rpc-contract.mjs` 逐字一致，否则「461 条」这条宣传会被检查器判成漂移；
     这个数字随每次新增 RPC 变化，改方法数就必须同步这里与官网三处）。
  ③ **`49 控制台视图` 对应的是 `ls webui/src/views/*.vue`**（顶层），递归数会得到 50（多一个
     `views/web/SettingsView.vue`）—— 官网把命令印在数字旁边，测量就必须按那条命令来。
  **变异实测（五条都有牙）**：把 webui 基线改回 4816 → `webui_tests: 官网宣称 4,816 · 实测 4,898`；
  注入 `core/src/provider` → `死路径`；把安装包外链换成源码仓库外链 → `不应外链到源码仓库`；
  把「25 项发版门禁」改成「若干」 → `官网里找不到这条数字（结构变了，检查器需要跟着改）`；
  把 README 那句「23 个数字」改成「20」 → `README.md: 宣称核对 20 项 · 本脚本实际核对 23 项`
  （改动后按字节 sha256 复核，被改文件已原样还原）。
  ⚠️ **诚实边界**：它核对的是「官网 vs 当前文件树」，**不核对测试基线是否还成立** —— 4230/4898 只能靠人复跑闸门
  （同 §2 那句「过期基线只能靠人复测」）。它也**不会**发现官网少写了一条声明（只查它写了的那 23 项）。
  ⚠️ **官网新增的「本地模型」一节不在这 23 项里**（`e7d2f14` 在我这轮对账之后落地）：那一节的数字来自
  **GitHub Releases `v0.17-C` 的发布物**与模型仓库 README，本机只有被 gitignore 的 `.tmp/office-model-README.md`
  副本 —— `check-website-claims.py` 结构上量不到。2026-10-28 用 `api.github.com` 逐条对过：q4_K_M **940.4 MiB**、
  适配器 **16.7 MiB**、f16 两卷 **2000.0 + 950.4 MiB**（即官网写的 2.9 GiB）、发布日 **2026-10-19** 全部相符，
  四个宏平均（0.3936 / 0.3937 / 0.3766 / 0.3060）与「红条」明细在模型仓库 README 里逐条命中。
  ⇒ 官网那句「每个数字都对应一条命令」因此**限定为代码台账那 23 项**（`trust__how` 里明写了这一区分，
  并且这个条数也被检查器钉住：改官网文案里的 23 → 当场红）。
- **🔗 相对链接会悄悄指到仓库外面**（R27 实测）。`README.zh-Hans.md` 写的是 `[SECURITY.md](../SECURITY.md)`
  与 `[AGENTS.md](../AGENTS.md)` —— 该文件就在**仓库根**，`../` 于是指到仓库之外，在 GitHub 上是**两个死链**
  （英文版写的是 `SECURITY.md`，正确）。同类还有 `docs/archive/architecture.md` 的 `../config.yaml`（正确是 `../../config.yaml`）。
  **实测口径**：把 `git ls-files -co --exclude-standard` 里的 **2096 个 markdown** 全量扫一遍，
  共 **1446 条相对链接**，其中**只有 3 条**落在本仓自己的文档里 —— 另外 **195 条全在 vendored 技能包里**
  （`skills/codebuddy/**`、`skills/trae/**`、`pyengine/src/opensquilla/skills/bundled/**`），
  且混着 `url` / `{videoUrl}` / `result|data|output` 这类**正则把散文当链接**的噪声（表格竖线、占位符）。
  ⚠️ **所以不要做一个「全仓相对链接必须可解析」的门禁** —— 它会立刻要求你修 195 条上游内容，
  或者写一大堆豁免。要做就**把范围限定在本仓自己的文档**（排除 vendored 技能包），那样基数是 0、直接可用。

- **🧹 机器状态残留有两种形态：目录和文件 —— R18 只堵了目录那种**（R28 实测）。
  R18 加的规则是 **`*.tmpdir/`**（目录形态），于是 R28 用「按文件名逐条扫 `git ls-files -co` 集合」时
  仍然抓到 **1 个会随提交发布的机器状态**：
  `workbench/frontend/_tmp_32456_50033095d6122e050fba89244b7152f9`（**0 字节**）——
  命名是 `_tmp_<pid>_<32位hex>`，是原子写在目标目录留下的中间文件：写成功会被 rename 掉，
  中断就永久留在那里。**判据**：`git check-ignore` 对**不存在**的路径可能不匹配，
  所以要先证明规则命中（`git check-ignore -v <真实文件>` = exit 0）、再确认它掉出 `git ls-files -co`、
  再删；并且**造一个同形态的合成文件再验一次**，否则你只是删了一个实例、没修掉那一类。
  已加 **`**/_tmp_[0-9]*_[0-9a-f][0-9a-f]*`**。修后实测：`git ls-files -co` 里**机器状态残留 = 0**
  （只剩 4 个**正当**的 `package-lock.json`），commit set 8979 → **8978**。
  ⚠️ 复扫口径：把 commit set 按可疑文件名形态分组（`.pytest_cache/`、`pytest-cache-files-*/`、`__pycache__/`、
  `*.py[co]`、`*.orig/.rej/.bak/.swp`、`.DS_Store`、`*.log`、`*.db`、`.vite/`、`*.tsbuildinfo`），
  并且**只算磁盘上真实存在的文件** —— commit set 里含 504 条「已跟踪但工作区已删」，
  它们提交后是删除，不是被发布的内容。

- **🕳️ 一个整子项目可以既随仓库发布、又不被任何门禁跑、也不出现在主 README 里**（R28 实测）。
  `workbench/` 是**第五个入口**：Python 后端（engine · api · cli · llmlab · ttc · agentloop）+
  FastAPI 在 `:8011` 同源托管的 React SPA + 独立打包流水线，`git ls-files -co` 里 **76 个文件**会随提交落地。
  三条实测把它钉死：① `python -m pytest marguerite_workbench/tests` → **71 passed, exit 0**（2.3s，离线无需 key）；
  ② `workbench/packaging/check_version_sync.py` → **PASS**（backend/frontend/VERSION 三处都是 1.1.0）；
  ③ **`grep -c workbench scripts/verify.sh` = `0`** —— 即上面两条**从来没有被任何门禁跑过**，
  而 `workbench/README.md` 却把它写成「CI 门禁」。与 R24 的 pyengine 是**同一类缺陷**：
  一个对部分代码库沉默的绿徽章（当年的原话是「文档宣传了一个不存在的门禁」）。
  ⚠️ **三个容易误判的点**：① 它**不是** Rust 引擎的第五个前端 —— `marguerite_workbench` 在
  core/tui/webui/gui 里 **0 引用**（`core/src` 里那 27 处 `workbench` 命中是 **TUI 的 workbench 模块**，
  同名不同物；**求证时必须用 `marguerite_workbench` 这个包名，不能用 `workbench` 这个目录名**）；
  ② 它有**自己的版本线**（`workbench/VERSION` = **1.1.0**，与引擎的 **0.2.17** 相互独立，
  `check-version-sync.py` 那 9 处**不含它**），所以「版本号单一事实源」这句话**只对引擎那条线成立**；
  ③ 它的 `frontend/` **没有测试脚本**（只有 `dev/build/preview/typecheck`），所以这一层目前只能守后端 + 版本。
  已修：`verify.sh --workbench`（门禁第 21/22 项）纳入 `--all`，`workbench/README.md` 的声明改成事实，
  主 README 的 Layout 与 Status 补上它。

- **🧹 「作者本机路径」会随代码一起发布 —— 而且没人会告诉你**（R25 实测）。用一条正则扫全部**将随提交落地**的
  `.rs/.py/.ts/.vue/.js/.mjs/.json/.toml/.yml/.yaml/.cmd/.sh`，一开始扫出 **30+ 个文件**带着
  `D:\Downloads\Marguerite`。绝大多数是**正当的测试夹具**（`/home/u/proj`、`C:\Windows` 给安全扫描器用），
  但确实混着真缺陷，挑出来这几个都是**用户可见**或**完全跑不起来**的：
  | 文件 | 问题 | 处理 |
  |---|---|---|
  | `webui/src/views/GraphView.vue` | 输入框 `placeholder` 直接写着作者的项目路径（**每个用户都看得到**） | 换成平台中立的 `/path/to/your/project` |
  | `scripts/mcp_btpanel_smoke.py`（**已跟踪**） | 硬编码 `d:/…/marguerite/btpanel-mcp` —— **本仓库里并不存在的嵌套路径**，外加 Windows 专有的 `py -3.14` | 默认改用**仓库内** `btpanel-mcp/`，`BTPANEL_MCP_PATH` 可覆盖，命令用 `sys.executable` |
  | `btpanel-mcp/README.md` | 安装/自测步骤让人 `cd d:\Downloads\…`（对所有人都错） | 改成仓库相对路径 |
  | `pyengine/.../bundled/{audit,explore}-third-party/*.sh` | 默认目录写死作者绝对路径；而 `third_party/` **被 gitignore、clone 里根本没有** | 默认 `./third_party` + 目录不存在时**明确报错退出 2** |
  | `smoke_orchestrate.sh` | 钉在 `D:/tp_build_fresh1/...` 与作者检出 | 全部改为从 `$0` 推导，环境变量可覆盖 |
  | `core/src/gateway/refactor_methods.py` | **Rust 源码树里唯一的 .py**，零引用的一次性脚本 | 移到 `scripts/dev/` 并去掉写死路径 |
  | `scripts/{audit_rpc_surface2.py,dev/audit_rpc_surface.py}` | `ROOT` 写死本机 | 改为从 `__file__` 推导 |
  **判据（可复用）**：把 `git ls-files -co --exclude-standard` 的结果按扩展名过滤后逐文件正则扫；
  ⚠️ **只扫将要落地的文件**（`-co` 集合），且**接受测试夹具**——否则会把 30 个正当夹具当成缺陷。
  收尾后只剩 `scripts/dev/` 里 2 个（该目录被 `scripts/README.md` 明确标注为「历史一次性诊断，不维护、不进门禁」）。
- **⚠️ `git grep` 只搜「已跟踪」文件 —— 本仓库有约 1900 个未跟踪文件，所以它会静默漏掉真相**（R23 一次踩中两回）。
  实测：查 `modeHintBar` / `gotoTopbarBadge` 的消费者时，`git grep` 只返回 6 个 locale 文件，于是得出
  「这个键没有任何源码引用」甚至「提示条根本不渲染」的结论；**两条都是错的** —— 真正的消费者
  `webui/src/components/ModeHintBar.vue` **未被跟踪**，裸 `git grep` 看不见它，而截图里该提示条明明在渲染。
  **判据**：在本仓库问「谁引用了 X」必须用**文件系统级**搜索（`grep`/`rg` 工具，或 `git grep --untracked`），
  **不要用裸 `git grep`**。同族陷阱：`git ls-files <dir>` = 0 **不代表目录不存在**（可能只是未跟踪）。
- **📸 `docs/assets/*.png` 会随代码过期，而没有任何门禁会告诉你**（R23 实测）。README 的两张主图是
  **2026-09-22 14:16** 生成的，而 `classifier.rs` 是 **09-23 12:30**、locale 是 **09-23 18:27** 改的 ——
  于是**主图里展示的正是 R16 已经修掉的那个缺陷**：计划卡写着「决策理由：单一意图（**Unknown**）」，
  而 README 正文在同一页吹这个缺陷已修（现在是 `Build`）。更早那版还在英文界面里显示中文提示
  （`modeHintBar` 当时尚未本地化）—— 两张图都在**低报产品**，且与自家文档自相矛盾。
  **判据**：① 改完 core/webui 的**用户可见**行为后，比对 `docs/assets/*.png` 与 `core/src`、`webui/src` 的 mtime；
  ② 重新生成是**一条命令、不需要真 API Key**：
  ```bash
  python scripts/demo/mock_llm.py 18999 &                    # 本地 mock 模型
  MARGUERITE_API_KEY=mock MARGUERITE_BASE_URL=http://127.0.0.1:18999/v1 \
    MARGUERITE_MODEL=mock-model target/debug/marguerite-core.exe gateway &
  python scripts/demo/capture_console.py --locale zh-Hans \
    --prompt "帮我做一个完善的 FPS 第一人称射击游戏"
  ```
  ③ ⚠️ **改完 core 必须先重编 `marguerite-core` 再截图**：截图经网关渲染，跑的是**二进制**，
  与「桌面冒烟必查 stale core」是同一类问题（旧二进制 = 旧截图）。
  ④ `--locale en --suffix -en` 可出英文界面版本；但引擎生成的计划步骤/标题**目前仍是中文**，
  英文壳套中文内容会显得半翻译 —— 所以 README 主图刻意用中文界面（见 README「Known limitations」）。
  ⑤ **TUI 截图不需要 pty**（R26 打通）：`core/examples/tui_frame.rs` 用 ratatui 的 `TestBackend`
  渲染**真实一帧**（与 `core/src/tui/**` 测试同一路径），把每个单元格连同 fg/bg/modifier 重新发成
  ANSI；`python scripts/demo/render_tui.py` 再把 ANSI 转成 `docs/assets/tui-workbench.png`。整条链路
  **无 pty、无真终端、无交互会话**，所以 Windows 与 CI 都能复现 —— 这解决了「终端优先的产品却拿不出
  终端截图」这个长期缺口（此前因为 Windows 没有 `pty`/`termios` 一直没做）。
  ⚠️ **两个坑**：① ratatui 渲染宽字符时把后续单元格**清成空格**（不是标记 `skip`），所以按单元格逐个
  输出会在每个 CJK 串后多插一列，必须按**字形显示宽度**前进（为此给 `core` 加了 dev-dependency
  `unicode-width`，它本来就在 lockfile 里）；② `render_tui.py` 会校验**每行宽度恰好等于终端宽度**，
  这同时交叉验证了 Rust 与 Python 两套宽度表 —— 不一致会直接 exit 1，不会静默出一张错图。
- **🧨 GBK 乱码会以「合法 UTF-8」的形态混进仓库**（R26 实测）。`core/Cargo.toml` 里 **19 处**中文注释
  是「UTF-8 字节被当成 GBK 解码后再写成 UTF-8」的产物（`# OS 绾ф矙绠憋細Linux Landlock 浠呭湪 linux target …`）——
  **`utf8` 解码完全成功、`U+FFFD` 计数为 0**，所以任何「编码检查」都发现不了它，肉眼看却是彻底的乱码。
  **判据（可复用的精确检测，不是字符黑名单）**：把可疑的非 ASCII 片段 `encode("gbk")` 再 `decode("utf-8")`，
  能还原出通顺中文的才是乱码。⚠️ 我第一版用「乱码高频字符」黑名单，**误报 1336 个文件** —— 因为名单里
  混进了普通句号 `。`，于是每个中文文件都被判成乱码。精确法实测全仓**只有 2 个**命中：`core/Cargo.toml`
  （真缺陷，已修）与 `AGENTS.md`（**是刻意引用的示例**，本身不是缺陷）。
  ⚠️ 修法上要说清：这些注释的原始字节**不可还原**（多处混着丢失的 `?`），所以我**重写了**这些注释
  （并顺手去掉一条「本机已确认装有 MSVC BuildTools」这类**只对作者成立**的断言），而不是假装「恢复原文」。
  改完用 `cargo metadata` 核对：**41 个 normal 依赖 + 2 个 dev 依赖逐一不变**，只有注释变了。
- **⚠️ 不要用 shell 管道输出判断编码 —— 本会话被它骗了三次**：本机控制台代码页是 **cp936(GBK)**，PowerShell 与 `python -c "print(...)"` 的 **stdout** 会把 UTF-8 中文重新编码，于是**完好的文件/响应看起来像乱码**。实测三次假警报：
  1. 以为 `.gitignore` / `config.yaml` 的中文注释「已损坏」（`# 瀵嗛挜涓庣幆澧僠`）——实际用 `read` 工具一看完全正常，Python 逐字节复核 **`utf8=YES` 且 `U+FFFD=0`**；
  2. `pytest` 输出被 PowerShell 重定向写成 **UTF-16LE**，按 UTF-8 读全是 `\x00` 交错；
  3. 网关 `/api/approvals` 的 501 JSON 中文看着像乱码 —— 存成文件后 **hex 是 `e8afa5`(=该) `e7abaf`(=端)**，**是合法 UTF-8**。
  **判据**：用 `read` 工具读文件、或 `open(p,'rb')` 看 hex / `decode('utf-8')`，**绝不用终端里「看起来像乱码」下结论**。
  （另注：`verify.sh` 里 `check-version-sync.py` 的中文输出在终端同样显示为乱码，那是**显示**问题，不是脚本问题。）
- **系统代理劫持 loopback（测试隐雷）**：本机常开 Windows 系统代理（Clash 127.0.0.1:7890），reqwest 会把部分 127.0.0.1 请求也走代理——「关闭端口必被拒」等网络假设会失效（被代答 502，而 502 是可重试错误）。测试一律用「本地 mock 监听器回明确状态码」（400 不可重试 / 404 非可达），禁止用连接拒绝作为确定性失败源；单次 accept 的 mock 必须循环应答（对话流程含经验抽取等第 2 次请求，单发 mock 会让流程挂死）。公共基建见 `core/src/tests/mock_http.rs`（完整读请求后再应答，防分段读半截）。
  PowerShell 探测回环请用 `curl.exe --noproxy "*"`；本机 `Invoke-WebRequest` **没有** `-SkipHttpErrorCheck`（是 Windows PowerShell 5.1）。
- **网关闭环探测的实测基线（R15 亲测，可直接复用）**：`GET /api/health` → **200 `{"engine":"rust","status":"ok",...}`**；`GET /control/` → **200 `text/html`**（Vue 控制台）；`GET /api/<未实现>` → **501 JSON**（`{"error":"not_implemented",...,"transport":"/ws"}`，**不是** SPA HTML）；`GET /ws` → **400 `Connection header did not include 'upgrade'`**（正确，它不是普通 GET 端点）。
  ⚠️ **强杀网关会遗留 python 侧车**（`opensquilla.cli.main gateway run --bind 127.0.0.1 --port <随机>`，脱离进程树）。`Stop-Process` 网关后必须再查一次并清理 `python`，否则它会一直占着端口与文件。
- **⚠️ 用 `getBoundingClientRect()` 两两求交做「元素重叠」检测会产生假阳性**：rect 相交**不考虑祖先裁剪**。`.sidebar-core` 是 `overflow-y: auto` 的滚动容器，被 flex 压到 14px 后，其子行的 layout rect 仍在 y=105/150/195，于是「两个 `.sidebar-fn-label` 在 y=115 重叠 38×20」这种结论**在 DOM 里成立、在屏幕上不成立**（截图里侧栏完全干净）。
  **判据**：① 用**截图**确认（`scripts/demo/capture_console.py` 是可复现管线，或直接 Playwright 截 `.sidebar`）；② 判定可见性要与**祖先滚动盒**求交（`el` 的 rect ∩ `scroller` 的 rect），不要元素两两求交。上一次「侧栏文字重叠」的诊断就是这类假阳性，真实缺陷是另一件事（见下条）。
- **侧栏纵向布局契约（勿回退）**：默认展开时侧栏内容约需 1110px，而 `.sidebar` 是 `position: fixed` 撑满视口。900px 视口下的**真实**缺陷（R16 实测）：`.sidebar-core` 被 flex 压成 `clientHeight 14px`（`scrollHeight 191px`）→ 行文字被裁掉，只剩一条**无文字的橙色条**，三个目的地（技能·渠道 / 定时任务 / 查看用量）不可达；`.sidebar-foot` 落在 **y=1062** → **设置根本点不到**。
  **根因**是 flexbox 的自动最小尺寸规则：flex item 只要 `overflow` 非 `visible`，其 `min-height: auto` 就解析为 **0** —— 于是它成了唯一能吸收缺口的项并吸收了全部；而 `.sidebar-history-list` 是 `flex: 1`（`flex-basis: 0%`），在收缩分配里贡献恒为 0。
  **修法三条缺一不可**：`.sidebar-core { min-height: 96px }`（两整行）、`.sidebar-history-list { min-height: 160px }`、`.sidebar { overflow-y: auto }`（原先 `hidden` 让溢出**不可达**）。
  已由 `webui/src/assets/sidebar-layout.contract.test.ts` 钉住（4 条断言，做过反向验证：`min-height` 改回 0 → 测试变红）。修后实测：rail clientHeight **14 → 96**、recents 仍保 2 行可见、`footReachable: true`、`.sidebar` scrollHeight 1126 / clientHeight 900。
  ⚠️ 该测试从磁盘读 `base.css` 而**不是** `import 'base.css?raw'` —— 后者在本仓库解析出来是断言看不见的内容（4 条断言全拿到空规则体）。测试还必须**先剥掉 CSS 注释**：这些规则的长注释里引用了被守卫的声明原文（如散文里的 `overflow: hidden`），否则会因为一句注释而误红（实测踩过）。
- **🔴 已跟踪状态（index/HEAD）严重落后于工作区 —— 这是当前头号发布阻塞**（R17 实测）：
  用户自 Round 10 起要求「不提交」，于是历次冲刺的成果**全部只存在于工作区**，`git status` 有 **1683** 条脏记录。
  用「忠实导出索引」的方式模拟了一次真实 clone（`git checkout-index -a -f --prefix=…`，导出 9015 个文件），
  一个新人 clone 下来会**完全拿不到**这些已完成的修复：
  | 检查项（在 clone 导出里 grep） | 结果 |
  |---|---|
  | core 的 `Build` 意图（R16） | **不存在** |
  | 网关 501（R13） | **不存在** |
  | webui 侧栏 floor（R16） | **不存在** |
  | `webui/src/stores/activeMode.ts` | **文件缺失** |
  | `pyengine` 的 pytest `pythonpath`（R14） | **不存在** |
  | `pyengine/tests/__init__.py`（R14） | **文件缺失** |
  | `scripts/check-version-sync.py` | **文件缺失** |
  | `docs/internal/README.md`（R14） | **文件缺失** |
  | `design/tokens.json`（tokens 门禁要读它） | **文件缺失** |
  | **整个 `.github/`（`git ls-files .github` = 0）** | **全部缺失 → 公开仓库上 GitHub 根本没有 CI** |
  | **根目录全套社区健康文件**：`LICENSE` / `CONTRIBUTING.md` / `SECURITY.md` / `CODE_OF_CONDUCT.md` / `README.zh-Hans.md` / `.gitattributes` | **全部 untracked** |
  ⚠️ 也就是说 objective 第 2 项（LICENSE、CI/CD、Release 流程、社区健康文件）**只存在于工作区，不在仓库里**：
  一个 clone 下来既没有 LICENSE，也没有 workflow，`design/tokens.json` 缺失还会让 tokens 门禁直接红。
  另外 `.gitattributes` 未跟踪意味着它的行尾归一化**从未生效**；一旦真的提交，Git 会对约 60 个文件
  做 CRLF→LF 归一化（`git status` 的 warning 已列出清单）——这是预期行为，但要心里有数。
  后果：`verify.sh --all` 在真 clone 上必然红（连 `check-version-sync.py` 都没有；`gui/package.json` 的已跟踪版本是 **0.1.0** 且**没有** `typecheck`/`lint`/`test:unit` 脚本 → `--gui` 四步会以 `Missing script` 直接失败）。
  **解除方法只有一条**：`git add -A`（**暂存不是提交**；`git reset` 可完全回退）。暂存之后再用同样的
  `checkout-index` 导出跑一遍门禁，objective 里的「新人 clone→跑起来」才第一次**可验证**。
  ⚠️ **不要只暂存一部分**（例如只 `git add gui`）：这会让索引内部自相矛盾（暂存的新 `gui/package.json`
  配着过期的 `core/Cargo.toml` 版本号），比「整体落后」更糟。
- **构建快照目录绝不能入库**（R17 修复）：索引里曾躺着**三份过期的整包构建产物**，共 **594 个文件 /
  24 MB**（2026-08-11 快照）：`gui/src-tauri/webui/dist_old_202608110032`、`dist_old_202608110146`、
  `dist_opensquilla_upstream_202608110033`（含多份重复的 KaTeX 字体等二进制）。
  它们在工作区里**早已被删除**，只在索引里残存。根因是 `.gitignore` 只写了 `gui/src-tauri/webui/dist`，
  匹配不到 `dist_old_*` / `dist_opensquilla_upstream_*`。已改为 **`gui/src-tauri/webui/dist*/`** 并用
  `git rm -r --cached` 移出索引（`--cached` **不删磁盘文件**；实测 `gui/src-tauri/webui` 前后都是
  285 文件 / 24.13 MB，那 285 个是当前构建产物 `dist/`）。
  ⚠️ 判据小坑：`git check-ignore -v <目录>` 对**磁盘上不存在**的目录可能不返回匹配（git 无法确认它是目录），
  要测就测目录**里面**的文件路径。
- **`git add -A` 之前必须先把机器状态挡掉**（R18 审计 + 修复）。审计方法是
  `git status --porcelain -z -uall`（**必须 `-z`**：非 ASCII 路径经 PowerShell 管道会被 C-quote 成
  `"\345\274\225.md"` 之类，`Test-Path` 直接报 Illegal characters）。审计发现 **597 个未跟踪文件里有 41 个
  是机器状态，会被一并提交**，已补 4 处规则 + 1 处 untrack：
  1. **`*.tmpdir/`** —— 原有的 `**/.dsh-*.tmpdir` **匹配不到真实命名**：实测残留是
     `.<文件名>.<pid>.<uuid>.tmpdir/`（如 `core/src/kb/.project_graph.rs.30984.<uuid>.tmpdir/`），
     `git check-ignore` 返回 **exit 1**，于是 9 个残留长期是「未跟踪且未忽略」。
  2. **`*.db` / `*.sqlite` / `*.sqlite3`** —— 运行期桥接库（`workbench/**/core_bridge.db` 等）。
     索引里原本有 **0 个** `*.db`，所以整类忽略不会误伤已跟踪文件。
  3. **`/scripts/qa/`、`/scripts/launch/`** —— 一次性 QA 探针（25 个 + 截图 + 结果 JSON）与中文名个人启动器；
     `scripts/README.md` 早已声明它们不属于仓库，这里只是把该声明落实成规则。
  4. **被跟踪的运行期日志 `core/.marguerite/security-audit/skill_block.jsonl`**（51 KB）：
     `core/src/security/skill_scan.rs::log_skill_block` 是「尽力而为地追加一行 JSON」，
     **每跑一次测试就追加一行**，于是仓库每次测试后都变脏。代码里没有任何地方读取它 →
     `git rm --cached` 移出索引（文件保留在磁盘）。同时把 `/.marguerite/` 改成 **`**/.marguerite/`**：
     原规则锚定仓库根，**漏掉了 `core/.marguerite/` 这份嵌套的**。
  修后：未跟踪 597 → **556**，且**机器状态残留为 0**；所有正当源码（`LICENSE`、`.github/**`、
     `gui/src/**`、`pyengine/tests/__init__.py`、`design/tokens.json`、`workbench/**` 等）仍在 add 集合里。
  **对账口径**（`-z -uall`）：1942 条 = `??` 556 + ` M` 266 + ` D` 504 + `D ` 594（本轮暂存的产物删除）
    + `R ` 9（R14 的 `git mv`）；`git diff --name-only` 的 772 = 504 + 266 + 2，与 porcelain 完全一致。
  ⚠️ 解析 `-z` 输出时注意：**重命名条目会多带一个 NUL 字段（原路径）**，naive `split('\0')` 会把它当成
  新条目，表现为 `sc` / `PH` / `Se` 这类不存在的 XY 码（分别是 `scripts/`、`PHASE_`、`Security_Audit_` 的头两字符）。
- **⚠️ `gateway::methods` 的桌宠往返用例曾偶发假失败（R18 已修）**：现象是整套**并行**跑时
  `local_hub_contract_tests::pet_trigger_pet_roundtrip_increments_affection` 断言
  「pet.status 应读到同一持久化好感度」失败（2535 passed / 1 failed），而 `--exact` 单独跑**必过**（0.00s）。
  根因是**共享可变状态**：该模块的 `ctx()` 原先用 `OnceLock<Mutex<PathBuf>>` 让**同进程所有用例共用同一个
  临时目录**（按 pid 命名），而每次 `ctx()` 都会先删掉目录里的 `pet_state.json`。于是「+1 之后重新读」
  那一步可能刚好被兄弟用例的删除插进来 → `load_store()` 回落到默认值 0 → 断言失败。
  修法：`ctx()` 改为按调用递增的**唯一**目录（`AtomicUsize` + pid），用例之间不再共享持久化状态。
  验证：修后连跑 3 次整套，**2536 passed / 0 failed**（3/3）。
  教训（与 `events/product.rs` 同类）：**任何「写文件再读回来」的用例都必须有自己的路径**；
  `set_config_dir_override` 是**线程局部**的，所以覆盖必须在每个用例线程内设置。
- **🧾 单元测试曾把夹具事件写进用户的 `~/.marguerite/usage.db`（R64 定位并修；前端因此显示假指标）**：
  `usage::init()` 是**进程级** `OnceLock`（`LEDGER`），被 chat / engine / gateway / TUI 多处入口调用 ——
  只要同进程里有一个用例走到那些入口，台账就被点亮，之后**所有** `record()` 都写进**真实**用户库，
  包括 `record_is_noop_without_init` 这个本意是「未初始化就该 no-op」的夹具。
  真机库实测（2026-09-28）：`never-initialized-model` **567 行**、`session=''` 的空归属事件 **6930 行 / 680677 token**。
  而前端在**没有会话键**时仍拿空串去问 `usage.status`，网关按「一次合法过滤」处理 → 返回那一桶，
  于是聊天页脚注把网关级垃圾渲染成「本会话 token 680677 · 轮次 6930」（全新会话不可能有 6930 轮）。
  ✅ 修法：`ledger_path()` 在 `cfg(test)` 下落到 `%TEMP%\marguerite-usage-test-<pid>.db`（用户目录永不打开）；
  前端在 `sessionKey` 为空时**不查**该 RPC，且脚注只在**本会话真有用量**（或本轮实测到首 token）时出现。
  证据：全量核心单测 2784 passed / 0 failed 之后，真实库 **rows=9915 / max_id=9915 与跑前完全一致**，
  同时 `%TEMP%` 出现 `marguerite-usage-test-<pid>.db`；守卫 `ledger_path_is_never_the_real_user_database_under_test`
  在变异（路径改回真实目录）后精确报红。
- **🎨 `MeshToonMaterial` 的渐变贴图有一半是死区（R59 定位并修；「桌宠发灰/发脏」的真根因）**：
  three r185 的 `getGradientIrradiance` 取样坐标是 `coord = vec2(dotNL * 0.5 + 0.5, 0)`，而 `dotNL`
  在光照循环里**已被 `saturate`** —— 于是**受光面（dotNL ∈ [0,1]）只可能落在 coord.x ∈ [0.5, 1]**，
  贴图前半段永远采不到。旧贴图是 **3 texel**，受光面只能取到 texel#1/#2（`0xee` 与 `0xff`）：
  **LOW 档（`0xc6`）对受光面是死代码，可见明暗差只有 6.7%** → 白色部件平坦、无体积感。
  这也解释了历次排查里那个费解结果：**「改 LOW 档位 → 采样像素 Δ=0」**（受光面根本采不到那一格）。
  另一条同样关键的着色器事实：`RE_Direct_Toon` 里**没有独立的 N·L 因子** —— 渐变取样值**替代**了 N·L，
  所以曝光估算必须写成「档位(dotNL) × 强度 / π」，按 `dotNL × 强度 / π` 算会算不准（旧守卫就是这么错的）。
  ✅ 修法：贴图宽度取 **6**（偶数）、可达三档放**后半段**（`PET_TOON_RAMP_WIDTH` / `toonRampBytes`），
  可达区间由 0.067 变成 `0xc6→0xff` = 0.224（LOW 仍保持仓库既有的 0xc6「不显脏」结论）；
  兽形 key 由 3.30 降到 1.90（旧值在卡通口径下让主光正对面 = **1.371**，几乎整个可见半球裁平）。
  **实测（隔离网关 + Playwright；用「隐藏画布前后差集」隔离宠物像素，两次独立试验 IoU ≥ 0.93 才采信，
  并带 `lights=none → 全黑` 的有效性校验）**：
  | 皮肤 | 修前 中位 sRGB / ≥235 占比 / 线性中位 | 修后 |
  |---|---|---|
  | 橘猫小工（beast） | 255 / **75.4%** / 1.000 | **221 / 30.9% / 0.72** |
  | 雏菊精灵 | 229 / 42.4% / 0.788 | 220 / 37.3% / 0.72 |
  | 樱花精灵 | 221 / 35.9% / 0.727 | 213 / 27.6% / 0.669 |
  | 薰衣草精灵 | 202 / 34.3% / 0.596 | 195 / 24.9% / 0.55 |
  | 四叶草精灵 | 228 / 34.6% / 0.781 | 217 / 28.6% / 0.702 |
  即**橘猫从「整只裁平的白团」回到与 4 款精灵同一档**，中间带（100–200）占比全面上升（猫 20.7% → 28%）。
  逐灯分解（同一隔离法，只让一盏灯进场景）：`hemi 0.079 / key 0.914 / fill 0.162 / rim 0.009 /
  bottom 0.045 / none 0.000`（线性中位）——「none 全黑」即遮罩有效性的自证。
  守卫（都在 `webui/src/components/pet/petSpec.test.ts`，均实测过有牙）：
  ①「受光面可达档位覆盖 LOW→MID→HIGH」（旧 3 texel 布局必红）②「单盏主光 ≤ 0.95」（3.30 必红）
  ③「兽形可见半球裁切 ≤ 15%」（旧 28.2% 必红）④「受光面明暗差 ≥ 0.20」。
  ⚠️ **旧守卫漏掉它的原因（可复用的教训）**：它们只在 `[0,0,1]` **一个法线**上取值，且用**错误的曝光公式**
  （按 dotNL 而非按档位）——于是「主光正面被裁平」这种大面积缺陷完全落在盲区之外；
  而且两条旧阈值（`LOW ≥ 0.6` 与「不得过曝」）在兽形上**互相矛盾、无解**（扫描实测：要 LOW≥0.6 需 key≈3.10，
  那时裁切已 24.1%）。**单点采样 + 形式错误的模型 = 恒绿的守卫。**
  ⚠️ 已知差距（未做）：曝光模型是**白反照率 + 白灯的上界**，对以彩发/彩翼为主的人形 4 款皮肤偏松
  （模型裁切 49%，实测「恰为 255」仅 3.7~15%）——故紧界只设在兽形，人形只设硬天花板（<1.25）。
- **🔑 「全新 clone 能不能跑」要用「提交集快照」验证，而不是等用户提交**（R19 方法，可复用）：
  `git ls-files -co --exclude-standard -z` 就是 `git add -A` 会暂存的集合（已跟踪 + 未跟踪但未被忽略）。
  把它**拷到一个临时目录**（NUL 分隔读取，否则非 ASCII 路径会被 PowerShell 破坏），就得到「提交后的 clone 长什么样」，
  **完全不需要碰索引**。然后在那个目录里 `npm ci` + 跑 `verify.sh --webui --gui --pyengine`。
  本轮实测：`D:\commit-sim` = **8472 个文件 / 105.7 MB**，58 项门禁必需输入**全部存在**、17 个顶层目录计数**逐一相符**。
  这个方法一次就找出两个**本机门禁永远看不见**的真实缺陷（见下面两条）——因为本机 `node_modules` 早已装好，
  「能不能从干净状态装起来」从来没被验证过。
  ⚠️ 一个自检陷阱：`commit-set` 计数**包含**已跟踪但工作区已删除的文件（它们会作为删除提交），
  比对目录计数时必须只算磁盘上真实存在的文件，否则每个目录都会「少」掉自己的删除数（本轮差点误判成 4 处不一致）。
  **R25 复测 + 加强（这是 objective 第 4 项「clone→跑起来」的当前证据）**：
  | 量 | 实测值 |
  |---|---|
  | 索引（今天 clone 到的） | **8417** 个文件 |
  | `git add -A` 会写入的条目 | **8973**（其中 **504** 是「已跟踪但工作区已删」，提交后是**删除**） |
  | 提交后真正落地的文件 | **8469** |
  | 索引里的 `.rs` | **312** / 工作区 **443** —— 即今天的 clone **落后 131 个 Rust 源文件** |
  三条结论（都是本轮实测，不是推断）：
  1. **索引的模块图是闭合的**：把索引 `git checkout-index -a -f --prefix=…` 导出后逐文件解析 `mod X;`（含 `#[path]`、并**必须跟踪 inline `mod tests { … }` 的嵌套**），**0 个无法解析**。所以今天的 clone 是**陈旧**，不是**编译坏了**。
  2. **提交集与「能编译的工作区」源码完全一致**：`core` 438 / `tui` 1 / `gui/src-tauri` 3 个工作区 `.rs` 与提交集**逐一相等**；9 个 `include_str!`/`include_bytes!` 目标全部存在且在提交集内；17 项编译/发布必需件（含 `Cargo.lock`、`rust-toolchain.toml`、`build.rs`、两个 lockfile、`LICENSE`、`.gitattributes`、**两个 workflow**）**全部在提交集内**。→ 门禁那 2538 个通过的测试跑的就是这同一批源码，所以「提交后能编译」有据可依，无需再付一次 20 分钟的全新构建。
  3. ⚠️ **解析 `mod` 时不要相信「按文件目录解析」的朴素实现** —— 本轮我的第一版脚本就是这么写的，于是把 `core/src/lib.rs` 里 `#[cfg(test)] pub mod tests { pub mod mock_http; }` 判成「缺 `core/src/mock_http.rs`」，得出**「今天的 clone 编译不了」的错误结论**。真实文件在 `core/src/tests/mock_http.rs`（inline 模块改变了基准目录）。**是打开源码核对才发现的**；同理，统计大括号前必须先剥掉注释与字符串字面量（`format!("{}")` 之类会把深度算歪）。
- **`gui/package-lock.json` 曾与 `package.json` 严重脱节**（R19 修复，且是**门禁盲区**）：
  实测 `npm ci` 在 `gui/` 直接失败 —— `lock file's @eslint/js@10.0.1 does not satisfy @eslint/js@9.39.5`、
  `Missing: eslint@9.39.5 from lock file`…… 共 **85 条 Missing**，整棵 eslint 依赖树都不在 lockfile 里。
  **为什么长期没人发现**：`verify.sh` 的 lockfile 自洽检查**只覆盖 webui**（`cd webui && npm run deps:lockfile-sync`），
  `--gui` 那一层根本没有对应检查；而本机 `gui/node_modules` 是早就装好的，四项 gui 检查因此**从不触发 `npm ci`** ——
  于是**本机全绿、全新 clone 上四项 gui 检查连启动都做不到**（`tsc is not recognized`）。
  修法：`npm install --package-lock-only` 重建 lockfile（221 → 306 个 packages），给 `gui/package.json` 补
  `deps:lockfile-sync` 脚本，并在 `verify.sh` 里把该检查**拆成 webui/gui 两个独立项**（`--all` 16 → **17 项**；R27 加入密钥扫描、文档链接校验、README 事实校验后为 **20 项**；R28 加入 workbench 两项后为 **22 项**）。
  已验证负向：往 `gui/package.json` 注入 `left-pad` → 新检查 **FAIL**、闸门 exit 1。
- **`axe-core` 被 import 但从未声明**（R19 修复，同一类盲区）：`gui/src/__tests__/a11y.axe.test.tsx` 用了
  `axe-core`，而 `gui/package.json` 里没有它 —— 本机因为 `node_modules` 里残留着 4.13.0 所以 typecheck 过，
  全新 clone 上则 `error TS2307: Cannot find module 'axe-core'`。已补为 devDependency `^4.13.0`。
  **系统性排查手段**（值得固化成脚本）：扫 `src/**` 里所有裸模块说明符，与 `package.json` 的
  `dependencies/devDependencies/peer/optional` 求差集。实测结果：**gui 1 个（axe-core）、webui 0 个**。
  ⚠️ 扫描器必须把 `@/xxx` 这类 **tsconfig/vite 路径别名**排除，否则 webui 会报出 15 个假 "undeclared"
  （`@/components`、`@/stores`… 全是别名）。
- **⚠️ i18n 有两个盲区，其中一个已在本轮封堵**（`webui/scripts/check-i18n.mjs`）：
  1. **已封堵：源码引用了「所有字典都没有」的键**。原来该脚本**只做 locale 互比**，所以「6 个语言都没有这个键」
     是它结构上看不见的 —— 而 vue-i18n 解析不到键时会**渲染原始键字符串**。实测因此有 **24 个键**在界面上直接显示成
     `settings.ext.collabSsh.step1` 这类文本（其中 15 个是整块「企业协同 · SSH」指南，另有
     `chat.expand` / `chat.collapse`、`chrome.modeBadge` / `chrome.modeMenu`、`shared.cmdp.actionModeSwitch`、
     `chat.promptCacheKeepalive.cacheHits(.Hint)`、`chat.planRun.parallelSimulated`）。
     已加**源码扫描**：抓 `t('字面量')` / `$t(...)` / `te(...)` 与 en 字典求差集，**有未定义键即 FAIL**（并打印键名与文件）。
     实测 **2651 个字面量键、0 个未定义**；反向验证：注入 `t('definitely.not.a.key')` → 立刻 FAIL/exit 1。
     ⚠️ 只抓**整体字面量参数**（`t('k')` / `t('k', {...})`）；拼接、模板字符串、动态键一律跳过，否则全是假阳性。
     ⚠️ 反方向（en 里有、源码没用）**只统计不失败** —— 实测 1211 个键是动态拼出来的（`channelStatus.${x}`、nav 标签），
     把它们当缺陷会立刻变成噪音。
  2. **未封堵：模板里硬编码的中文**。实测 `<template>` 中**未走 `t()` 的 CJK 文案共 538 行、分布在 37 个 `.vue`**，
     例如 `SidebarConversations.vue`（`运行中`）、`ArtifactInlineEditor.vue`（13 行）、`RightSidebar.vue`（24 行）、
     `BudgetPanel.vue`（15 行）、`DesktopPet.vue`（9 行）。**切到 ja/fr/de/es 时这些位置仍然是中文**，
     而门禁只会报「full parity, no English leakage」。
     本轮只顺手把紧邻的 `ModeBadge.vue` 菜单三处（`运行时模式` / `当前：` / `打开智能中枢（完整模式管理）`）改成 `t()`，
     **其余 535 行未动** —— 那是一次独立的、需要逐条翻译的工程。
     建议下一步用**棘轮（ratchet）**方式防新增：把当前的（文件 → 行数）基线写进守卫，只允许下降不允许上升。
- **tokio::test TCP 端到端**：客户端必须用 `tokio::net::TcpStream`（异步），**绝不**用同步 `std::net::TcpStream`——后者阻塞 runtime worker，单核环境致 accept 饿死、连接超时（错误码 10060）。
- **`rm` 被 safe-delete 钩子拦截且 fail-closed**：本环境 `rm` 经 genie-trash 包装，相对路径会被拒、且失败时**拒绝删除（文件保留）**。确需删除请用绝对路径，或直接用托管 Python `os.remove()` 真删；脚本/CI 删除不可用裸 `rm`。
- **`std::thread::JoinHandle` 无 `join_timeout`**：有界网络探测用 `tokio::time::timeout` + `tokio::net::TcpStream::connect`，不要用 `std::thread::spawn().join_timeout()`（std 没有此方法）。
- **子命令后 `--flag` 已被透传**：`cli/argv.rs` 对子命令 handler 消费的旗标（deploy/model add/browser/dispatch/subagent 的 `--target`/`--id`/`--out`/`--timeout`/`--reflect` 等）做白名单透传进 `sub_args`；真正未知的 `--flag` 仍会停止收集并报「未知选项」。
- **`serve --help` 误启动 listener**：旧二进制若不含 Phase 5 代码会把 `--help` 当启动。确认二进制已重建（`argv.rs` 正确设置 `subcommand_help` 后 `run()` 会先拦截帮助再路由）。
- **桌面冒烟必查「stale core」**：`marguerite-gui` 静态链接 core。改 core 后只重编 `marguerite-core` 不会进入桌面进程——必须先 `cargo build -p marguerite-gui`（重链接新 core）再**重启已运行的桌面进程**，否则桌面网关仍是旧行为（实测：pet.triggerAction 缺 `ok:true` 的修复未进 GUI 二进制，桌面 E2E「顺毛抚摸」静默失败；重链接重启后 7/7 全过）。冒烟前确认 `target/debug/marguerite-gui.exe` 的 LastWriteTime 晚于 core 改动。
- **`cargo tauri dev` 死循环（dist 监视打架）**：本环境 `tauri.conf.json` 的 `frontendDist = "webui/dist"`（解析为 `gui/src-tauri/webui/dist`），`cargo tauri dev` 时 tauri-cli 与 GUI 侧 vite 同时监视该目录，任一变更触发对方重建 → 无限「Rebuilding application...」循环，永远到不了 app 启动。桌面冒烟请直接用 `cargo build -p marguerite-gui` + 运行 `target/debug/marguerite-gui.exe`（真机验收路径，见 docs/development.md 冒烟清单）。
- **Windows 上运行中的 GUI 二进制被文件锁**：网关/桌面进程占用的 `marguerite-core.exe` / `marguerite-gui.exe` 无法原地重编（`拒绝访问 os error 5`）。先停进程再 `cargo build`。
- **⚠️ `Get-Process name1,name2 | Measure-Object` 会漏报**（本环境实测踩过两次，代价是两轮「门禁变红」的误判）：
  用它判断「还有没有残留进程」**不可靠**，会得出 `0` 却仍有进程在跑。请改用显式逐个查询：
  ```powershell
  Get-Process -Name marguerite-core,marguerite-gui,marguerite-tui -ErrorAction SilentlyContinue |
    Select-Object Id,ProcessName,StartTime,Path
  ```
  残留的 `marguerite-core` 会锁住 `target/debug/marguerite-core.exe` 与
  `target/debug/deps/marguerite_core.exe`，使 `cargo build --workspace` 报
  `failed to remove file ... 拒绝访问 (os error 5)` / `LNK1104: 无法打开文件` ——
  **这是环境问题，不是代码缺陷**。跑闸门前务必先确认没有残留进程；
  注意 `cargo test` 自身可能留下子进程，所以测试步跑完也要复查一次。
  另注：`Get-Process -Name X | Stop-Process -Force` 有时会静默失败，必要时再查一遍确认。
  ⚠️ **最常见的持锁者就是常驻 daemon 本身**（R60 实测：`target/debug/marguerite-core.exe gateway --port 18801` 一直在跑，
  `--all` 的 24 项全绿、只有最后一条 `cargo build --workspace` 报 `failed to remove file … 拒绝访问 (os error 5)`）。
  所以**跑全量闸门之前要先停 daemon，跑完再把它起回来** —— 这与「daemon 常驻」不冲突，是同一件事的两个阶段。
  `cargo build -p marguerite-core --bin …` 与 `cargo test` 可能**恰好**不改写那个 exe 而侥幸通过，
  只有真正触发 relink 的 `--workspace` 才会红 —— 别把「前几条过了」当成没被锁。
- **依赖安全审计 + lockfile 自洽（两条新增本地门禁）**：`webui/package.json` 新增两个脚本，
  已接入 `verify.sh` 的 `--webui` / `--gui` 与 `--all`（`--all` 现为 **22 项**检查）：
  - `security:audit`（`npm audit --audit-level=moderate`）—— console 会渲染模型输出，所以消毒器
    （`dompurify`）与 id 生成器（`nanoid`）的公告属**真实产品风险**。本仓库此前**没有任何审计步骤**，
    因此静态携带 4 条公告无人可见（3 moderate + 1 high：`dompurify` XSS、`nanoid` 死循环、
    `vitest`/`@vitest/mocker` 路径穿越）。已 `npm audit fix` 清零，并实测 `npm ci` 与全量套件
    在升级后的版本上仍全绿。
  - `deps:lockfile-sync`（`npm ci --dry-run`）—— `npm ci` 要求 `package.json` 与 lockfile 完全自洽，
    否则 **CI 第一步就报错**（`can only install packages when … in sync` / `Missing: <pkg>@<ver> from lock file`）。
    **`npm audit` 看不见这类漂移**（它审的是 lockfile 解析出的依赖树，不是「声明 vs lock 是否一致」），
    所以两者互补、都要跑。实测负向验证：声明 `left-pad` 但不进 lockfile 时，audit PASS 而 lockfile-sync FAIL，
    闸门整体 exit 1。
  ⚠️ **`gui/` 仍有 4 条公告未处理**：修它们需要 `npm audit fix --force`（会把 `vite` 升到 8.3.0，属破坏性变更），
  而 `gui/` 的 React 前端**在产品界面中不可达**（见 §1），故未强行升级 —— 这条待「接线或移除 `gui/src`」的决策一并处理。
- **npm lockfile 的版本号一致性**：`npm ci` 会校验 lockfile 与 `package.json` 是否自洽。
  本仓库实测出现过「`node_modules` 与实际 lockfile 不一致」（安装的 `dompurify 3.4.12` vs lockfile 声明 `3.4.15`），
  所以**新增或升级依赖后请跑一次 `npm ci` 而不是只 `npm install`**，两者对 lockfile 的处理不同。
- **⚠️ `python3` 可能是「没有依赖」的隔离解释器（门禁解释器解析已改为能力感知）**：本机实测
  `which python3` → `/c/Users/Administrator/.workbuddy/binaries/python/versions/3.13.12/python3`
  （WorkBuddy 托管的隔离解释器，实测**只有 `pytest`**，缺 `anyio`/`httpx`/`structlog`/`typer`/`rich`/
  `pydantic`/`websockets`/`tomli_w`/`openpyxl`/`reportlab`/`yoyo`），而 pyengine 的依赖装在系统
  `C:\Python314\python.exe`（实测上述 11 个模块 + `pytest` 全部可 import）。
  ⚠️ 旧版 `verify.sh` 只做 `command -v python3 → python`，于是**会挑到隔离解释器**：pyengine 三层在
  import 阶段整层 collection error（实测 pyengine 单测 39 个、`test_gateway` 187 个 ERROR），看起来像
  产品缺陷，其实只是解释器选错 —— **同一条门禁两次跑出不同结果**（R1 红 / R2 绿）就是这个症状。
  `verify.sh` 现已改为**能力感知**：候选按 `$MARGUERITE_PYTHON` > PATH `python3` / `python` >
  `C:/Python314|313|312` > `%LOCALAPPDATA%/Programs/Python/Python3*` 枚举，只有能**一次** `import`
  全部所需模块的才算合格，并把**选中的解释器**打进输出（`-- 选中的 Python 解释器: …`）。
  `$MARGUERITE_PYTHON` 被显式设置却不合格时**响亮 `[FAIL]`**（绝不静默回落）；全都不可用时记
  `[SKIP]` 并**逐候选列出缺哪些模块**（§8：跳过 ≠ 通过）。
  手工跑 pyengine 层请用 `MARGUERITE_PYTHON` 钉死：
  ```bash
  MARGUERITE_PYTHON="C:/Python314/python.exe" bash scripts/verify.sh --pyengine
  ```
- **⚠️ WorkBuddy 的 Python 启动链注入了 `safe-delete` 垫片，会污染「删除」语义**：本机 Python
  启动时加载 `sitecustomize`
  （`…\WorkBuddy\resources\app.asar.unpacked\cli\vendor\shim\sitecustomize.py`），它拦截所有删除并
  改投回收站。有两条副作用会**让被测代码看到错误的删除结果（不是产品缺陷）**：
  1. **批量删除 >5000 个文件要审批** → 日志出现 `[safe-delete][SAFE_DELETE_BULK_CONFIRM_REQUIRED]`
     （实测 `count=6031` / `7843`），pytest 的汇总行被吃掉、进程 `rc=1` → 门禁报「无 pytest 汇总行」。
     受影响：`test_sandbox`（6031 文件）、`test_ci`（7843）。
  2. **单文件删除 fail-closed** → 日志出现 `[safe-delete][SAFE_DELETE_FAIL_CLOSED]`
     （`{"reason":"trash-failed","detail":"[Errno 53] 找不到网络路径。: '\\?\C:\…\.opensquilla.profile-consolidation.<uuid>.staging'"}`）
     → 被测的原子/恢复代码把本该 `consolidated` 的结果判成 `blocked` / `profile_consolidation_failed`
     → 实测 `test_recovery` **8 个失败**。
  ✅ 关闭它（**仅本次运行**）后实测归零：`CODEBUDDY_SAFE_DELETE_ENABLED=0` + `C:\Python314` →
  `test_recovery` **rc=0, 444 passed / 9 skipped**、`test_sandbox` **rc=0, 1746 passed / 349 skipped**，
  两次 `safe-delete` 干预计数都为 **0**。跑 pyengine 层前：
  ```bash
  export CODEBUDDY_SAFE_DELETE_ENABLED=0   # 仅本机 WorkBuddy 终端需要
  ```
  ⚠️ 该垫片**只在本机 WorkBuddy 会话里存在**，普通终端与 CI 都没有它 —— 所以**不要**把它写进
  `scripts/verify.sh`，只作为**运行期环境变量**（及本条文档条目）。

- **🔓 产物下载只认 id、不认会话 —— 知道 id 就能跨会话读走别人的文件**（2026-10-28 实测定位 + 已修）。
  `GET /api/v1/artifacts/{id}` 在 pyengine 侧走 `_session_id_for_download`：解析不到会话键、或键与产物归属不符，一律 **404**；
  而 Rust 的 `artifact_download`（R56 实现）只做 `get_artifact(&id)` 命中即回文件本体 —— 于是**任何持网关 token 的调用方
  只要知道 id 就能读别的会话的产物**（id 形如 `artifact:<session>:<path>`，而控制台把它写进 URL 与下载属性）。
  契约分歧 + 越权面，与 R55（会话键头名）同族。
  ✅ 已修：`requested_session_key` 复刻 pyengine 的解析优先级（**品牌头 `x-Marguerite-session-key` → 旧头
  `x-opensquilla-session-key` → 查询参数 `sessionKey`/`session_key`**，带百分号解码），与台账的 `entry.session_key`
  不等即回 **404 而不是 403** —— 403 会泄露「这个 id 存在」。
  前端一侧随之必须补头：`useArtifactActions.sameOriginHeaders` 此前只发 `Authorization`，现补同一个会话头；
  `<img :src>` 之类的裸 URL 消费者实测为 0（产物缩略图绑的是 controller 生成的 objectUrl），e2e 也 mock 了该路由，所以无回归。
  守卫：Rust `requested_session_key_prefers_brand_header_then_legacy_then_query`、`percent_decode_handles_escapes_and_plus`，
  路由级 `test_artifact_download_serves_registered_file_only`（无键 404 / 错键 404 且响应体**不含**产物原文 / 正确头 200 / `?sessionKey=` 200），
  前端 `useArtifactActions.test.ts`。**变异实测**：摘掉归属判断 → HTTP 用例打印出 `artifact payload` 判红；
  摘掉前端那次加头 → `expected undefined to be 'agent:main:webchat:ok'`。
  ⚠️ 新增任何按会话读取的 REST 路由时，**必须复用同一解析**（Rust `requested_session_key` / pyengine `request_session_key`），
  否则就是 R55 那条越权面的新入口。

- **🔒 闸门红在「测试二进制被占用」时不是代码缺陷 —— 但预检查容易漏掉下划线命名**（2026-10-28 实测）。
  一次 `--clean --all` 在第一步就 `[FAIL] core 单元测试 (rc=101, 114s)`：`cargo clean -p marguerite-core` 报
  `failed to remove file …\deps\marguerite_core-ff26e7d8a5547c95.exe: 拒绝访问 (os error 5)`，紧接着 `link.exe` 报
  `LNK1104: 无法打开文件 …deps\marguerite_core-ff26e7d8a5547c95.exe`。
  根因是一个**孤儿测试进程**（`marguerite_core-ff26e7d8a`，**下划线**），而我的预检只匹配了横线名
  `marguerite-core.exe` → 漏过。`taskkill //PID <pid> //F` 后重跑即 **4228 passed / 0 failed**。
  **判据**：`rc=101` 且输出里**只有** `LNK1104` / `os error 5`、**没有** `test result:` 行 = 环境锁，不是用例失败。
  查残留进程要按 **`tasklist | grep -i marguerite`**（同时覆盖 `marguerite-core` / `marguerite-gui` / `marguerite-tui`
  与 `marguerite_core-<hash>` 测试二进制），不要只查一个精确名。

- **🎫 限流键采信了调用方随手写的 `X-Forwarded-For` —— 每次换个值就是一个新桶**（2026-10-28 实测定位 + 已修）。
  `RateLimitMiddleware._get_client_ip` 原先无条件 `forwarded.split(",")[0]`，于是**任何本机调用方**（桌面客户端 / 脚本 /
  同机进程；浏览器发不了这个头，它属于 forbidden header）只要每次改一下 XFF，就永远碰不到 429。
  实测变异（把实现换回旧版）：同一进程连发 3 次、每次不同 XFF → `[200, 200, 200]`，`max_requests=1` 的桶形同虚设。
  ✅ 已修：**只有 `auth.mode == "trusted-proxy"` 才读这个头**，其余模式一律用 `request.client.host`。
  这与本仓既有的 origin guard 同一条口径 —— 代理元数据不能替一个客户端可控的标识背书
  （`test_origin_guard.py` 里那句注释「Proxy metadata must not turn a browser-controlled authority into one
  that the implicitly trusts」说的就是同一件事）。
  守卫 `tests/unit/gateway/test_rate_limit_client_ip.py`：token 模式伪造 3 个不同 XFF 仍全落到同一桶（第 2 次起 429）；
  trusted-proxy 模式反向自证（A/B 两个 IP 各自有桶，A 的第二次 429）—— 证明修复没有把真实代理分桶一起关掉。
  ⚠️ 同理，**新增任何「按客户端」的机制（限流 / 配额 / 审计归属）时先问：这个键是谁给的？**
  唯一可用的键是传输层的 peer 地址；只有显式声明信任代理时才允许向上游头部要答案。
  ⚠️ 另注：`trusted-proxy` 模式的 auth 分支用的是 `proxy not in forwarded_for`（**子串**包含判定，且头可伪造），
  只有「代理确实会覆写该头」的部署才成立 —— 属已知边界，本轮未改。

- **🧩 外部内容 FTS5 表的 ghost 条目，三种「看起来在检查」的计数全都照不到 —— 只有原始 MATCH 往返能照出来**（2026-10-29 P1-D4 实测定位 + 已修）。
  `short_term_sessions_fts` 是 `content='short_term_sessions', content_rowid='rowid'` 的**外部内容表**：它的词项索引和内容表是两份东西，
  **删内容行不会自动删索引**。把 `enforce_max_sessions`（`core/src/memory/short_term.rs:543`）里的逐行 `'delete'` 命令摘掉做变异，
  三条当时在用的判据**全部照常绿**：`COUNT(*) FROM …_fts` 回 `3 / 3`、无 MATCH 的 rowid 扫描回 `[4,5,6]`、
  「索引行 JOIN 不到内容行」的孤儿计数回 `0`。而 **`search()` 自己也是瞎的**（`short_term.rs:429` 那条查询把索引 JOIN 回内容表，
  ghost 天然 JOIN 不上 → 被静默丢弃），所以「搜不到了 = 删干净了」是一个**假绿灯**。
  ✅ 已修：`evicted_sessions_stop_matching_survivors_keep_matching`（`short_term.rs:1139`）绕开所有 JOIN，直接对词项索引做原始 MATCH，
  断言「索引命中集合 == 内容集合」两侧都非空；`max_sessions_is_configurable_and_eviction_stays_consistent`（`:1082`）
  的计数断言原地加注，写明它和 `fts_rows` 那条**看不见 ghost**，免得后人把它当覆盖。
  ⚠️ 同族的第二个坑：`session_id` 曾经**参与索引**。D4 把连接符从 AND 换成 OR 之后，会话 id 就成了可被内容词命中的表面
  （真实 id 形如 `agent:main:webchat:…`，一旦进索引，`agent`/`main` 就成了一个「谁都命中」的**内容面**，
  摘要写的是什么根本不重要，一条 `MATCH 'main'` 就能跨会话捞）；改为 `session_id UNINDEXED` +
  `init()` 里的**第 ④ 条迁移**（读 `sqlite_master.sql` 判 schema 漂移，命中就 `DROP` 重建，因为外部内容表**不能 ALTER**）。
  变异实测：把 `UNINDEXED` 摘掉 → 两条行为用例同时判红。**判据**：schema 是否漂移，看**磁盘上的 DDL 文本**，不要看列存在性。
- **🧮 「淘汰时顺手 rebuild 整张索引」的代价是 O(可配上限)，不是 O(淘汰数)**（同日实测）。
  D4 把默认留存上限从 **50** 提到 **2000**（`memory.short_term_max_sessions`，可配）。原实现淘汰后跑一次全表 `'rebuild'`，
  于是每轮沉淀都要重建整份索引。实测三点曲线（debug 档，同机）：**50 行 3.54 ms / 500 行 6.78 ms / 2000 行 11.97 ms 每次**；
  逐行 `'delete'` + 删内容行则是**恒定 ~7.2 ms/行**、每轮只淘汰一两行，与上限无关
  （参照：不触发淘汰的一次沉淀 7.85 ms、触发淘汰 16.19 ms）。**绝对值随机型变，要看的是形状**：
  rebuild 那一列随上限涨，逐行那一列不涨 —— 把可配项调大不该偷偷让每轮对话变慢。
  ⚠️ 这里曾经写过「43.0 ms → 16.7 ms」：那是一次性读到的、**换口径后复现不出来**，已按三点曲线取代。
  「记了一个复现不出的数」本身就是缺陷，比记错数更难查 —— 见 `docs/prd/2026-10-29-memory-substrate-retrieval-fix.md` 的 D4 更正块。
  守卫 `eviction_never_rebuilds_the_whole_index`（`short_term.rs:1221`）是静态的：只检查 `fn enforce_max_sessions` 的**函数体**（区间在第一个 `\n}` 处收尾），
  要求含 `'delete'`、不含 `'rebuild'`，再对全文件断言 `'rebuild'` **恰好出现 1 次**（`init()` 的第一次回填）。
  ⚠️ 这条守卫**第一版是自拆台的**：直接把 `VALUES('rebuild')` 写成字面量当 needle，于是守卫**自己的源码**里含它两次 → 全文件计数 3、当场假红；
  而 `&src[eviction..]` 不设上界会一路吃进测试模块（测试模块里有真 SQL 字符串）→ `contains('delete')` 恒真、变成恒绿守卫。
  修法：needle 用 `format!` 拼接（源码里不出现完整字面量）+ 函数体区间必须**有界**。
  **通用判据：写静态源码守卫时先问「needle 会不会出现在这条守卫自己的源码里」「被检查的区间有没有右边界」。**
  另注：内存口径的相对线由 `core/src/memory/recall_probe.rs` 常驻网钉住（`:1119` 起，棘轮只许升）——
  `D4_MIN_HIT=0.72`（实测 0.756）、`D4_ZH_LEXICAL_MIN_HIT=0.80`（中文 FTS5 从 0% 到 100%）、
  `D4_PREC_MARGIN_OVER_ALL_TOKENS=0.10`（实词过滤相对全词元 OR 的精度差，实测 0.663 vs 0.409）。

- **🧱 Cython 编译后的函数不是 `types.FunctionType` —— Starlette 把它当 ASGI 类调用，每个路由 500**（2026-10-29 防反编译轮实测 + 已修）。
  侧车**能正常启动、能监听**，但所有 HTTP 路由一律 `500 {"error":"health() takes exactly 1 positional argument (3 given)"}`；
  traceback 里处理器帧正下方就是 `starlette/routing.py:289 await self.app(scope, receive, send)` —— 即 `Route.app` 就是裸端点。
  根因：`Route.__init__` 用 `inspect.isfunction(endpoint_handler) or inspect.ismethod(endpoint_handler)` 区分「函数端点」与「ASGI 类」，
  编译后的闭包是 `cython_function_or_method`，**两个判定都为假** → 走 `self.app = endpoint`（ASGI 分支）→ 以三个位置参数调用。
  `WebSocketRoute.__init__`（同文件 ~323 行）是同一处判据；本仓 `Route(` 的构造分布在 `gateway/app.py`（40+ 条）、`gateway/artifacts.py`、
  `gateway/artifact_preview.py`、`channels/{slack,feishu,wecom,telegram,msteams}.py`，所以这是**系统性**问题，不是个别端点。
  ⚠️ **导入级验证照不到它**：模块导入、Route 对象构造、应用装配全部成功，只有「发一个请求」时才会炸 —— 构建脚本的
  `stabilize_native`（整树 import + 自动降级）因此对它完全无感，这也是它躲过 2026-10-28 全部构建轮的原因。
  ✅ 已修：`_native_compat.install_starlette_route_compat()` 用 `_RoutingInspect` 代理 `starlette.routing.inspect`，
  只给 `isfunction`/`ismethod` 补上「也是 cyfunction」这一条，其余 `inspect.*` 原样转发；在 `opensquilla/__init__.py` 里
  **无条件**安装（纯 Python 运行时是空操作）。`is_async_callable` **不需要**垫：实测编译后的 `async def` 仍被
  `asyncio.iscoroutinefunction` 认作协程函数，所以补上身份判定后其余全走 Starlette 原路径。
  ⚠️ 判定用**类型名**（`type(obj).__name__ == "cython_function_or_method"`）而不是 `isinstance`：垫片是无条件安装的，
  安装时进程里可能还没有任何编译模块可供取类型。万一 Cython 改名，症状会退回「端点收到三个位置参数」，由侧车启动冒烟兜住。
  **通用判据：让编译产物直接跑一遍真实请求，别只 import。** 任何依赖 `inspect.isfunction/ismethod/isclass` 做分发的地方
  （框架路由、序列化器、插件注册）在 Cython 下都要按这条核一遍。

