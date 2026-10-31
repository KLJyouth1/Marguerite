//! 统一网关 RPC 协议层
//!
//! 对齐 Web UI 客户端（`webui/src/lib/rpc.ts`）的 Marguerite 协议契约：
//! - 握手：服务端发 `connect.challenge` 事件 → 客户端回 `connect` req（携带 token）→
//!   服务端回带 `protocol` 字段的 HelloOk res 帧。
//! - 常规调用：客户端 `req`（id/method/params）→ 服务端 `res`（ok/payload | error）。
//! - 事件流：`event` 帧（event/payload/meta/seq），seq 从 1 递增，客户端按连续校验，
//!   断档触发重连（`_noteIncomingFrame`）；`ping` → `pong` 心跳；`tick` 事件保活。
//! - 认证：仅回环本地 token（`connect.params.auth.token` 与服务端一致才放行）。

use serde::Serialize;
use serde_json::{json, Value};

/// 协议版本（客户端 `minProtocol/maxProtocol = 3`，HelloOk 回写同一值）
pub const PROTOCOL_VERSION: u32 = 3;

/// tick 心跳间隔（毫秒，HelloOk `policy.tick_interval_ms` 与推送周期共用）
pub const TICK_INTERVAL_MS: u64 = 30_000;

/// 支持的方法清单（HelloOk `features.methods`，UI 按 `supportsMethod` 探测）
pub const SUPPORTED_METHODS: &[&str] = &[
    // 状态 / 只读
    "health",
    "status",
    // S5：宿主（Python 侧车）退出结构化证据（exit_code + uptime_ms + stderr tail）。
    // Rust 原生实现，**永不代理到侧车**——侧车可能就是已经死掉的那个。
    "host.exit_evidence",
    // P1-6：宿主（Python 侧车）资源采样（RSS）。Rust 原生采集，永不代理。
    "host.metrics",
    // 运维只读：由 transport-neutral ops adapter 接入
    "ops.list_hosts",
    "ops.status",
    "ops.list_files",
    "ops.tail_logs",
    "ops.inspect_process",
    // BUG-6 补登记：system.status 与 status 同义实现
    "system.status",
    "doctor.status",
    "onboarding.router.configure",
    "onboarding.ensemble.configure",
    "onboarding.provider.probe",
    "onboarding.provider.configure",
    // v3 补登记：设置页服务商档案保存/激活/移除（此前未实现也未声明，
    // UI supportsMethod 探测不到 → 保存模型配置直接失败）
    "onboarding.llmProfile.upsert",
    "onboarding.llmProfile.activate",
    "onboarding.llmProfile.active.remove",
    "config.get",
    // 计划执行配置的专用读写口（Rust-first，写 `~/.marguerite/config.yaml`）：
    // `config.get/patch` 在线时由 Python 侧车应答（写它自己的 config.toml），而计划
    // 执行器读的是 config.yaml —— 不给出同源读写口，设置页的开关就是「读到 A、改到 B」。
    "config.plan.get",
    "config.plan.set",
    // P1-6 补登记：已实现但此前未声明，UI supportsMethod 探测不到而功能静默
    "models.list",
    "mcp.list",
    "sessions.create",
    "sessions.delete",
    // 会话可移植性：导出（json/markdown 分享）与导入（不覆盖既有会话）
    "sessions.export",
    "sessions.import",
    // 任务复盘：按会话聚合任务类型/工具调度/思考正文（只读 sessions.db）
    "sessions.insights",
    // 文件改动 diff / 团队成员动向：只有 Rust 侧有数据源
    "sessions.fileChanges",
    "team.members",
    // 交付回执：把一回合的改动 / 产物 / 验证 / 用量合成一条可核验的回执
    "task.receipt",
    // 用户规则：个人/项目规则的读写（纯本地 Markdown，注入 System 前缀）
    "rules.list",
    "rules.get",
    "rules.save",
    "rules.delete",
    // `@` 上下文引用：按查询返回文件/目录/规则/历史会话候选
    "context.references",
    "sessions.get",
    "sandbox.status",
    "sessions.list",
    "sessions.messages.subscribe",
    "sessions.messages.snapshot",
    "sessions.messages.hydrate",
    "chat.history",
    "tools.catalog",
    "tools.patch.get",
    "tools.patch.set",
    // 工作区根 / 绑定状态（P0-1b：未绑定时给可执行提示，前端据此展示落点）
    "workspace.status",
    // 网关层联网检索（与 tools.catalog 宣称对齐：此前目录有、方法无）
    "tools.web_search",
    "tools.web_research",
    "tools.web_fetch",
    // 宝塔面板凭据（baota_login 落盘目标）
    "baota.credentials.set",
    "baota.credentials.get",
    "baota.credentials.clear",
    "agents.list",
    "agents.get",
    "agents.create",
    "agents.update",
    "agents.delete",
    // 专家团队（行业领军人经验蒸馏档案库）+ 用户画像抽取
    // 一句话许愿解析（规则引擎，零 LLM）：口语 → 六要素契约 + 任务 DAG
    "wish.parse",
    "experts.list",
    "agent.profile.analyze",
    // 主智能体能力自检（真实工具/MCP/技能/内置面/人设来源）；前端 supportsMethod 门控它
    "agent.selfcheck",
    "cron.list",
    "usage.status",
    "usage.query",
    // 运行时模式（mode.get 只读 / mode.switch 异步切换）
    "mode.get",
    "mode.switch",
    "mode.status",
    // Bloom 会话（异步）
    // v3.3 运行中插话（对齐 WebUI steer 状态机；无活动轮时 fallback_safe 降级）
    // v3.4 PSI 意识引擎（Aris Brain）
    "brain.status",
    "brain.psi_cycle",
    "brain.set_circadian",
    "brain.interact",
    "sessions.steer.v2",
    "chat.send",
    "chat.abort",
    // 技能
    "skills.list",
    "skills.get",
    "skills.reload",
    "skills.search",
    "skills.install",
    "skills.uninstall",
    "skills.create",
    "skills.update",
    "skills.setEnabled",
    // 技能覆盖报告：递归扫描 `.yaml` + 目录型 `SKILL.md` 后，如实列出已加载/跳过数与原因。
    // 此前 `SkillLoader` 只扫顶层 YAML，`skills/trae/**`、`skills/codebuddy/**` 等
    // 415 个目录型技能对产品完全不可见（用户能看到技能页却找不到它们）。
    "skills.coverage",
    // 记忆层
    "memory.search",
    "memory.save",
    "memory.list",
    // 记忆治理（可验证 / 可删除 / 管理面只读列表）
    "memory.validate",
    "memory.delete",
    "memory.entries",
    "memory.stats",
    "memory.layers.status",
    "memory.layers.switches.get",
    "memory.layers.switches.set",
    "memory.sensory.search",
    "memory.short_term.search",
    "memory.long_term.search",
    "memory.recall.search",
    "memory.cross_project.search",
    "memory.cross_project.expand",
    "memory.behavior.list",
    "memory.behavior.summary",
    // 元智能体自迭代闭环（监控 → 诊断 → 规划 → 沙箱裁决 → 版本闭环）
    "selfevolve.status",
    "selfevolve.signals",
    "selfevolve.analyze",
    "selfevolve.history",
    "selfevolve.sweep",
    "selfevolve.cycle",
    "selfevolve.approve",
    "selfevolve.reject",
    "selfevolve.promote",
    "selfevolve.rollback",
    // 联想记忆网络（突触图）状态
    "memory.synapse.status",
    // 分层稀疏记忆（flyhash + tiered）只读观测面（W-O3）
    "memory.tiered.status",
    // 上一轮记忆注入快照（命中/各层贡献/注入 token/预算裁剪）
    "memory.inject.last",
    // Harness 运行台账（SQLite：runs / items 追加写 + 确定性回放摘要）
    // 数据链路：session → run → item(seq)，字段对齐 Codex 的 thread/turn/item 三级事件。
    "harness.ledger.status",
    "harness.ledger.query",
    "harness.ledger.replay",
    // 验收结论写入（passed / failed / unverified / unknown）：完成门禁与人工复核共用
    "harness.ledger.verdict",
    // harness 运行面快照：钩子注册情况 / 能力成熟度 / 路由策略 / 子代理预算 /
    // 工作区与数据目录可用性 / 台账健康 + problems[]（只读，不触发任何模型调用）
    "harness.status",
    // 战术推演：协作战术的确定性蒙特卡洛 / 裁决 / 台账反事实回放（Rust 原生）
    "tactics.catalog",
    "tactics.simulate",
    "tactics.replay",
    // 对话产线：把蜂群/专家团/子智能体/主智能体从互斥挡位改造成可编译的阶段规格。
    // `line.run` 是唯一会真正派单的方法（异步 + 权限门），其余全是只读预演。
    "line.catalog",
    "line.compile",
    "line.preflight",
    "line.plan",
    "line.run",
    "line.retro",
    // 3×3 权限矩阵只读观测面（W-O3）
    "guard.permission_matrix.status",
    // 上下文管理 / 压缩（compact 走 async LLM 分支；rollback 恢复预压缩快照）
    "context.status",
    "context.compact",
    "context.rollback",
    // 任务拆解规划
    "plans.analyze",
    "plans.create",
    "plans.list",
    "plans.run",
    "plans.cancel",
    "plans.implement",
    "plans.revise",
    "plans.setMode",
    "plans.capabilities",
    // 规划升级（B4）：执行前清单 / 动态重规划 / 并行子任务
    "plans.checklist",
    "plans.run.advance",
    "plans.replan",
    "plans.parallel.run",
    // 计划步骤增删改（B4 编辑器：`plan::plans_step_*` 真实实现）
    "plans.step.add",
    "plans.step.remove",
    "plans.step.update",
    // Goal Round Driver（通过 gateway::goal 适配器；methods.rs dispatch 接入待后续）
    "goal.create",
    "goal.get",
    "goal.edit",
    "goal.pause",
    "goal.resume",
    "goal.complete",
    "goal.blocked",
    "goal.begin_round",
    // 渠道
    "channels.status",
    "channels.pairings",
    "channels.list",
    // 沙箱策略（Rust-first，侧车不可用时确定性可用）
    "sandbox.policy.get",
    "sandbox.policy.defaults",
    "sandbox.policy.update",
    "sandbox.run_mode.preference.get",
    "sandbox.run_mode.preference.set",
    "sandbox.capability.status",
    "sandbox.setup.status",
    // 项目工作区目录浏览 / 选择 / 新建（Rust 本地文件系统实现）
    "sandbox.path.pick",
    "sandbox.path.list",
    "sandbox.path.create-directory",
    // 文件名搜索（Ctrl+P 快速打开）：有界、不跟随链接，结果只在 basePath 之内
    "sandbox.path.search",
    // AI 智能融合路由
    "router.decision",
    "router.selflearning.status",
    // 项目工作区（新建任务引导打开/创建项目文件夹；会话绑定项目根目录）
    "workspaces.list",
    "workspaces.open",
    "workspace.file_changes.list",
    "workspace.file_changes.revert",
    // 内置工作区编辑器（多文件）RPC：读取/写入工作区文本文件
    "workspace.file.read",
    "workspace.file.write",
    // 项目树上下文菜单：重命名 / 删除 / 在文件管理器中定位（2026-10-24）
    "workspace.file.rename",
    "workspace.file.delete",
    "workspace.file.reveal",
    // 企业协同与 SSH：主机台账 + 连接探测（部署/终端工具链复用）
    "collab.ssh.list",
    "collab.ssh.add",
    "collab.ssh.remove",
    "collab.ssh.probe",
    "artifacts.list",
    // Epic A：产物台账（Rust 原生，不代理 Python 侧车）
    "artifacts.get",
    "artifacts.register",
    "artifacts.open_in_system",
    "artifacts.edit_inline",
    // Epic C：桌面自动化（Rust 原生：启动 / 动作 / 录制 / 闸门）
    "desktop.list_apps",
    "desktop.launch",
    // W6.2：附着已打开的应用（返回可复用句柄；handle 可省略）
    "desktop.attach",
    // W6.5：桌面侧 RPC 对齐浏览器。
    // `desktop.a11y` / `desktop.action` / `desktop.windows` 是阻塞 UIA 调用，
    // 只由异步 dispatch（spawn_blocking）提供，并列入 `ASYNC_ONLY_METHODS`。
    "desktop.action",
    "desktop.windows",
    "desktop.a11y",
    "desktop.snapshots",
    "desktop.stop",
    "workspaces.update",
    "workspaces.pin",
    "workspaces.remove",
    "workspaces.history.delete",
    // 画像导入（用户/记忆/导入物 三类目标 + preview/apply/undo 闭环）
    "memory.import.info",
    "memory.import.start",
    "memory.import.status",
    "memory.import.cancel",
    "memory.import.retry",
    "memory.import.apply",
    "memory.import.undo",
    "memory.import.discard",
    // 知识库（KB 2.0：BM25 检索 / 状态 / 灌入）
    "kb.status",
    "kb.search",
    "kb.ingest",
    "kb.list",
    "kb.delete",
    "kb.ingest_file",
    // 代码智能（CodeIntel：项目索引 + 符号/文本混合检索）
    "codeintel.status",
    "codeintel.index",
    "codeintel.search",
    "codeintel.symbols",
    // 内置浏览器 RPC 直通（browser.*：真实 Chromium/CDP 桥暴露给前端）
    "browser.status",
    "browser.open",
    "browser.navigate",
    "browser.evaluate",
    "browser.screenshot",
    "browser.a11y",
    "browser.click",
    "browser.close",
    "browser.control.get",
    "browser.control.set",
    // 引导目录 / 状态（Rust 离线回退：侧车不可用时返回内置 provider 目录）
    "onboarding.catalog",
    "onboarding.status",
    // 电脑操控（三层架构：屏幕感知/键鼠控制/GUI定位）
    "computer_use.action",
    // 模型舰队（多角色路由）
    "fleet.list",
    "fleet.route",
    // Rust 模型档案（models.json / ModelSelector）：WebUI 直接可视可切换
    "models.config_path",
    "models.profiles.list",
    "models.profiles.set_active",
    "models.profiles.set_copilot",
    "models.profiles.add",
    "models.profiles.remove",
    "models.profiles.set_capabilities",
    "model.deepseek.render",
    "model.tokenizer.status",
    // 上一次请求实际发出的思考参数（只读进程级快照）。前端 `supportsMethod` 门控它，
    // 不登记就等于该提示永不显示 —— check-rpc-contract 会拦下这种"写了 UI 没接线"。
    "model.effort.last",
    // DeepSeek 原始输出解析（render 的反向诊断口）
    "model.deepseek.parse",
    // 本地模型中心（聚合下载源 + 智能推荐 + 一键部署/启停 + 本地/云路由标记）
    "models.local.catalog",
    "models.local.recommend",
    "models.local.list",
    "models.local.download",
    "models.local.deploy",
    "models.local.start",
    "models.local.stop",
    "models.local.remove",
    "models.local.status",
    "models.local.system",
    "models.local.finetune",
    "models.local.finetune.list",
    "models.local.finetune.status",
    "models.local.finetune.cancel",
    // 实时 Ollama 枚举 live tag / 一键部署为 kind=Local 路由档案（可选激活）
    "models.local.ollama",
    "models.local.ollama.deploy",
    "models.gate.check",
    // MCP 服务器管理（Rust 配置持久化 + URL 探测；stdio 桥为下一阶段）
    "mcp.servers.list",
    "mcp.servers.add",
    "mcp.servers.update",
    "mcp.servers.remove",
    "mcp.servers.probe",
    "mcp.servers.setEnabled",
    "mcp.tools.list",
    // 自进化历史
    "evolve.history",
    // 插件槽位
    "plugins.list",
    "plugins.status",
    // 插件二次增强（2026-10-24）：统一覆盖率 + 热启停 + 热重载。
    // 全部 Rust 原生（plugins. 不是 Python 代理前缀），不经侧车。
    "plugins.coverage",
    "plugins.setEnabled",
    "plugins.reload",
    // 插件详情 / 删除（管理页 CRUD：清单原文 + 注册表状态 + 适配映射）
    "plugins.detail",
    "plugins.delete",
    // 插件原生加载（MCP 子进程，async-only）：真启动/停止/列工具/调用
    "plugins.mcp.start",
    "plugins.mcp.stop",
    "plugins.mcp.list",
    "plugins.mcp.call",
    // ── Rust 原生实现但长期漏声明的四组（前端 supportsMethod() 因此判 false、
    //    功能入口被隐藏；已由 every_dispatch_arm_is_declared_in_supported_methods 守卫）──
    // 信任校准（trust_rpc.rs）
    "trust.list",
    "trust.grant",
    "trust.revoke",
    "trust.revoke_all",
    // 升级请求（escalation_rpc.rs）
    "escalation.pending",
    "escalation.ack",
    // 任务队列（queue_rpc.rs）
    "queue.list",
    "queue.enqueue",
    "queue.start",
    "queue.pause",
    "queue.resume",
    "queue.cancel",
    "queue.move",
    // ── P2-11 长任务 checkpoint（Rust 侧本地实现，侧车离线也可用）──
    "tasks.checkpoint.list",
    "tasks.checkpoint.abandon",
    "tasks.checkpoint.replay",
    // ── 原生功能（Game Hall / 知识图谱 / 鲸鱼娘 / 用量日聚合）──
    "usage.daily",
    "usage.cost",
    "usage.taskCost",
    "usage.budget.get",
    "usage.budget.set",
    // 三档模型分层 + 会话预算（5c）
    "model.tier.get",
    "model.tier.set",
    "model.ledger",
    "session.budget.get",
    "session.budget.set",
    // 会话推理等级（输入框拖动条）：不声明的话前端 `supportsMethod` 判为不支持，
    // 滑块会静默退回「只在本地记一个值」——功能看似有、实际不落盘。
    "session.thinking.get",
    "session.thinking.set",
    // P3 单一事实源：能力注册表（capabilities_groups）下发给能力中心 /
    // 对话编排能力面板 / 舰队页探测；不声明则前端 supportsMethod 判 false、
    // 能力中心拿不到全量注册表（契约守卫每个 dispatch 臂都必须声明）。
    "capabilities.groups",
    // ── M3-D1 行为事件：最近活动时间线 ──
    "activity.recent",
    "games.list",
    "games.play",
    "runtime.status",
    "kb.graph.view",
    "kb.graph.related",
    "pet.status",
    "pet.list",
    "pet.setSkin",
    "pet.setMood",
    "pet.triggerAction",
    "pet.action",
    "poker.create",
    "poker.join",
    "poker.start",
    "poker.action",
    "poker.state",
    "poker.auto",
    "workspaces.create",
    "modlens.analyze",
    // ── 契约补登记（已实装但历史遗漏，导致 UI `supportsMethod` 探测不到而功能静默）──
    // 配置读写对称（PY_FIRST_METHODS：侧车在线代理 Python，离线回退 Rust）
    "config.effective",
    "config.patch",
    "config.patch.safe",
    // 模型路由（PY_ENGINE_PREFIXES "models.routing." 前缀转发 + Rust 本地实现）
    "models.routing.get",
    "models.routing.set",
    // 运行模式规格 / 蜂群规模上限（ModeSpec / BloomSpec 全字段可视化依赖）
    "mode.spec",
    // Bloom 三段锚定权威阶段（V2.2：ControlCenter 阶段显示）
    "mode.stage",
    "mode.swarm.get",
    "mode.swarm.set",
    // 前缀缓存与语义缓存统计（对话计量条命中率依赖）
    "cache.status",
    // 知识库混合检索（BM25 + 向量 + RRF 融合）
    "kb.search.hybrid",
    // 画像自动抽取（对话流水线自动更新用户画像）
    "agent.profile.auto",
    // SSH 远程执行（主机台账 + 部署工具链）
    "collab.ssh.exec",
    // 项目级知识图谱（保存 / 列出 / 查看 / 分析 / 删除）
    "graph.project.save",
    "graph.project.list",
    "graph.project.view",
    "graph.project.analyze",
    "graph.project.delete",
    // 长时任务（检查点 / 暂停 / 恢复 / 列表）
    "long_running.checkpoint",
    "long_running.pause",
    "long_running.resume",
    "long_running.list",
    // 插件市场（检索 / 安装 / 批量安装 / 升级 / 审计 / 导入导出 / 源管理 / harness 生态）
    "market.list",
    "market.search",
    "market.remote.search",
    "market.install_url",
    "market.install",
    "market.install.batch",
    "market.upgrade",
    "market.audit",
    "market.import",
    "market.export",
    "market.sources",
    "market.harness",
    "market.categories",
    // 页面台账（新增 / 列出 / 检索）
    "pages.add",
    "pages.list",
    "pages.search",
    // 沙箱终端（真实 PTY：spawn / read / input / key / resize / status / shells / kill / list）
    "sandbox.terminal.spawn",
    "sandbox.terminal.read",
    "sandbox.terminal.input",
    "sandbox.terminal.key",
    "sandbox.terminal.resize",
    "sandbox.terminal.status",
    "sandbox.terminal.shells",
    "sandbox.terminal.kill",
    "sandbox.terminal.list",
    "sandbox.run_context.set",
    "sandbox.setup.ensure",
    // 跨平台终端后端探测与执行（WSL / Docker / Native）：让 Windows 用户也能跑 Linux 命令
    "terminal.backends.status",
    "terminal.backend.exec",
    "terminal.backend.auto",
    // 团队编排（编制保存/列出/删除 + 前台与后台一键启动）
    "team.compositions.save",
    "team.compositions.list",
    "team.compositions.remove",
    "team.run",
    "team.run.background",
    // 对话编排统一入口（P2·E）：chat composer 提交编排 JSON，按 form 分发
    "orchestrate.run",
    // 编排终态对账：每会话最后一条进度帧（前端重连 / 长静默后拉取纠正卡片状态）
    "orchestration.status",
    // ── P0 契约补齐（R5：前端调用但后端完全缺失的 23 个不可达方法）──
    // 借鉴 wemux packages/shared 模式：所有跨端方法必须在 SUPPORTED_METHODS 声明，
    // 确保 supportsMethod() 返回 true → UI 功能入口可见
    // cron 全套（schedule 模块已有真实 CRUD，此前仅 list 声明且为 stub）
    "cron.create",
    "cron.update",
    "cron.remove",
    "cron.run",
    "cron.runs",
    "cron.subscribe",
    "cron.unsubscribe",
    // sessions 扩展（abort/patch/preview/reset/search/subscribe/unsubscribe）
    "sessions.abort",
    "sessions.patch",
    "sessions.preview",
    "sessions.reset",
    "sessions.search",
    "sessions.subscribe",
    "sessions.unsubscribe",
    // exec.proposals（转发到 Python 侧车）
    //
    // 曾在此声明 `.create/.update/.remove/.approve/.apply` —— 这 5 个名字**侧车里
    // 根本不存在**（权威清单 `pyengine/.../gateway/scopes.py::METHOD_SCOPES` 里只有
    // pending_count/list/show/accept/reject/settings.*/auto_enabled.*）。它们是早期 API
    // 的残留名，声明了但调用必失败 → 已删除，改为按侧车真实方法名声明。
    "exec.proposals.list",
    "exec.proposals.pending_count",
    "exec.proposals.show",
    "exec.proposals.accept",
    "exec.proposals.reject",
    "exec.proposals.settings.get",
    "exec.proposals.settings.set",
    "exec.proposals.auto_enabled.list",
    "exec.proposals.auto_enabled.disable",
    // 其他缺失方法
    "chat.clarify_submit",
    "providers.status",
    "sandbox.resume",
    "skills.deps.install",
    // ── P1 契约补齐（20 个已通过 PY_ENGINE_PREFIXES 转发但未声明的方法）──
    // 这些方法 Python 侧车已实现，但 supportsMethod() 返回 false 导致 UI 隐藏
    "channels.admin.set",
    "channels.pairing.revoke",
    "channels.probe",
    "channels.restart",
    "logs.status",
    "logs.tail",
    "meta.drafts.discard",
    "meta.runs.confirm_preflight",
    "migration.sources.list",
    // 注：`migration.preview` 与 `onboarding.channel.configure` 曾在此声明，但侧车里
    // 并不存在这两个名字（真实的是 `migration.sources.preview` / `onboarding.channel.{probe,upsert,remove,enable,disable}`）
    // —— 幽灵声明会让 UI 显示一个调用必失败的入口，故删除。
    "onboarding.audio.configure",
    "onboarding.capability.reset",
    "onboarding.imageGeneration.configure",
    "onboarding.llmProfile.remove",
    "onboarding.memory_embedding.configure",
    "onboarding.provider.credential.reveal",
    "onboarding.search.configure",
    "router.feedback.submit",
    // ── P2 契约补齐：前端在调、Python 侧车已实现，但长期漏声明 ──
    // 其中 4 个被前端 `supportsMethod()` 直接门控 → 功能**永远不可达**（不是"偶尔失败"）：
    //   sessions.promptCacheKeepalive.*（提示缓存保活对话框永不渲染）
    //   migration.sources.preview（数据迁移预览被跳过）
    //   onboarding.imageGeneration.models.discover（图片生成模型列表永远为空）
    // 由 webui/scripts/check-rpc-contract.mjs 守卫，防止再次漂移。
    "sessions.promptCacheKeepalive.status",
    "sessions.promptCacheKeepalive.set",
    "sessions.messages.unsubscribe",
    "migration.sources.preview",
    "onboarding.channel.probe",
    "onboarding.channel.upsert",
    "onboarding.channel.remove",
    "onboarding.models.discover",
    "onboarding.llmProfile.models.discover",
    "onboarding.imageGeneration.models.discover",
    "onboarding.channel.enable",
    "onboarding.channel.disable",
    "meta.run",
    "meta.runs.recovery",
    "meta.runs.replay",
    "commands.list_for_surface",
    "channels.pairing.approve",
    // 内容更新源（Rust 原生，**不经 Python 侧车**）：GitHub Release 检查 +
    // update-manifest.json 内容同步（技能/插件）。前端 supportsMethod 门控这 4 个方法。
    "update.config.get",
    "update.config.set",
    "update.check",
    "update.syncContent",
    // 两层助理（L1/L2）：作用域、结构化简报、钉住与稳定偏好。
    // 全部是**纯本地文件 + 本地记忆**，Rust 原生实现，不经 Python 侧车。
    "assistant.scope.get",
    "assistant.scope.set",
    "assistant.brief.get",
    "assistant.prefs.list",
    "assistant.prefs.pin",
    "assistant.prefs.unpin",
    // R-02：写回事实的删除入口（硬删除，仅限本引擎事实库）
    "assistant.facts.delete",
    // 会话分支 / 恢复 / 回放（只读写 `sessions.db` 与本地上下文台账）
    "sessions.fork",
    "sessions.forkThroughTurn",
    "sessions.resume",
    "sessions.replay",
    // WP1 设备总线（devices.*：list/status/call；L2/L3 经审批队列抬升，fail-closed）
    "devices.list",
    "devices.status",
    "devices.call",
];

/// 支持的事件名清单（HelloOk `features.events`，一期为系统/会话事件）
pub const SUPPORTED_EVENTS: &[&str] = &[
    "tick",
    "sessions.changed",
    "artifact.updated",
    "desktop.action_done",
    "session.event.*",
];

/// RPC 错误（res.error 结构，对齐客户端 RpcErrorDetail）
#[derive(Debug, Clone, Serialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    /// 结构化 details（装箱：`RpcError` 是 72 个 `Result` 签名的 Err 变体，
    /// 直接把 32 字节的 `Value` 内联会让整个 Result 超过 clippy 128 字节阈值；
    /// `Box` 不改变 JSON 线格式，只把错误体压到阈值以下）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Box<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted: Option<bool>,
}

/// 超时错误的建议退避间隔（毫秒），供 webui 退避重试参考
const TIMEOUT_RETRY_AFTER_MS: u64 = 1000;

impl RpcError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
            retryable: None,
            retry_after_ms: None,
            accepted: None,
        }
    }

    /// 方法未实现：UI 层 `markMethodUnavailable` 感知（Phase 3 逐个落地）
    pub fn not_implemented(method: &str) -> Self {
        Self::new(
            "NOT_IMPLEMENTED",
            format!("方法 `{method}` 尚未在 Rust 网关实现（Phase 3 引擎桥接入）"),
        )
    }

    pub fn unauthorized() -> Self {
        Self::new("UNAUTHORIZED", "本地网关 token 校验失败")
    }

    /// 附加结构化 details（webui `currentPolicyFromConflict` 等消费）
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(Box::new(details));
        self
    }

    pub fn internal(msg: &str) -> Self {
        Self::new("INTERNAL_ERROR", msg.to_string())
    }

    /// 标记为可重试（瞬时故障语义：侧车超时、网关过载等），
    /// 供 webui `useRpcCall`/上层按 retryable 决定是否退避重试。
    pub fn with_retryable(mut self, retryable: bool, retry_after_ms: Option<u64>) -> Self {
        self.retryable = Some(retryable);
        if let Some(ms) = retry_after_ms {
            self.retry_after_ms = Some(ms);
        }
        self
    }

    /// 调用超时（区别于用户中止/上游取消）：UI 应退避重试。
    /// `waited_ms` 为实际等待时长（毫秒），`retry_after_ms` 建议退避 1s。
    pub fn timeout(method: &str, waited_ms: u64) -> Self {
        Self::new(
            "TIMEOUT",
            format!("调用 `{method}` 超时（已等待 {waited_ms}ms，Python 侧车未响应）"),
        )
        .with_retryable(true, Some(TIMEOUT_RETRY_AFTER_MS))
    }

    /// 用户/上层主动中止（例如用户点击取消）：语义上**不应**退避重试。
    pub fn aborted(reason: &str) -> Self {
        Self::new("ABORTED", format!("调用已中止: {reason}")).with_retryable(false, None)
    }

    /// 上游信道/依赖被取消（例如连接断开、任务被取消）：不可重试。
    pub fn cancelled(reason: &str) -> Self {
        Self::new("CANCELLED", format!("调用已取消: {reason}")).with_retryable(false, None)
    }
}

/// 入站帧（客户端 → 网关），宽松解析未知字段
#[derive(Debug, Clone)]
pub struct InboundFrame {
    pub r#type: String,
    pub id: Option<String>,
    pub method: Option<String>,
    pub params: Value,
}

/// 握手参数（`connect` req 的 `params`）
#[derive(Debug, Clone)]
pub struct ConnectParams {
    pub min_protocol: u32,
    pub max_protocol: u32,
    pub token: Option<String>,
    pub guest_session_key: Option<String>,
}

pub fn parse_inbound(text: &str) -> Result<InboundFrame, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("JSON 解析失败: {e}"))?;
    let r#type = v
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let id = v.get("id").and_then(Value::as_str).map(str::to_string);
    let method = v.get("method").and_then(Value::as_str).map(str::to_string);
    let params = v.get("params").cloned().unwrap_or(Value::Null);
    Ok(InboundFrame {
        r#type,
        id,
        method,
        params,
    })
}

pub fn parse_connect_params(params: &Value) -> ConnectParams {
    let auth = params.get("auth").unwrap_or(&Value::Null);
    let num = |key: &str| -> u32 { params.get(key).and_then(Value::as_u64).unwrap_or(0) as u32 };
    ConnectParams {
        min_protocol: num("minProtocol"),
        max_protocol: num("maxProtocol"),
        token: auth
            .get("token")
            .and_then(Value::as_str)
            .map(str::to_string),
        guest_session_key: auth
            .get("guestSessionKey")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

// ---- 出站帧构造 ----

/// `connect.challenge` 事件（握手第一步，不带 seq）
pub fn challenge_frame() -> Value {
    json!({
        "type": "event",
        "event": "connect.challenge",
        "nonce": uuid::Uuid::new_v4().to_string(),
    })
}

/// HelloOk res 帧：携带 protocol/policy/auth/features，客户端据此判定连接成功
pub fn hello_frame(id: &str, methods: &[&str], events: &[&str]) -> Value {
    json!({
        "type": "res",
        "id": id,
        "ok": true,
        "protocol": PROTOCOL_VERSION,
        "policy": {
            "tick_interval_ms": TICK_INTERVAL_MS,
            "max_frame_bytes": 16 * 1024 * 1024,
        },
        "auth": {
            // 本地回环 owner：UI 的 `isLocalOwner` 依赖 principal.isOwner
            "principal": { "isOwner": true, "kind": "local" },
        },
        "features": {
            "methods": methods,
            "events": events,
        },
    })
}

/// 成功 res 帧
pub fn ok_frame(id: &str, payload: Value) -> Value {
    json!({ "type": "res", "id": id, "ok": true, "payload": payload })
}

/// 失败 res 帧（error 对象对齐客户端 RpcErrorDetail）
pub fn err_frame(id: &str, err: &RpcError) -> Value {
    json!({ "type": "res", "id": id, "ok": false, "error": err })
}

/// 事件帧（带递增 seq）
pub fn event_frame(event: &str, payload: Value, meta: Value, seq: u64) -> Value {
    json!({
        "type": "event",
        "event": event,
        "payload": payload,
        "meta": meta,
        "seq": seq,
    })
}

/// tick 保活事件帧
pub fn tick_frame(seq: u64) -> Value {
    event_frame(
        "tick",
        json!({ "ts": chrono::Utc::now().to_rfc3339() }),
        json!({}),
        seq,
    )
}

/// pong 心跳回帧
pub fn pong_frame() -> Value {
    json!({ "type": "pong" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_inbound_req() {
        let f = parse_inbound(r#"{"type":"req","id":"7","method":"health","params":{}}"#).unwrap();
        assert_eq!(f.r#type, "req");
        assert_eq!(f.id.as_deref(), Some("7"));
        assert_eq!(f.method.as_deref(), Some("health"));
    }

    #[test]
    fn test_parse_inbound_ping_and_malformed() {
        let f = parse_inbound(r#"{"type":"ping"}"#).unwrap();
        assert_eq!(f.r#type, "ping");
        assert!(f.method.is_none());
        assert!(parse_inbound("not json").is_err());
    }

    #[test]
    fn test_parse_connect_params_auth() {
        let p = parse_connect_params(&json!({
            "minProtocol": 3,
            "maxProtocol": 3,
            "client": { "name": "opensquilla-web" },
            "auth": { "token": "t1", "guestSessionKey": "osqg_x" },
        }));
        assert_eq!(p.min_protocol, 3);
        assert_eq!(p.max_protocol, 3);
        assert_eq!(p.token.as_deref(), Some("t1"));
        assert_eq!(p.guest_session_key.as_deref(), Some("osqg_x"));
    }

    #[test]
    fn test_hello_frame_contract() {
        let h = hello_frame("1", SUPPORTED_METHODS, SUPPORTED_EVENTS);
        assert_eq!(h["type"], "res");
        assert_eq!(h["ok"], true);
        assert_eq!(h["protocol"], PROTOCOL_VERSION);
        assert_eq!(h["auth"]["principal"]["isOwner"], true);
        assert!(h["features"]["methods"].is_array());
    }

    #[test]
    fn test_event_frame_seq_and_tick() {
        let e = event_frame("sessions.changed", json!({}), json!({}), 5);
        assert_eq!(e["seq"], 5);
        assert_eq!(e["event"], "sessions.changed");
        let t = tick_frame(6);
        assert_eq!(t["event"], "tick");
        assert_eq!(t["seq"], 6);
    }

    #[test]
    fn test_err_frame_roundtrip_shape() {
        let e = RpcError::unauthorized();
        let f = err_frame("9", &e);
        assert_eq!(f["ok"], false);
        assert_eq!(f["error"]["code"], "UNAUTHORIZED");
        assert_eq!(f["error"]["message"], "本地网关 token 校验失败");
    }

    #[test]
    fn test_rpc_error_timeout_is_retryable() {
        let e = RpcError::timeout("health", 60_000);
        assert_eq!(e.code, "TIMEOUT");
        assert_eq!(e.retryable, Some(true));
        assert!(e.retry_after_ms.is_some());
    }

    #[test]
    fn test_rpc_error_aborted_not_retryable() {
        let e = RpcError::aborted("用户取消");
        assert_eq!(e.code, "ABORTED");
        assert_eq!(e.retryable, Some(false));
    }

    #[test]
    fn test_rpc_error_cancelled_distinct_from_aborted() {
        let a = RpcError::aborted("x");
        let c = RpcError::cancelled("y");
        assert_ne!(a.code, c.code);
        assert_eq!(a.retryable, Some(false));
        assert_eq!(c.retryable, Some(false));
        assert_eq!(c.code, "CANCELLED");
    }

    /// 从 dispatch 源码里抽出所有 dispatch 臂的方法名。
    ///
    /// 用源码扫描而不是运行时反射：`match` 臂无法在运行期枚举，而「实现了但没声明」
    /// 这个漂移只存在于源码里。解析规则与 dispatch 的书写约定一一对应
    /// （行首 `"a.b" =>`），并且跳过注释行。
    ///
    /// 覆盖三个真正持有 `match` 臂的文件：主分派 `methods.rs`，以及被它委托出去的
    /// `goal.rs` / `ops_dispatch.rs`（两者各自有内部 match，同样会产生漏声明）。
    fn dispatch_arm_names() -> Vec<String> {
        const SOURCES: &[(&str, &str)] = &[
            ("methods.rs", include_str!("methods.rs")),
            ("goal.rs", include_str!("goal.rs")),
            ("ops_dispatch.rs", include_str!("ops_dispatch.rs")),
        ];
        let mut out = Vec::new();
        for (_, src) in SOURCES {
            for raw in src.lines() {
                let line = raw.trim();
                if line.starts_with("//") {
                    continue;
                }
                let Some(rest) = line.strip_prefix('"') else {
                    continue;
                };
                let Some(end) = rest.find('"') else { continue };
                let name = &rest[..end];
                if !name.contains('.') {
                    continue;
                }
                if !rest[end + 1..].trim_start().starts_with("=>") {
                    continue;
                }
                out.push(name.to_string());
            }
        }
        out
    }

    /// 契约守卫：每个 dispatch 臂都必须在 `SUPPORTED_METHODS` 声明。
    ///
    /// 历史上曾有 `test_supported_methods_subset_of_dispatch` 守这个方向，但它**已被删除**
    /// （`docs/archive` 里还留着它通过的记录，`methods.rs` 的注释也仍在引用它）——
    /// 于是两个方向都无人看守，漂移就此发生：`trust.*` / `queue.*` / `escalation.*` /
    /// `plans.step.*` 共 16 个方法后端完全可用，而前端 `supportsMethod()` 判 false →
    /// 对应功能入口被隐藏，用户根本点不到。
    ///
    /// 反方向（声明 ⊆ 可分派）无法在 Rust 侧静态校验：`dispatch` 末尾有「Rust 未实现
    /// 且侧车在线 → 自动全量代理」的兜底，Python 侧车实现的方法（权威清单在
    /// `pyengine/.../gateway/scopes.py::METHOD_SCOPES`）在 Rust 源码里天然没有臂。
    /// 前端门控点那一侧由 `webui/scripts/check-rpc-contract.mjs` 守卫。
    #[test]
    fn every_dispatch_arm_is_declared_in_supported_methods() {
        // `agent_rpc_err` 里对「错误码字符串」的 match 不是 RPC 臂，显式排除。
        const ERR_CODE_LOOKALIKES: &[&str] =
            &["agent.exists", "agent.not_found", "agent.builtin_immutable"];
        let missing: Vec<String> = dispatch_arm_names()
            .into_iter()
            .filter(|n| !ERR_CODE_LOOKALIKES.contains(&n.as_str()))
            .filter(|n| !SUPPORTED_METHODS.contains(&n.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "以下 dispatch 臂未在 SUPPORTED_METHODS 声明（前端会判为不支持、功能入口被隐藏）：{missing:?}"
        );
    }

    /// 守卫自身不得退化成空集（否则断言永远通过）。
    #[test]
    fn dispatch_arm_scan_finds_a_plausible_number_of_arms() {
        let arms = dispatch_arm_names();
        assert!(
            arms.len() > 200,
            "源码扫描只解析出 {} 个 dispatch 臂，解析规则可能已与书写约定脱节",
            arms.len()
        );
        assert!(arms.iter().any(|a| a == "chat.send"));
        assert!(arms.iter().any(|a| a == "plans.step.add"));
    }
}
