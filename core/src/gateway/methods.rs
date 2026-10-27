//! 网关方法路由与首批实现
//!
//! 方法名对齐 Marguerite 协议（HelloOk `features.methods` 只暴露已实现者）。
//! Phase 2 交付「状态 / 只读」方法族（健康、配置、沙箱、会话、工具目录）；
//! Phase 3 交付「§5.2 Python 方法代理」（channels/router/meta/onboarding/wizard/secrets/memory/search/artifacts 等）：
//! - 若 Python 侧车可用（[`server::py_client`] 返回 Some），转发调用
//! - 否则返回 `NOT_IMPLEMENTED`，UI 层据此 `markMethodUnavailable`
//!
//! 本文件为方法路由分派中心，具体实现分散到各方法族模块（system/config/mode/sandbox/cache/...）。

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use serde_json::{json, Value};

use crate::aris_brain;
use crate::events::{EventBus, EventType};
use crate::gateway::artifacts;
use crate::gateway::baota;
use crate::gateway::cache;
use crate::gateway::chat;
use crate::gateway::collab;
use crate::gateway::config as gateway_config;
use crate::gateway::desktop;
use crate::gateway::escalation_rpc;
use crate::gateway::fleet;
use crate::gateway::harness_rpc;
use crate::gateway::kb;
use crate::gateway::llm_profile;
use crate::gateway::local_models;
use crate::gateway::long_running;
use crate::gateway::market;
use crate::gateway::mcp;
use crate::gateway::memory as gateway_memory;
use crate::gateway::mode as gateway_mode;
use crate::gateway::models;
use crate::gateway::native;
use crate::gateway::onboarding;
use crate::gateway::plugins_rpc;
use crate::gateway::pet as gateway_pet;
use crate::gateway::plugin_status;
use crate::gateway::protocol::RpcError;
use crate::gateway::router as gateway_router;
use crate::gateway::sandbox;
use crate::gateway::sandbox_paths;
use crate::gateway::selfevolve;
use crate::gateway::server;
use crate::gateway::sessions;
use crate::gateway::shipped_gate;
use crate::gateway::skills;
use crate::gateway::system;
use crate::gateway::team;
use crate::gateway::tools;
use crate::gateway::update;
use crate::gateway::vision;
use crate::gateway::workspace;
use crate::memory::behavior;
use crate::memory::cross_project;

use crate::persistence::config::Config;
use crate::plan;
use crate::pyengine::client::PyGatewayClient;
use crate::sandbox::policy_store;
// 桌面自动化（Epic C）：启动 / 动作 / 录制 / 闸门。仅类型导入，COM 实现隔离在
// `desktop_automation::com`（cfg(windows)），非 Windows 下 AppRegistry 不注册 WpsPpt。
// 注：desktop:: 模块已接管方法实现，这里仅保留少量辅助类型给其余方法族复用。

/// AI 最近一次活动时间戳（毫秒）：chat.send 时标记，供 `pet.status` 判定「工作中」态
static LAST_AGENT_ACTIVITY_MS: AtomicI64 = AtomicI64::new(0);

/// 毫秒时间戳 → RFC3339（`cron.list` 的 `next_run` 主字段）。
fn ms_to_rfc3339(ms: i64) -> Value {
    match chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms) {
        Some(dt) => Value::String(dt.to_rfc3339()),
        None => Value::Null,
    }
}

fn now_ms_i64() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn mark_agent_active() {
    LAST_AGENT_ACTIVITY_MS.store(now_ms_i64(), Ordering::Relaxed);
}

/// 最近 60 秒内是否有 AI 活动（桌宠据此切「工作中」态）
fn agent_is_running() -> bool {
    now_ms_i64() - LAST_AGENT_ACTIVITY_MS.load(Ordering::Relaxed) < 60_000
}

/// §5.2 Python 方法前缀（命中任一前缀，转发到 py_client 调用）
static PY_ENGINE_PREFIXES: &[&str] = &[
    "channels.",
    "router.",
    "routing.",
    "models.routing.",
    "meta.",
    "onboarding.",
    "wizard.",
    "secrets.",
    "memory.dream",
    "memory.import.",
    "memory.repair.",
    "search.",
    "artifacts.",
    "migration.",
    "models.list",
    "models.",
    "approvals.",
    "exec.approval.",
    "proposals.",
    "uploads.",
    "diagnostics.",
    "logs.",
    "commands.",
    "bundle.",
    // P0 契约补齐：exec.proposals.* / chat.clarify_* / providers.* 转发到 Python 侧车
    "exec.proposals.",
    "chat.clarify",
    "providers.",
];

fn is_python_method(method: &str) -> bool {
    // Rust 独占的方法优先判定，见 [`RUST_OWNED_METHODS`] 的说明。
    if RUST_OWNED_METHODS.contains(&method) {
        return false;
    }
    PY_ENGINE_PREFIXES.iter().any(|pre| method.starts_with(pre))
}

/// **Rust 独占**（禁止代理到 Python 侧车）的方法。
///
/// # 为什么需要（实测根因，2026-10-09）
///
/// `PY_ENGINE_PREFIXES` 含 `"artifacts."`，于是 `artifacts.list` 会被**代理到 Python 侧车**；
/// 而产物实际登记在 **Rust** 侧（`write_file` → `tools::file_change::record_artifact`）。
/// 两侧各有一份产物库 ⇒ **写入与读取不同源**：
/// - Rust 日志明确「登记产物 session=<会话键>」（键正确）；
/// - `artifacts.get` 走 Rust（Python 未实现该动作 → 回落）→ **查得到**；
/// - `artifacts.list` 被 Python 抢先应答 → 返回它自己的**空列表**，字段形状也不同
///   （`page_size`/`total_count`/`newest_cursor` 而非 Rust 的 `next_cursor`）。
///
/// 结果：**磁盘已有产物、右栏永远空态**。
///
/// 这与 `config.*` 的既有教训同源（注释见上）：**读链路必须与写链路同源**。
/// 同步分派路径早已是 Rust 实现（见 `dispatch` 内 `artifacts.list` 的注释），
/// 异步路径却仍先代理 —— 两条路径数据源不一致本身就是缺陷。
///
/// 只列**当前确认由 Rust 独占**的动作；`artifacts.edit_inline` / `open_in_system` /
/// `register` 等仍按原样走前缀判定，不在本表内。
static RUST_OWNED_METHODS: &[&str] = &[
    // 产物列表：读写必须同源（登记在 Rust）。见上方说明。
    "artifacts.list",
    // 模型配置文件路径：Rust 独占（无 Python 侧实现）。它命中 `models.` 前缀，若不登记会
    // 被 check-rpc-contract 的双实现守卫判为"新增双实现"——而它本就只该由 Rust 回答，
    // 因为路径真值来自 Rust 的 Config::config_dir()。
    "models.config_path",
];

/// Python 优先的方法（读写对称）：双引擎（侧车在线）模式下先代理 Python，
/// 侧车离线时回退 Rust 实现（config.get 精简实现）或 NOT_IMPLEMENTED（config.patch.*）。
///
/// 背景（修复：设置页开关保存后自动回弹）：
/// webui 设置页保存走 `config.patch.safe`（Rust 未实现 → 代理 Python，写入
/// Marguerite 完整配置），保存后 `loadData()` 读 `config.get`。若 `config.get`
/// 由 Rust 精简实现拦截（只返回 model/mcp/memory/skills/log），则
/// `privacy` / `naming` / `memory.auto_capture_enabled` 等字段全部缺失，
/// 表单被默认值重建 → 开关视觉上自动回弹。因此 `config.*` 读链路必须与写链路
/// 走同一数据源（侧车在线时都代理 Python）。
static PY_FIRST_METHODS: &[&str] = &[
    "config.get",
    "config.effective",
    "config.patch",
    "config.patch.safe",
    // 以下 3 个此前只列入 RUST_FIRST_METHODS，导致 Rust 的降级实现**恒抢答**，
    // Python 侧更完整的真实现永不执行，用户看到「恒空」等确定缺陷。
    // 它们同在 PY_ENGINE_PREFIXES 内，故改列 PY_FIRST：在线走 Python（真源），
    // 离线自动落回 RUST_FIRST 的 Rust 实现（语义自然降级，行为与修复前离线态一致）。
    //
    // ⚠️ 刻意**不含** `onboarding.status`（虽同属被遮蔽族）：A/B 变异实测证明
    // 把它改列 PY_FIRST 会让隔离环境（全新 config）的引导遮罩重现并遮挡全部交互，
    // 导致 webui e2e 从 95 passed 掉到 46 passed（-49）。该方法的 Python 与 Rust
    // 应答形状存在差异（`sectionDetails`/`sections` 等），**需要先做形状对齐与产品
    // 意图确认**，不能简单翻转优先级 —— 见文档 §13。它保持 RUST_FIRST 不变。
    "onboarding.catalog",
    "router.selflearning.status",
    "providers.status",
];

/// 解析前端 cron.create/update params 中的 trigger 字段。
///
/// 支持三种格式（对齐 schedule::ReminderTrigger）：
/// - `{ "trigger": { "kind": "after", "seconds": 600 } }`
/// - `{ "trigger": { "kind": "at", "ts_ms": 1234567890000 } }`
/// - `{ "trigger": { "kind": "every", "interval_seconds": 3600 } }`
fn parse_cron_trigger(params: &Value) -> Result<crate::schedule::ReminderTrigger, String> {
    let trigger = params.get("trigger")
        .ok_or("missing trigger field")?;
    let kind = trigger.get("kind").and_then(Value::as_str).unwrap_or("");
    match kind {
        "after" => {
            let seconds = trigger.get("seconds")
                .and_then(Value::as_u64)
                .ok_or("missing seconds for after trigger")?;
            Ok(crate::schedule::ReminderTrigger::After { seconds })
        }
        "at" => {
            let ts_ms = trigger.get("ts_ms")
                .and_then(Value::as_i64)
                .ok_or("missing ts_ms for at trigger")?;
            Ok(crate::schedule::ReminderTrigger::At { ts_ms })
        }
        "every" => {
            let interval_seconds = trigger.get("interval_seconds")
                .and_then(Value::as_u64)
                .ok_or("missing interval_seconds for every trigger")?;
            Ok(crate::schedule::ReminderTrigger::Every { interval_seconds })
        }
        _ => Err(format!("unknown trigger kind: {kind}")),
    }
}

/// 网关上下文（每个网关进程一份，跨连接共享）
///
/// `Clone` 是断连修复的基础：WS 循环把慢方法的 dispatch 丢到独立任务执行，
/// 每个任务需要自己一份上下文（详见 `gateway::server::handle_ws`）。
#[derive(Clone)]
pub struct GatewayContext {
    pub config: Config,
    pub token: String,
    pub started_at: Instant,
    /// 网关共享事件总线（Bloom 会话事件经它回流所有 WS 客户端）
    pub bus: Arc<EventBus>,
}

/// 仅异步分派的方法（`dispatch_sync` 与 SUPPORTED_METHODS 子集测试需跳过）
pub(crate) const ASYNC_ONLY_METHODS: &[&str] = &[
    "chat.send",
    "chat.abort",
    "skills.install",
    "sandbox.policy.update",
    "context.compact",
    "plans.implement",
    // 并行子任务 v19 真执行化：有 LLM Key 时后台有界并发真执行（Orchestrator），
    // 离线回退确定性标记（simulated=true 如实声明）；IO/LLM 均走 async 分支。
    "plans.parallel.run",
    "agents.create",
    "agents.update",
    "agents.delete",
    "mode.switch",
    // 引导服务商连通性探测：真实 HTTP 请求（onboarding_provider_probe，
    // 带 15s 超时），只能由异步 dispatch 提供；dispatch_sync 无网络 I/O 能力，
    // 故归入 async-only，由 test_supported_methods_subset_of_dispatch 跳过同步校验。
    // 该能力已在异步 dispatch（RUST_FIRST_METHODS 分支）中实现并对外可达。
    "onboarding.provider.probe",
    // 内置浏览器 RPC 直通（browser.open 等为 IO 操作，走 async 分支；status 为同步）
    "browser.open",
    "browser.navigate",
    "browser.evaluate",
    "browser.screenshot",
    "browser.a11y",
    "browser.click",
    "browser.close",
    // MCP 服务器探测：真实 HTTP 连接 + 工具拉取（async-only）
    "mcp.servers.probe",
    // MCP 工具清单聚合：需真实连接各服务器拉取工具（async-only）
    "mcp.tools.list",
    // SSH 连接探测 / 远程执行：真实 ssh CLI（async-only）
    "collab.ssh.probe",
    "collab.ssh.exec",
    // 目录浏览 / 选择 / 新建（pick 为 async 子进程对话框；list/create-directory
    // 虽为 std::fs 同步操作，但统一在 async dispatch 中提供，避免 sync/async 双份实现）
    "sandbox.path.pick",
    "sandbox.path.list",
    "sandbox.path.create-directory",
    // 文件名搜索（Ctrl+P 快速打开）：有界遍历 + spawn_blocking，仅异步 dispatch 可达
    "sandbox.path.search",
    // 电脑操控（W6.5）：控件树 / 动作 / 窗口枚举都是**阻塞** COM/UIA 调用
    // （单次最长 UIA_CALL_TIMEOUT=10s）。留在 dispatch_rust（同步路径）会占住
    // tokio worker 线程、卡住同一运行时的其它 WS RPC；故统一走异步 dispatch +
    // spawn_blocking（与 sandbox.path.search 同一范式），sync 路径明确回 ASYNC_ONLY。
    "desktop.a11y",
    "desktop.action",
    "desktop.windows",
    // 电脑操控：computer_use.action 是 async-only（await 真实屏幕感知/键鼠控制），
    // 仅异步 dispatch 可达；dispatch_sync 无 async 能力，故归入 async-only，
    // 由 test_supported_methods_subset_of_dispatch 跳过同步校验（客户端走 WS 异步 RPC）。
    "computer_use.action",
    // 插件原生加载（MCP 子进程）：spawn/管道读写必须 await，sync 路径不可用
    "plugins.mcp.start",
    "plugins.mcp.stop",
    "plugins.mcp.list",
    "plugins.mcp.call",
    // modlens 视觉取证为 async（await 视觉模型），仅异步 dispatch 可达
    "modlens.analyze",
    // 专家团队并行 spawn（Team::run，真实成员产出 + lead 综合；async-only）
    "team.run",
    // 蜂群后台化（long_running 台账挂载，async-only）
    "team.run.background",
    // 模型路由读写：仅在异步 dispatch 注册（set 需 await 持久化；get 与之成对，
    // 不做 sync/async 双份实现），dispatch_sync 不可达 → async-only。
    "models.routing.get",
    "models.routing.set",
    // 沙箱终端 PTY：spawn/input/kill 需 await 子进程 IO（read/list 为同步，
    // 已在 dispatch_rust 提供，故不在此列）。
    "sandbox.terminal.spawn",
    "sandbox.terminal.input",
    "sandbox.terminal.kill",
    // 实时 Ollama 枚举 / 部署：真实 HTTP（GET /api/tags，3s 超时），只能由异步
    // dispatch 提供；dispatch_sync 无网络 I/O 能力。deploy 的档案写入/激活走同步路径，
    // 但因前置枚举是 async，整个方法归 async-only。
    "models.local.ollama",
    "models.local.ollama.deploy",
    // 对话编排统一入口（P2·E）：single/subagent/team/expert/swarm 分发走 async
    // （subagent spawn、team.run、team.run.background 均需 async 上下文）
    "orchestrate.run",
    // 编排终态对账：返回每会话最后一条进度帧（断线/丢帧后由前端拉取纠正状态）
    "orchestration.status",
    // 会话实时快照 / 延迟水合：在线走侧车（Rust 无流缓冲），离线返回诚实降级；
    // 二者都需要 read py_client()，只在异步 dispatch 提供，单点实现（W2.1）。
    "sessions.messages.snapshot",
    "sessions.messages.hydrate",
    // 侧车专属写能力（daemon-pause 清除 / operator.admin 依赖安装）：Rust 侧
    // 已删除假成功实现，只经异步代理可达；离线时 NOT_IMPLEMENTED（W2.2/W2.3）。
    "sandbox.resume",
    "skills.deps.install",
];

/// Rust 实现优先的方法：即使命中 Python 前缀，也先走 Rust 实现。
///
/// 融合后 `channels.status / pairings / list` 在 Rust 网关内提供最小台账，
/// 不再被 `channels.` 前缀代理到（不可用的）Python 侧车。
static RUST_FIRST_METHODS: &[&str] = &[
    // 用户规则：文件在本地 config_dir，侧车没有对应实现；走 Rust 才不会「读到 A 改到 B」。
    "rules.list",
    "rules.get",
    "rules.save",
    "rules.delete",
    // 引用候选依赖会话绑定的项目根与本地 sessions.db，侧车无此语义。
    "context.references",
    // 计划执行配置（写 `~/.marguerite/config.yaml`）：必须 Rust-first —— 否则侧车在线时
    // 设置页的「计划审批」开关会写进 Python 的 config.toml，而 Rust 计划执行器读的是
    // config.yaml，用户看到「已开启」但执行时根本没有闸门（静默失效）。
    "config.plan.get",
    "config.plan.set",
    "channels.status",
    "channels.pairings",
    "channels.list",
    "router.decision",
    // 自学习状态（Rust 离线基线：显示 enabled/gate/样本占位；真训练由
    // Python 侧车 + dream cadence 推进；Rust 版本不报 NOT_IMPLEMENTED，
    // 让「高级选项 › 记忆与自学习」组 UI 可用）
    "router.selflearning.status",
    // 模型档案（Rust models.json / ModelSelector）：即使命中 models 前缀也先走 Rust，
    // 让 WebUI 在侧车离线时也能列出/切换 Anthropic/OpenAI/DeepSeek 种子与用户档案。
    "models.profiles.list",
    "models.profiles.set_active",
    "models.profiles.set_copilot",
    "models.profiles.add",
    "models.profiles.remove",
    // 多模态能力声明（用户勾选/自定义端点/输入输出参数）
    "models.profiles.set_capabilities",
    // DeepSeek 官方 recipe 渲染（诊断自托管 DeepInfer / 提示词漂移）
    "model.deepseek.render",
    "model.tokenizer.status",
    // DeepSeek 原始输出解析（与 render 成对，诊断 / 评测用，不参与对话链路）
    "model.deepseek.parse",
    // 本地模型中心（Rust 原生：聚合目录 + 智能推荐 + 部署/启停台账），同样命中 models. 前缀
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
    // 实时 Ollama 枚举 / 一键部署：Rust 原生（打 localhost:11434/api/tags + 写 models.json），
    // 命中 models. 前缀时先走 Rust，不被代理到侧车（侧车无此方法会 NOT_IMPLEMENTED）。
    "models.local.ollama",
    "models.local.ollama.deploy",
    // shipped gate 六要素校验（开源权重「宣布 ≠ 可部署」）
    "models.gate.check",
    "mcp.servers.list",
    "mcp.servers.add",
    "mcp.servers.update",
    "mcp.servers.remove",
    // MCP 工具清单：命中 mcp. 前缀时也先走 Rust（Python 侧车无此方法，代理过去会
    // 得 NOT_IMPLEMENTED，前端的工具来源徽标就永远亮不起来）
    "mcp.tools.list",
    // 插件覆盖率 / 热启停：Rust 原生（plugins. 非 Python 代理前缀；显式列入防未来前缀扩张）
    "plugins.coverage",
    "plugins.setEnabled",
    "plugins.reload",
    // 插件原生加载（MCP 子进程）：长驻进程 + 超时，必须由 Rust 原生处理
    "plugins.mcp.start",
    "plugins.mcp.stop",
    "plugins.mcp.list",
    "plugins.mcp.call",
    // 工具面 patch 层（热启停）：命中 tools. 前缀时也必须先走 Rust——Python 侧车
    // 没有工具补丁概念，代理过去会得到 NOT_IMPLEMENTED，前端开关就变成摆设。
    "tools.patch.get",
    "tools.patch.set",
    // 沙箱策略：Rust 确定性实现，避免 Python 侧车慢响应阻塞 WS 事件循环。
    // `sessions.messages.snapshot/hydrate` 不在此列：Rust 没有实时流缓冲，
    // 只提供「离线诚实降级」，在线时必须由侧车的真实实现应答（见 async dispatch）。
    "sandbox.policy.get",
    "sandbox.policy.defaults",
    "sandbox.policy.update",
    "sandbox.run_mode.preference.get",
    "sandbox.run_mode.preference.set",
    "sandbox.capability.status",
    "sandbox.setup.status",
    // 项目树上下文菜单：重命名 / 删除 / 在文件管理器中定位必须走 Rust
    // （Python 侧车没有这些实现，代理过去只会 NOT_IMPLEMENTED）
    "workspace.file.rename",
    "workspace.file.delete",
    "workspace.file.reveal",
    // 上下文管理 / 压缩（Rust 确定性实现，离线可用；compact 走 async LLM 分支）
    "context.status",
    "context.rollback",
    // 跨项目会话记忆（检索本机 AI 工具记忆目录；B3 升级：向量语义 + 渐进披露 + 行为捕获）
    "memory.cross_project.search",
    "memory.cross_project.expand",
    "memory.behavior.list",
    "memory.behavior.summary",
    // 元智能体自迭代闭环（selfevolve.*）：监控 / 诊断 / 规划 / 沙箱裁决 / 版本闭环。
    // 与既有 `meta.*`（Python 侧车方法族）语义不同，故不复用该前缀。
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
    // 联想记忆网络（突触图）状态：真实计数 + 隐私安全的 top 边（哈希 id + 权重）
    "memory.synapse.status",
    // 记忆楼层开关（8 层）：输入框旁弹窗的读写面，必须走 Rust 侧（配置在 config.yaml）
    "memory.layers.switches.get",
    "memory.layers.switches.set",
    // 任务拆解规划（长难复杂任务自动拆解）
    "plans.analyze",
    "plans.create",
    "plans.list",
    "plans.run",
    "plans.cancel",
    "plans.implement",
    "plans.revise",
    "plans.setMode",
    // 规划升级（B4）：执行前清单 / 动态重规划（并行子任务 plans.parallel.run
    // v19 真执行化后走 async 分支，见 ASYNC_ONLY_METHODS）
    "plans.checklist",
    "plans.run.advance",
    "plans.replan",
    // 长时任务（Bloom 长续航：心跳 / 检查点 / 暂停 / 续跑）
    "long_running.list",
    "long_running.pause",
    "long_running.resume",
    "long_running.checkpoint",
    // 沙箱交互式终端（常驻 shell：spawn / input / read / kill / list）
    "sandbox.terminal.spawn",
    "sandbox.terminal.input",
    "sandbox.terminal.read",
    "sandbox.terminal.kill",
    "sandbox.terminal.list",
    // P1 补齐：Python 侧车离线时此前 NOT_IMPLEMENTED 的方法（前端模型选择器/
    // MCP 面板空白、无法建删/查会话）。Rust 回退实现与在线路径数据源一致。
    "models.list",
    "mcp.list",
    "sessions.create",
    "sessions.delete",
    "sessions.get",
    // 会话可移植性：导出/分享/导入必须走 Rust（Python 侧车无此实现）
    "sessions.export",
    "sessions.import",
    // 任务复盘：Python 侧车没有 sessions.db 聚合实现
    "sessions.insights",
    // 文件改动 + 可渲染 diff（代码评审视图数据源；侧车无此台账）
    "sessions.fileChanges",
    // 团队成员实时动向（团队看板数据源；纯 Rust 编排投影）
    "team.members",
    // 交付回执：跨 sessions.db / 文件改动台账 / 产物台账 / harness 台账聚合，侧车无实现
    "task.receipt",
    // v3：设置页服务商档案保存/激活/移除（此前 NOT_IMPLEMENTED，
    // 导致 webui「无法保存模型及路由配置」）
    "onboarding.llmProfile.upsert",
    "onboarding.llmProfile.activate",
    "onboarding.llmProfile.active.remove",
    // 引导目录 / 状态（Rust 离线回退：侧车不可用时返回内置 provider 目录与最小状态，
    // 让设置页 / 渠道编辑器在无 Python 侧车时仍可渲染）
    "onboarding.catalog",
    "onboarding.status",
    "onboarding.router.configure",
    "onboarding.ensemble.configure",
    "onboarding.provider.probe",
    "onboarding.provider.configure",
    "agent.profile.auto",
    // 主智能体能力自检：读的是本进程的真实注册表与编译期内联的人设，
    // 侧车没有对应语义 —— 必须 Rust-first，否则侧车在线时会回 NOT_IMPLEMENTED。
    "agent.selfcheck",
    // 智能体与专家中心（agents.list / experts.list / mode.spec 真实数据）
    "agents.list",
    "experts.list",
    // 一句话许愿解析（规则引擎）：口语 → 六要素契约 + 任务 DAG
    "wish.parse",
    // 产物台账（Epic A）：artifacts.get/register/open_in_system/edit_inline 由 Rust 实现，
    // 不代理 Python 侧车（侧车未实现这些方法，会被误判 NOT_IMPLEMENTED）。
    "artifacts.get",
    "artifacts.register",
    "artifacts.open_in_system",
    "artifacts.edit_inline",
    // ── 原生功能（Game Hall / 知识图谱 / 鲸鱼娘 / 用量日聚合）──
    "usage.daily",
    "kb.graph.view",
    "kb.graph.related",
    // 混合检索（BM25 + 向量 RRF；向量后端按需启用，engine 字段诚实回报）
    "kb.search.hybrid",
    // 专家团队一键启动（Team::run 真实并行 spawn）+ 蜂群后台化 + 团队持久化组合
    "team.run",
    "team.run.background",
    "team.compositions.list",
    "team.compositions.save",
    "team.compositions.remove",
    // 项目级关系图谱（一项目一图：项目文件夹唯一图谱 + 离线静态分析）
    "graph.project.list",
    "graph.project.view",
    "graph.project.save",
    "graph.project.delete",
    "graph.project.analyze",
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
    // DSH 插件市场（本地 16 清单 + 技能市场）
    "market.list",
    "pages.list",
    "pages.add",
    "pages.search",
    "market.search",
    "market.install",
    "market.upgrade",
    // 远程插件源（DSH 开源生态）：索引检索 + 清单直链安装（Rust 侧联网，不经 Python 侧车）
    "market.remote.search",
    "market.install_url",
    // 插件市场安全审计 / 导入 / 导出（本地清单 + 红线扫描）
    "market.audit",
    "market.import",
    "market.export",
    // 企业协同 SSH 执行 + 运行时模式规格（蜂群上限等）
    "collab.ssh.list",
    "collab.ssh.add",
    "collab.ssh.remove",
    "collab.ssh.exec",
    "mode.spec",
    // Bloom 三段锚定权威阶段（V2.2：ControlCenter 显示阶段的后端权威值）
    "mode.stage",
    // 蜂群控制（数量上限设置）/ 插件批量安装与下载源 / 前缀缓存状态
    "mode.swarm.get",
    "mode.swarm.set",
    "market.install.batch",
    "market.sources",
    // deepseek harness 生态浏览（蒸馏表目录 + 分类 facet）
    "market.harness",
    "market.categories",
    "cache.status",
    // P0 契约补齐：cron.* / sessions.* 扩展 / providers.status
    // `sandbox.resume` / `skills.deps.install` **不在此列**：它们是 Python 侧车的能力
    // （denial-pause 清除 / 真实依赖安装），Rust 原生实现此前只返回 {ok:true} 的假成功
    // （2026-10-26 W2 删除）。在线时走通用代理，离线时诚实回 NOT_IMPLEMENTED。
    "cron.list",
    "cron.create",
    "cron.update",
    "cron.remove",
    "cron.run",
    "cron.runs",
    "cron.subscribe",
    "cron.unsubscribe",
    "sessions.search",
    "sessions.abort",
    "sessions.patch",
    "sessions.preview",
    "sessions.reset",
    "sessions.subscribe",
    "sessions.unsubscribe",
    "providers.status",
    // 技能覆盖报告（递归 `.yaml` + 目录型 `SKILL.md`）与 Harness 运行台账：
    // 全部 Rust 原生实现，绝不能被代理到 Python 侧车（`harness.` 是新前缀，
    // 侧车没有同名方法，代理过去只会 NOT_IMPLEMENTED）。
    "skills.coverage",
    "harness.ledger.status",
    "harness.ledger.query",
    "harness.ledger.replay",
    "harness.ledger.verdict",
    "harness.status",
    // 三档模型分层 + 会话预算（5c，Rust 原生；持久化 session_state.json）
    "model.tier.get",
    "model.tier.set",
    "model.ledger",
    "session.budget.get",
    "session.budget.set",
    // 会话推理等级（输入框拖动条的数据源；与 chat.send 的 effort 参数同源）
    "session.thinking.get",
    "session.thinking.set",
    // 上一次请求实际发出的思考参数：进程内快照（`model/llm.rs` 的 effort_wire_slot），
    // 侧车没有同名实现 —— 必须 Rust-first，否则侧车在线时前端永远读到 NOT_IMPLEMENTED，
    // 「推理档位到底发没发出去」这件事就又变成不可自查。
    "model.effort.last",
];

/// 从 RPC 参数里取会话键（兼容 `sessionKey` / `session_key`），空值响亮拒绝。
///
/// 为什么单独抽出来：`session.thinking.*` 两个方法都要取同一个键，而历史 RPC 里
/// 两种命名并存（camelCase 来自 WebUI，snake_case 来自侧车/CLI）。缺键时若静默
/// 用空串去读盘，会写出一条 `thinking/.json` 垃圾文件 —— 必须拒绝。
pub(crate) fn session_key_param(params: &Value) -> Result<String, RpcError> {
    params
        .get("sessionKey")
        .or_else(|| params.get("session_key"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| RpcError::new("INVALID_REQUEST", "sessionKey 不可为空"))
}

/// 方法分派入口（异步）：按方法名路由到 Rust 实现或 Python 侧车代理
pub async fn dispatch(
    ctx: &GatewayContext,
    method: &str,
    params: Value,
) -> Result<Value, RpcError> {
    // 运维只读 ops.*：经 transport-neutral adapter 注入 executor，不执行命令。
    if let Some(result) = crate::gateway::ops_dispatch::dispatch_with_config(method, &params, &ctx.config) {
        return result;
    }
    // 本地模型存在性校验：在**写盘之前**拦下"配了一个 Ollama 里不存在的模型"。
    //
    // 背景（2026-10-22 实测）：用户把 `qwen3.8:27b` 加进路由候选并保存成功，
    // 但该模型在 Ollama 里根本不存在 —— 应用照收不误，用户以为配好了，
    // 之后怎么都"接不上"。这类"静默假成功"比直接报错更伤人。
    //
    // 只对 Ollama 类 provider 生效；Ollama 不可达时放行（见 precheck_local_model 的三态判据）。
    if matches!(
        method,
        "models.profiles.add" | "models.profiles.set_active"
    ) {
        if let Some(reject) = models::precheck_local_model(method, &params).await {
            return Ok(reject);
        }
    }
    // 画像导入 8 RPC（闭环 INFO/START/STATUS/CANCEL/RETRY/APPLY/UNDO/DISCARD）
    // 放最顶部：Result<Value, (code,msg)> 统一转换为 RpcError
    match method {
        "memory.import.info" => return Ok(crate::gateway::memory_import::info(&params)),
        "memory.import.status" => {
            return crate::gateway::memory_import::status(&params)
                .map_err(|(c, m)| RpcError::new(&c, &m));
        }
        "memory.import.cancel" => {
            return crate::gateway::memory_import::cancel(&params)
                .map_err(|(c, m)| RpcError::new(&c, &m));
        }
        "memory.import.start" => {
            return crate::gateway::memory_import::start(&params)
                .map_err(|(c, m)| RpcError::new(&c, &m));
        }
        "memory.import.retry" => {
            return crate::gateway::memory_import::retry(&params)
                .map_err(|(c, m)| RpcError::new(&c, &m));
        }
        "memory.import.apply" => {
            return crate::gateway::memory_import::apply(&params)
                .map_err(|(c, m)| RpcError::new(&c, &m));
        }
        "memory.import.undo" => {
            return crate::gateway::memory_import::undo(&params)
                .map_err(|(c, m)| RpcError::new(&c, &m));
        }
        "memory.import.discard" => return Ok(crate::gateway::memory_import::discard(&params)),
        // 会话推理档位（输入框拖动条）：需要 RpcError 语义（非法档位响亮拒绝），
        // 故与 memory.import 一样在同步/异步两处显式分派，实现只在 sessions 模块一份。
"session.thinking.get" => return crate::gateway::sessions::thinking_get(&params),
        "session.thinking.set" => return crate::gateway::sessions::thinking_set(&params),
        // P3 单一事实源：能力注册表（capabilities_groups）经 RPC 下发给所有消费
        // 方（对话编排能力面板 / CapabilitiesHubView / 舰队页能力探测），前端不再维护镜像。
        "capabilities.groups" => {
            let groups = crate::domain::capabilities::by_group()
                .into_iter()
                .map(|(g, caps)| {
                    serde_json::json!({
                        "id": g,
                        "label": g.label(),
                        "capabilities": caps,
                    })
                })
                .collect::<Vec<_>>();
            return Ok(serde_json::json!({ "ok": true, "groups": groups }));
        }
        // 会话实时快照 / 延迟水合：Rust 没有可回放的流缓冲，在线时必须由侧车
        // 应答（真实 live_turn 事件 + run_mode_lock 等元数据）；离线时返回
        // **空事件 + degraded** 的诚实降级，绝不冒充「已完整水合」（W2.1）。
        "sessions.messages.snapshot" => {
            if let Some(c) = server::py_client() {
                match py_call_timeout(c, method.to_string(), params.clone()).await {
                    Ok(sidecar) => return Ok(sidecar),
                    // 侧车重连期间代理会失败；前端的 bootstrap 把非 METHOD_NOT_FOUND
                    // 当致命错误，所以这里必须退回**诚实降级**（空事件 + degraded），
                    // 不能让一次快照失败把整条订阅判死。
                    Err(err) => tracing::warn!(
                        method = "sessions.messages.snapshot",
                        error = %err.message,
                        "侧车快照失败，退回 Rust 诚实降级"
                    ),
                }
            }
            return Ok(crate::gateway::sessions::messages_snapshot_degraded(&params));
        }
        "sessions.messages.hydrate" => {
            let rust = crate::gateway::sessions::messages_hydrate_rust(&params);
            if let Some(c) = server::py_client() {
                match py_call_timeout(c, method.to_string(), params.clone()).await {
                    Ok(sidecar) => {
                        return Ok(crate::gateway::sessions::merge_hydration(rust, sidecar))
                    }
                    Err(err) => tracing::warn!(
                        method = "sessions.messages.hydrate",
                        error = %err.message,
                        "侧车水合失败，退回 Rust 元数据子集（degraded）"
                    ),
                }
            }
            return Ok(rust);
        }
        _ => {}
    }
    // W9.1：cron.* 在线时**一律**代理到 Python 侧车（真正的调度器 + agent turn 执行器）；
    // Rust 的 ScheduleStore 只是离线兜底。此前 cron.* 在 RUST_FIRST 里被 Rust 抢先应答，
    // 于是控制台看到的是 Rust 的空提醒表，而侧车真实任务从不显示（前端 App.vue 还按
    // 数组读，拿到 {items} 导致「受影响定时任务数」恒为 0）。
    if method.starts_with("cron.") {
        if let Some(c) = server::py_client() {
            return py_call_timeout(c, method.to_string(), params.clone()).await;
        }
    }
    // Goal Round Driver：通过独立 adapter 共享全局 GoalRegistry，保持路由层薄。
    if method.starts_with("goal.") {
        return crate::gateway::goal::dispatch(method, params);
    }
    // WP1 设备总线（devices.*）：list/call 走 async（可能打 LAN HA），status 为同步只读快照。
    // L2/L3 call 在 handler 内经 approval_registry 抬升审批，未批不执行（fail-closed）。
    if method.starts_with("devices.") {
        return match method {
            "devices.list" => crate::gateway::devices::devices_list(ctx, &params).await,
            "devices.call" => crate::gateway::devices::devices_call(ctx, &params).await,
            "devices.status" => Ok(crate::gateway::devices::devices_status(ctx)),
            _ => Err(RpcError::not_implemented(method)),
        };
    }
    // 异步方法族：需后台任务 / 网络，直接在异步上下文实现
    match method {
        "chat.send" => {
            mark_agent_active();
            return chat::send(ctx, &params).await;
        }
        // v3.4 PSI 意识引擎（Aris Brain）：状态/循环/节律/交互情感
        "brain.status" => return Ok(aris_brain::brain_status(&params)),
        "brain.psi_cycle" => return Ok(aris_brain::brain_psi_cycle(&params)),
        "brain.set_circadian" => return Ok(aris_brain::brain_set_circadian(&params)),
        "brain.interact" => return Ok(aris_brain::brain_interact(&params)),
        // v3.3 运行中插话：入队注入而非中断（对齐 WebUI steer 状态机契约）
        "sessions.steer.v2" => return chat::steer_v2(ctx, &params).await,
        "chat.abort" => return chat::abort(ctx, &params).await,
        // doctor.status 必须走 async 分支：其第 7 项是真实网络探针。若留在
        // 同步表里自建 runtime + block_on，会在 tokio worker 线程上二次进入
        // runtime 而 panic（Cannot start a runtime from within a runtime，
        // 实测真机出现 2 次）。async 版全程无 block_on，安全。
        "doctor.status" => return Ok(system::doctor_status(ctx, &params).await),
        "skills.install" => return skills_install(ctx, &params).await,
        // 远程插件源（DSH 开源生态）：索引检索 + 清单直链安装。都要联网，
        // 必须在 async 分支（同步表里 block_on 会触发 runtime 嵌套 panic）。
        "market.remote.search" => {
            return crate::gateway::market_remote::remote_search(&params).await
        }
        "market.install_url" => {
            return crate::gateway::market_remote::install_url(&params).await
        }
        // modlens 视觉取证（图片 base64 → 结构化 JSON；无视觉模型降级元数据兜底）
        "modlens.analyze" => return Ok(vision::modlens_analyze(&params).await),
        // 版本化策略更新：冲突需返回结构化 details，走 async 分支
        "sandbox.policy.update" => return sandbox_policy_update(&params).await,
        // 上下文压缩（B2）：LLM 语义摘要优先、确定性回退，走 async 分支
        "context.compact" => {
            let session = params
                .get("sessionKey")
                .and_then(Value::as_str)
                .unwrap_or("default");
            return Ok(
                crate::context::session::context_compact_async(session, Some(&ctx.bus)).await,
            );
        }
        // 沙箱交互式终端（常驻 shell：spawn/input 为 IO 操作，走 async 分支）
        "sandbox.terminal.spawn" => {
            // 前端可能传 session / sessionKey；未传时保持旧的单根行为。
            // 有会话绑定时，spawn 的允许根取该会话的项目集合（P2-9），
            // 这样终端面板默认 cwd 落在项目主根，而不是应用数据目录。
            let session = params
                .get("session")
                .or_else(|| params.get("sessionKey"))
                .and_then(Value::as_str);
            return Ok(
                crate::tools::terminal::terminal_spawn_for_session(&params, session).await,
            );
        }
        "sandbox.terminal.input" => {
            return Ok(crate::tools::terminal::terminal_input(&params).await);
        }
        "sandbox.terminal.kill" => {
            return Ok(crate::tools::terminal::terminal_kill(&params).await);
        }
        // 目录选择器：Rust 本地原生对话框，不走 Python 侧车（其 tkinter 子进程
        // 在 CREATE_NO_WINDOW 下永久挂起 → WS 代理超时，目录选择器无法打开）
        "sandbox.path.pick" => return sandbox_path_pick(&params).await,
        // 目录浏览 / 新建目录：Rust 本地文件系统实现（项目工作区选择器依赖）。
        // 此前缺失 → dispatch 回退到 NOT_IMPLEMENTED，UI 报「无法加载平台目录」。
        "sandbox.path.list" => return sandbox_path_list(&params).await,
        "sandbox.path.create-directory" => return sandbox_path_create_directory(&params).await,
        // 文件名搜索（Ctrl+P 快速打开）：有界、不跟随链接，结果只在 basePath 之内。
        "sandbox.path.search" => return sandbox_path_search(&params).await,
        // 电脑操控（W6.5）：UIA / 窗口枚举是阻塞 COM 调用，必须离开 async worker
        // （spawn_blocking），否则一次读树/点击会卡住整个 WS 通道（见 ASYNC_ONLY_METHODS）。
        "desktop.a11y" => return desktop_a11y_rpc(params).await,
        "desktop.action" => return desktop_action_rpc((*ctx).clone(), params).await,
        "desktop.windows" => return desktop_windows_rpc().await,
        // 方案实施：既有修订按步骤真实执行（前端「实施计划」按钮）。数据源为
        // Rust 计划系统 plans.json，而非 Python 侧车 plan_revisions 表。
        "plans.implement" => return plans_implement(ctx, &params).await,
        // 并行子任务（v19 真执行化）：配置 LLM Key 时后台有界并发真执行；
        // 离线回退确定性标记（simulated=true 如实声明，绝不冒充已执行）。
        "plans.parallel.run" => return plans_parallel_run_dispatch(ctx, &params).await,
        // 智能体写操作：Rust 台账（agents.json）真实 CRUD，业务错误以 RpcError
        // 返回（客户端依 `data.ok=false` + `error.code` 识别 agent.exists 等码）。
        "agents.create" => {
            return crate::agent::store::create(&params).map_err(agent_rpc_err);
        }
        "agents.update" => {
            return crate::agent::store::update(&params).map_err(agent_rpc_err);
        }
        "agents.delete" => {
            let id = params.get("id").and_then(Value::as_str).unwrap_or_default();
            return crate::agent::store::delete(id).map_err(agent_rpc_err);
        }
        // 运行时模式切换：更新进程级模式 + 持久化 config.yaml + 广播 mode.changed
        "mode.switch" => return gateway_mode::mode_switch(ctx, &params).await,
        // ── 网关层联网检索（tools.web_search / tools.web_research）：
        //    与 tools.catalog 宣称对齐（此前目录有、方法无 → 前端调用必 NOT_IMPLEMENTED）。
        //    复用 agent 工具层同一套 cn.bing + DDG 搜索实现。
        "tools.web_search" => return Ok(tools::web_search(&params).await),
        "tools.web_research" => return Ok(tools::web_research(&params).await),
        "tools.web_fetch" => return Ok(tools::web_fetch(&params).await),
        // 模型路由模式切换：Rust 原生实现（direct/router/ensemble/bloom）。
        "models.routing.set" => return gateway_mode::models_routing_set(ctx, &params).await,
        "models.routing.get" => return Ok(gateway_mode::models_routing_get(ctx)),
        // 内置浏览器 RPC 直通（browser.*）：把真实 Chromium/CDP 桥暴露给前端
        "browser.status" => return Ok(crate::browser::gateway::status()),
        "browser.open" => {
            let bridge = crate::browser::BrowserBridge::new();
            return crate::browser::gateway::open(&params, &bridge).await;
        }
        "browser.navigate" => return crate::browser::gateway::navigate(&params).await,
        "browser.evaluate" => return crate::browser::gateway::evaluate(&params).await,
        "browser.screenshot" => return crate::browser::gateway::screenshot(&params).await,
        "browser.a11y" => return crate::browser::gateway::a11y(&params).await,
        "browser.click" => return crate::browser::gateway::click(&params).await,
        "browser.close" => return crate::browser::gateway::close(&params).await,
        // 控制权租约（用户接管 / 交还）：接管期间 tool.browser 被硬拦截，
        // 前端据 owner 决定面板是「只读镜像」还是「可操作」。
        "browser.control.get" => return Ok(crate::browser::control::snapshot()),
        "browser.control.set" => {
            return match crate::browser::control::set_owner(&params) {
                Ok(v) => Ok(v),
                Err(msg) => Err(RpcError::new("INVALID_REQUEST", msg)),
            };
        }
        // ── 插件原生加载（MCP 子进程）：真启动/停止/列工具/调用 ──
        "plugins.mcp.start" => {
            return Ok(crate::gateway::plugins_mcp_rpc::start(&params).await)
        }
        "plugins.mcp.stop" => return Ok(crate::gateway::plugins_mcp_rpc::stop(&params).await),
        "plugins.mcp.list" => return Ok(crate::gateway::plugins_mcp_rpc::list()),
        "plugins.mcp.call" => return Ok(crate::gateway::plugins_mcp_rpc::call(&params).await),
        // ── 电脑操控（computer_use：三层架构屏幕感知/键鼠控制/GUI定位）──
        "computer_use.action" => {
            let mut args: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            if let Some(obj) = params.as_object() {
                for (k, v) in obj {
                    if let Some(s) = v.as_str() {
                        args.insert(k.clone(), s.to_string());
                    } else {
                        args.insert(k.clone(), v.to_string());
                    }
                }
            }
            // 视觉 grounding 闭环：请求携带 vision/grounding=true 时按配置注入真实视觉传输。
            // 未配置视觉模型 key → 诚实软失败（VISION_UNAVAILABLE），绝不静默当作普通截图。
            let vision = if crate::gateway::vision::wants_vision_grounding(&params) {
                match crate::gateway::vision::computer_use_vision(&ctx.config) {
                    Some(v) => Some(v),
                    None => {
                        return Ok(json!({
                            "ok": false,
                            "error": "请求了视觉 grounding，但当前未配置可用视觉模型（需视觉模型名 + API Key）",
                            "code": "VISION_UNAVAILABLE",
                        }))
                    }
                }
            } else {
                None
            };
            return match crate::tools::computer_use::dispatch(args, vision).await {
                Ok(crate::agent::tool_cycle::ToolResult::Success { data, metadata }) => {
                    let mut out = json!({ "ok": true, "data": data });
                    // 透出 grounding 结果（坐标/置信度，字段名与 GuiGrounding 既有返回一致）
                    if let (Some(x), Some(y)) = (metadata.get("grounding.x"), metadata.get("grounding.y"))
                    {
                        let mut grounding = json!({ "x": x, "y": y });
                        if let Some(c) = metadata.get("grounding.confidence") {
                            grounding["confidence"] = json!(c);
                        }
                        if let Some(d) = metadata.get("grounding.description") {
                            grounding["description"] = json!(d);
                        }
                        out["grounding"] = grounding;
                    }
                    Ok(out)
                }
                Ok(crate::agent::tool_cycle::ToolResult::Error { message, code }) => {
                    Ok(json!({ "ok": false, "error": message, "code": code }))
                }
                Ok(crate::agent::tool_cycle::ToolResult::Timeout {
                    tool_name,
                    duration,
                }) => Ok(
                    json!({ "ok": false, "error": format!("{} 超时 ({:?})", tool_name, duration), "code": "timeout" }),
                ),
                Err(e) => Ok(json!({ "ok": false, "error": e.to_string() })),
            };
        }
        // MCP 服务器探测：真实连接 URL MCP 服务器并拉取工具目录（busy IO，走异步分支）
        "mcp.servers.probe" => {
            return mcp_servers_probe(&params).await;
        }
        // MCP 工具清单（全服务器聚合 + 60s 缓存）：前端据此显示工具来源徽标。
        // `cachedOnly: true` 只读缓存不探测（渲染路径用），避免渲染时产生网络副作用。
        "mcp.tools.list" => {
            let cached_only = params
                .get("cachedOnly")
                .or_else(|| params.get("cached_only"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            return mcp::mcp_tools_list(cached_only).await;
        }
        // SSH 主机探测 / 远程执行：真实 ssh CLI（BatchMode 非交互，busy IO，走异步分支）
        "collab.ssh.probe" => {
            return Ok(collab::collab_ssh_probe(&params).await);
        }
        "collab.ssh.exec" => {
            return Ok(collab::collab_ssh_exec(&params).await);
        }
        // 专家团队一键启动（真实并行 spawn，未配 Key 时如实失败）
        "team.run" => {
            // 必须带总线：否则成员 / 工具进度无处发布，对话页泳道永远空白。
            return Ok(team::team_run_with(&params, Some(ctx.bus.clone())).await);
        }
        // 蜂群后台化：注册 long_running 台账后异步执行
        "team.run.background" => {
            return Ok(team::team_run_background_with(&params, Some(ctx.bus.clone())).await);
        }
        // 对话编排统一入口（P2·E）：按 form 分发 single/subagent/team/expert/swarm，
        // 过 P2·D 权限矩阵，统一上报 agent.started/progress/stopped 事件
        "orchestrate.run" => {
            return crate::gateway::orchestrate::run(ctx, &params).await;
        }
        // 编排终态对账（只读）：前端重连/长时间无进展时拉一次，避免卡片永久「运行中」
        "orchestration.status" => {
            return crate::gateway::orchestrate::status(ctx, &params).await;
        }
        _ => {}
    }
    // Python 优先（读写对称）：config.* 全量读写由 Python 侧车提供完整字段。
    // 侧车在线时先代理，保证 config.patch.safe（写）与 config.get（读）同源，
    // 修复设置页开关保存后自动回弹；离线时回退下方 Rust 精简实现 / NOT_IMPLEMENTED。
    if PY_FIRST_METHODS.contains(&method) {
        if let Some(c) = server::py_client() {
            let py_res = py_call_timeout(c, method.to_string(), params.clone()).await;
            // `config.patch*` 双写桥：Python 落盘的是 `config.toml`（WebUI 的读源），
            // Rust 执行侧只认 `config.yaml`。只代理不镜像 = 设置页保存成功、引擎无变化。
            if method == "config.patch" || method == "config.patch.safe" {
                return mirror_config_write(ctx, py_res, &params);
            }
            // R72：`config.get` / `config.effective` **读侧覆盖**。
            //
            // 双源实测：侧车 `config.toml` 里**没有** `model` / `model_routing` /
            // `bloom` 段（这三个段是 Rust 引擎消费的），而 `onboarding.provider.configure`
            // 与 `llmProfile.*` 写的是 Rust 侧（config.yaml / models.json）。
            // 于是设置页读到的是 Python 的残缺视图 → 「模型路由不显示」「添加模型
            // 保存了却看不见」。这里把 Rust 磁盘配置的这三个引擎自有段覆盖回响应，
            // 让设置页如实反映引擎的真实状态（写侧镜像 + 读侧覆盖 = 单源语义）。
            if method == "config.get" || method == "config.effective" {
                return overlay_engine_owned_sections(ctx, py_res);
            }
            // `onboarding.status` 的 LLM 就绪度同样必须由 Rust 判定。
            //
            // 真机实测（2026-10-22）：引擎正在用 ollama/llama3.2:1b，而该 RPC 回
            // `hasConfig:false`、`llmConfigured:false` → 状态栏显示「需配置」，
            // 用户以为没配好。根因与 `config.get` 同源：Python 的 config.toml 里
            // 没有 `model` 段（那段是 Rust 引擎自有的），它的 `llm_section_status`
            // 于是判 MISSING。执行侧既然归 Rust，就绪度就该问 Rust。
            if method == "onboarding.status" {
                return overlay_onboarding_llm_truth(ctx, py_res);
            }
            return py_res;
        }
    }
    // Rust 实现优先（channels.* 与 Python 前缀重叠）
    if RUST_FIRST_METHODS.contains(&method) {
        // onboarding.provider.probe 是 async（HTTP 探测），在 async 上下文中直接调用
        if method == "onboarding.provider.probe" {
            return Ok(onboarding::onboarding_provider_probe(&params).await);
        }
        // 服务商档案（v3.1）：upsert 校验失败返回结构化 RpcError（带 code），
        // activate/remove 返回 Value。dispatch_rust 的同步映射用 _value 版本兜底。
        if method == "onboarding.llmProfile.upsert" {
            return llm_profile::llm_profile_upsert(&params);
        }
        if method == "onboarding.llmProfile.activate" {
            // 激活现在会落到 `models.json`（执行侧的唯一真相源）并在档案缺失时
            // 返回结构化错误 —— 不能再 `Ok(...)` 吞掉错误当成功。
            return llm_profile::llm_profile_activate(ctx, &params);
        }
        if method == "onboarding.llmProfile.active.remove" {
            return Ok(llm_profile::llm_profile_active_remove(ctx, &params));
        }
        // 实时 Ollama 枚举（GET /api/tags，3s 超时）：async HTTP，直接 await。
        if method == "models.local.ollama" {
            return Ok(local_models::local_ollama_list(&params).await);
        }
        // 部署任意 live Ollama tag 为 kind=Local 档案，可选立即激活。
        if method == "models.local.ollama.deploy" {
            return Ok(local_models::local_ollama_deploy(ctx, &params).await);
        }
        // 元智能体自迭代闭环（监控→诊断→规划→沙箱裁决→版本闭环）：
        // 全部为 Rust 原生确定性实现，且 `selfevolve.` 不是 Python 前缀，
        // 绝不会被代理到侧车（避免「离线即不可用」）。
        if method.starts_with("selfevolve.") {
            return selfevolve::dispatch(method, &params);
        }
        let res = dispatch_rust(ctx, method, &params);
        if !res.is_null() {
            return Ok(res);
        }
    }
    if is_python_method(method) {
        if let Some(c) = server::py_client() {
            return py_call_timeout(c, method.to_string(), params).await;
        }
        return Err(py_unavailable(method));
    }
    let res = dispatch_rust(ctx, method, &params);
    if res.is_null() {
        // Rust 未实现但 Python 侧车在线 → 自动全量代理。
        // 这打通 skills./memory./agents./sandbox./usage./cron./chat./tools.* 等
        // webui 依赖但未显式列入 PY_ENGINE_PREFIXES 的全部 Python 方法组。
        if let Some(c) = server::py_client() {
            return py_call_timeout(c, method.to_string(), params).await;
        }
        return Err(py_unavailable(method));
    }
    Ok(res)
}

/// `plans.implement`：把既有修订按步骤真实执行（前端「实施计划」按钮）。
///
/// 背景：webui 方案卡片由 Rust 计划系统（`plans.analyze/create` → plans.json）
/// 生成，但「实施」按钮此前路由到 Python 侧车的 `plans.implement`，其
/// `plan_revisions` 表从不包含 Rust 创建的修订 → 永远报「Plan revision not found」。
/// 现改由 Rust 原生执行：校验修订 → 启动运行 → 立即返回运行中卡片（并广播
/// plan_revision/plan_run 事件，新任务实施时前端切换会话也能渲染）→ 后台
/// 逐步执行（复用 [`crate::plan::runner::run_plan_revision_steps`]），进度实时回流。
async fn plans_implement(ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    let session_key = params
        .get("sessionKey")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("ws.{}", uuid::Uuid::new_v4()));
    let revision_id = params
        .get("revisionId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RpcError::new("INVALID_REQUEST", "revisionId 不可为空"))?
        .to_string();
    let rev = plan::get_revision_view(&revision_id).ok_or_else(|| {
        RpcError::new(
            "PLAN_REVISION_NOT_FOUND",
            format!("Plan revision not found: {revision_id}"),
        )
    })?;
    if plan::has_active_run(&revision_id) {
        return Err(RpcError::new(
            "PLAN_RUN_ACTIVE",
            "该方案已有实施任务在进行中",
        ));
    }
    // 新任务实施：把入口登记为新会话（sessions.list / chat.history 可见）
    if params.get("intent").and_then(Value::as_str) == Some("new_chat") {
        crate::gateway::chat::record_turn_persisted(
            &session_key,
            "user",
            &format!(
                "【新任务】实施计划「{}」",
                rev["title"].as_str().unwrap_or("")
            ),
        );
        chat::persist_session(&session_key);
    }
    // 启动运行实例并立即返回（后台执行逐步任务）
    let started = plan::plans_run(&json!({ "revisionId": revision_id }));
    let run = started.get("run").cloned().unwrap_or_default();
    if run.is_null() {
        return Err(RpcError::new(
            "PLAN_RUN_START_FAILED",
            started
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("计划运行启动失败"),
        ));
    }
    // 广播卡片 + 运行事件：新任务实施时前端切换会话后据此渲染（applyPlanRun 依赖 currentPlan）
    crate::plan::runner::publish_plan(
        &ctx.bus,
        &session_key,
        EventType::PlanRevision,
        json!({ "plan_revision": rev.clone() }),
    );
    crate::plan::runner::publish_plan(
        &ctx.bus,
        &session_key,
        EventType::PlanRun,
        json!({ "plan_run": run.clone() }),
    );
    let run_id = run["runId"].as_str().unwrap_or("").to_string();
    let bus = ctx.bus.clone();
    let config = ctx.config.clone();
    let sk = session_key.clone();
    let rid = revision_id;
    tokio::spawn(async move {
        let _ = crate::plan::runner::run_plan_revision_steps(
            config,
            rid,
            run_id,
            sk.clone(),
            bus.clone(),
            String::new(),
        )
        .await;
    });
    Ok(json!({
        "sessionKey": session_key,
        "planRevision": rev,
        "planRun": run,
    }))
}

/// `plans.parallel.run` 网关入口（v19 真执行化）。
///
/// 配置了 `model.api_key` 时：后台 spawn [`run_plan_steps_parallel`] 有界并发
/// 真执行，立即返回当前运行视图（`executed=true`，进度经事件回流）。
/// 离线（无 Key）时：回退确定性扇出标记（`simulated=true`），如实声明未真正执行。
async fn plans_parallel_run_dispatch(
    ctx: &GatewayContext,
    params: &Value,
) -> Result<Value, RpcError> {
    let run_id = params
        .get("runId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RpcError::new("INVALID_REQUEST", "runId 不可为空"))?
        .to_string();
    let step_ids: Vec<String> = params
        .get("stepIds")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let session_key = params
        .get("sessionKey")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("ws.{}", uuid::Uuid::new_v4()));

    // 在线（有 LLM Key）：后台真执行，立即返回
    if !ctx.config.model.api_key.is_empty() {
        let config = ctx.config.clone();
        let bus = ctx.bus.clone();
        let rid = run_id.clone();
        let sk = session_key.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::plan::runner::run_plan_steps_parallel(
                config,
                rid.clone(),
                sk.clone(),
                bus,
                step_ids,
            )
            .await
            {
                tracing::warn!(run_id = %rid, err = %e, "并行子任务真执行失败");
            }
        });
        // 立即回当前运行快照（前端据 PlanRun 事件跟进终态）
        let view = plan::plans_checklist(&json!({ "runId": run_id }));
        if view.get("run").is_some() {
            return Ok(json!({ "run": view["run"], "executed": true }));
        }
        return Err(RpcError::new(
            "PLAN_RUN_NOT_FOUND",
            format!("运行不存在：{run_id}"),
        ));
    }

    // 离线回退：确定性扇出标记（simulated=true，绝不冒充）
    let res = plan::plans_parallel_run(params);
    if res.get("error").is_some() {
        return Err(RpcError::new(
            "PLAN_RUN_NOT_FOUND",
            res["error"].as_str().unwrap_or("运行不存在"),
        ));
    }
    Ok(json!({ "run": res["run"], "simulated": true }))
}

/// `config.get` / `config.effective` 的**读侧覆盖**（R72，与 `mirror_config_write`
/// 构成同源读写对的另一半）。
///
/// 只覆盖 **Rust 引擎自有**的三个段：`model`（对话执行）、`model_routing`（档位路由）、
/// `bloom`（候选融合）。其余段（naming / privacy / memory / squilla_router / control_ui…）
/// 是侧车消费的，保持 Python 视图不动 —— 「谁消费、谁提供同源读写」的边界。
fn overlay_engine_owned_sections(
    ctx: &GatewayContext,
    py_res: Result<Value, RpcError>,
) -> Result<Value, RpcError> {
    // Python 失败时**原样透传错误**（绝不能吞成空成功——那是另一种假成功）。
    let mut py = py_res?;
    let rust = crate::persistence::config::Config::load().unwrap_or_else(|_| ctx.config.clone());
    let Some(obj) = py.as_object_mut() else {
        return Ok(py);
    };
    // 顶层扁平形态（config.get 可能返回扁平键）
    obj.insert("model".to_string(), serde_json::to_value(&rust.model).unwrap_or_default());
    obj.insert(
        "model_routing".to_string(),
        serde_json::to_value(&rust.model_routing).unwrap_or_default(),
    );
    obj.insert(
        "modelRouting".to_string(),
        serde_json::to_value(&rust.model_routing).unwrap_or_default(),
    );
    obj.insert("bloom".to_string(), serde_json::to_value(&rust.bloom).unwrap_or_default());
    // 嵌套形态（config.effective 返回 { config: {...} }）
    if let Some(inner) = obj.get_mut("config").and_then(|v| v.as_object_mut()) {
        inner.insert("model".to_string(), serde_json::to_value(&rust.model).unwrap_or_default());
        inner.insert(
            "model_routing".to_string(),
            serde_json::to_value(&rust.model_routing).unwrap_or_default(),
        );
        inner.insert(
            "modelRouting".to_string(),
            serde_json::to_value(&rust.model_routing).unwrap_or_default(),
        );
        inner.insert("bloom".to_string(), serde_json::to_value(&rust.bloom).unwrap_or_default());
    }
    // 执行侧模型视图同样以 Rust 为准（`llm` / `llm_profiles` / `llm_profile_rows` / 超时）。
    //
    // 为什么必须覆盖：Python 的 `config.toml` 里没有 `models.json` 的 active 档案，
    // 它答 `config.get` 会把「引擎实际正在用的模型」报成 Python 侧默认值。
    // 本机实测：引擎已切到本地 Ollama（`models.json.active=ollama-llama3-2-1b`、
    // `config.yaml` 同步），而 `config.get` 仍返回 `provider=deepseek`、
    // `timeout=120`、且没有 `llm_profile_rows` —— 于是：
    //   · 设置页显示"当前模型 deepseek"，与引擎真实状态不符（用户反复报的"没保存"）；
    //   · 输入框模型选择器拿不到带 `kind` 的档案行，三档分组退化成 provider 覆盖后的单条。
    // 复用 `config_get` 的同一份计算（不另写一套，避免两处漂移），只取这四个键。
    {
        let rust_view = crate::gateway::config::config_get(ctx);
        let keys = [
            "llm",
            "llm_profiles",
            "llm_profile_rows",
            "llm_request_timeout_seconds",
        ];
        for key in keys {
            if let Some(value) = rust_view.get(key).cloned() {
                obj.insert(key.to_string(), value.clone());
                if let Some(inner) = obj.get_mut("config").and_then(|v| v.as_object_mut()) {
                    inner.insert(key.to_string(), value);
                }
            }
        }
    }
    Ok(py)
}

/// `onboarding.status` 的**读侧覆盖**：用 Rust 的执行事实回答「模型是否配好」。
///
/// 只改四个语义明确的键，其余（渠道/搜索/图像/语音…）仍由侧车回答 ——
/// 「谁消费、谁提供」的同源边界与 [`overlay_engine_owned_sections`] 一致。
///
/// 判据（全部来自执行侧，不猜）：
///   · `hasConfig`：`config.yaml` 真实存在，或 `Config::load()` 能读出主模型；
///   · `llmConfigured`：`effective_model_fields` 解析出非空 provider + model，
///     且不处于 `action_required` 状态；
///   · `llmSource`：有 active 档案时记 `config`，否则保留侧车结论；
///   · `llmEnvKey`：仅在 provider 要求密钥时才保留侧车的环境变量名。
fn overlay_onboarding_llm_truth(
    ctx: &GatewayContext,
    py_res: Result<Value, RpcError>,
) -> Result<Value, RpcError> {
    let mut py = py_res?;
    let cfg = crate::persistence::config::Config::load().unwrap_or_else(|_| ctx.config.clone());
    let Some(obj) = py.as_object_mut() else {
        return Ok(py);
    };
    let (provider, model, _base_url) =
        crate::model::selector::effective_model_fields(&cfg);
    let configured = !provider.trim().is_empty() && !model.trim().is_empty();
    let config_on_disk = crate::persistence::config::Config::config_path().exists();
    obj.insert(
        "hasConfig".to_string(),
        Value::Bool(config_on_disk || configured),
    );
    obj.insert("llmConfigured".to_string(), Value::Bool(configured));
    if configured {
        obj.insert("llmSource".to_string(), Value::String("config".to_string()));
    }
    Ok(py)
}

/// `config.patch` / `config.patch.safe` 的**双写桥**（Phase 2 根因修复）。
///
/// 侧车在线时这两个方法由 Python 处理（`PY_FIRST_METHODS`），写的是
/// `%LOCALAPPDATA%\Marguerite\pyengine-home\default\config.toml`；而 Rust 执行侧
/// （计划执行器 / 记忆 / 命名 / Bloom / 自学习 / 路由）读的是
/// `~/.marguerite/config.yaml`。只代理不镜像 → 设置页显示保存成功，引擎行为毫无变化
/// （用户口径的「开了没生效」）。
///
/// 语义约定：
/// - **仅在 Python 成功返回后**镜像（Python 校验失败 = 什么都不写，fail-closed）；
/// - Rust 未识别的键由 `config_patch_local` 静默忽略，不需要维护第二份键表；
/// - `restartRequired` 取并集、`linked` 合并去重（Rust 的进程级缓存只有它自己知道）；
/// - 镜像失败如实回 `mirrored:false` + `mirrorError`，绝不伪装成成功。
fn mirror_config_write(
    ctx: &GatewayContext,
    py_res: Result<Value, RpcError>,
    params: &Value,
) -> Result<Value, RpcError> {
    let mut py = py_res?;
    let mirror = gateway_config::config_patch_local(ctx, params);
    let mirror_ok = mirror.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let Some(obj) = py.as_object_mut() else {
        return Ok(py);
    };
    obj.insert("mirrored".to_string(), json!(mirror_ok));
    if !mirror_ok {
        obj.insert(
            "mirrorError".to_string(),
            mirror
                .get("error")
                .cloned()
                .unwrap_or_else(|| json!("config.yaml 镜像写入失败")),
        );
        return Ok(py);
    }
    if mirror
        .get("restartRequired")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        obj.insert("restartRequired".to_string(), json!(true));
    }
    let mut linked: Vec<String> = obj
        .get("linked")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if let Some(extra) = mirror.get("linked").and_then(Value::as_array) {
        for v in extra.iter().filter_map(Value::as_str) {
            if !linked.iter().any(|x| x == v) {
                linked.push(v.to_string());
            }
        }
    }
    if !linked.is_empty() {
        obj.insert("linked".to_string(), json!(linked));
    }
    Ok(py)
}

/// Python 侧车代理调用（带超时）：侧车慢/挂起时快速失败，避免阻塞 WS 事件循环
/// 导致 tick 停发（webui 检测到心跳超时即断连降级「实时更新暂不可用」）。
async fn py_call_timeout(
    c: &'static Arc<PyGatewayClient>,
    method: String,
    params: Value,
) -> Result<Value, RpcError> {
    use std::time::Duration;
    match tokio::time::timeout(Duration::from_secs(3), c.call(&method, params)).await {
        Ok(res) => res,
        Err(_) => Err(
            RpcError::internal(&format!("Python 侧车响应超时（{method}）"))
                .with_retryable(true, Some(800)),
        ),
    }
}

/// `sandbox.policy.update`（Rust-first）：basePolicyVersion 冲突返回 POLICY_VERSION_CONFLICT + currentPolicy
async fn sandbox_policy_update(params: &Value) -> Result<Value, RpcError> {
    let base_version = params
        .get("basePolicyVersion")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let policy = params.get("policy").cloned().unwrap_or_default();
    match policy_store::policy_update(base_version, policy) {
        Ok(p) => Ok(p),
        Err(e) if e.starts_with(policy_store::POLICY_VERSION_CONFLICT) => {
            let current: Value = e
                .split_once('|')
                .and_then(|(_, rest)| serde_json::from_str(rest).ok())
                .unwrap_or_else(|| json!({}));
            Err(RpcError::new(
                policy_store::POLICY_VERSION_CONFLICT,
                "沙箱策略版本冲突（页面已刷新为最新策略，请重试）",
            )
            .with_details(json!({ "currentPolicy": current })))
        }
        Err(e) => Err(RpcError::internal(&e)),
    }
}

/// `sandbox.path.pick` → 代理到 sandbox_paths 模块。
async fn sandbox_path_pick(params: &Value) -> Result<Value, RpcError> {
    sandbox_paths::sandbox_path_pick(params).await
}

/// `sandbox.path.list` → 代理到 sandbox_paths 模块。
async fn sandbox_path_list(params: &Value) -> Result<Value, RpcError> {
    sandbox_paths::sandbox_path_list(params).await
}

/// `sandbox.path.create-directory` → 代理到 sandbox_paths 模块。
async fn sandbox_path_create_directory(params: &Value) -> Result<Value, RpcError> {
    sandbox_paths::sandbox_path_create_directory(params).await
}

/// `sandbox.path.search` → 代理到 sandbox_paths 模块。
async fn sandbox_path_search(params: &Value) -> Result<Value, RpcError> {
    sandbox_paths::sandbox_path_search(params).await
}

// ── 电脑操控 RPC（W6.5）：阻塞 UIA 调用的异步外壳 ─────────────────────────
//
// 三个方法都必须用 `spawn_blocking`：UIA 单次调用最长 10s（`UIA_CALL_TIMEOUT`），
// 在 tokio worker 上直接跑会把同一运行时的其它 WS RPC 一起卡住。desktop 模块
// 返回的是 `{code,message,data}` 业务信封（不是 `RpcError`），故此处只透传成功值；
// 仅 blocking 任务本身 panic/取消时才降级为 `INTERNAL_ERROR`（绝不假装成功）。

/// `desktop.a11y` → 控件树（阻塞 UIA 读树）。
async fn desktop_a11y_rpc(params: Value) -> Result<Value, RpcError> {
    match tokio::task::spawn_blocking(move || desktop::desktop_a11y(&params)).await {
        Ok(v) => Ok(v),
        Err(e) => Err(RpcError::internal(&format!(
            "desktop.a11y 的 blocking 任务未完成: {e}"
        ))),
    }
}

/// `desktop.action` → UI 动作回执（阻塞 UIA/COM，含 observe→act→verify）。
async fn desktop_action_rpc(ctx: GatewayContext, params: Value) -> Result<Value, RpcError> {
    match tokio::task::spawn_blocking(move || desktop::desktop_action(&ctx, &params)).await {
        Ok(v) => Ok(v),
        Err(e) => Err(RpcError::internal(&format!(
            "desktop.action 的 blocking 任务未完成: {e}"
        ))),
    }
}

/// `desktop.windows` → 可见顶层窗口列表（阻塞 `EnumWindows`）。
async fn desktop_windows_rpc() -> Result<Value, RpcError> {
    match tokio::task::spawn_blocking(desktop::desktop_windows).await {
        Ok(v) => Ok(v),
        Err(e) => Err(RpcError::internal(&format!(
            "desktop.windows 的 blocking 任务未完成: {e}"
        ))),
    }
}

/// 智能体写操作错误码 → 语义化消息（agents.create/update/delete 业务错误映射）
fn agent_rpc_err(code: String) -> RpcError {
    let msg = match code.as_str() {
        "agent.exists" => "Agent already exists",
        "agent.not_found" => "Agent not found",
        "agent.builtin_immutable" => "Builtin agent is immutable",
        _ => &code,
    };
    RpcError::new(&code, msg)
}

/// 台账实测预算的剩余比例（未配置预算 → `None`，退回会话态口径）。
///
/// 为什么必须走台账：`SessionState.budget.tokens_used/usd_used` 全仓无人写入（恒 0），
/// 直接读会让预算降档永不触发；真实用量与用户设的阈值都在 `usage` 台账。
fn budget_snapshot_ratio() -> Option<f64> {
    crate::usage::tightest_budget_snapshot().map(|b| b.remaining_ratio)
}

/// 会话预算视图：**有实测快照（用户配过预算）时以台账为准**，否则回退会话态。
///
/// 返回的 `budget_source` 如实标注口径来自哪一侧，避免「改了会话态却看不到变化」
/// 这类困惑（两侧都存在时展示的是真正生效的那一侧）。
fn budget_json(s: &crate::model::tier::SessionState, snap: Option<&crate::usage::BudgetSnapshot>) -> Value {
    match snap {
        Some(b) => {
            let budget = s.budget.clone().with_measured_snapshot(
                b.tokens_cap,
                b.usd_cap,
                b.tokens_used,
                b.usd_used,
            );
            json!({
                "tokens_cap": budget.tokens_cap,
                "usd_cap": budget.usd_cap,
                "tokens_used": budget.tokens_used,
                "usd_used": budget.usd_used,
                "remaining_ratio": budget.remaining_ratio(),
                "state": budget.state().as_str(),
                "budget_source": "usage_ledger",
                "budget_session": b.session,
                "since_ms": b.since_ms,
            })
        }
        None => json!({
            "tokens_cap": s.budget.tokens_cap,
            "usd_cap": s.budget.usd_cap,
            "tokens_used": s.budget.tokens_used,
            "usd_used": s.budget.usd_used,
            "remaining_ratio": s.budget.remaining_ratio(),
            "state": s.budget.state().as_str(),
            "budget_source": "session_state",
        }),
    }
}

/// `wish.parse` 的载荷构造（纯函数，便于单测）。
///
/// 与 `chat` 里注入的 `wish_contract_hint` **同源**（都调 `WishParser` + `decompose`），
/// 所以「控制台预览的契约」与「模型实际收到的契约」不可能不一致 —— 这正是历史上
/// 「UI 说一套、引擎做另一套」的根因形态，这里从源头避免。
fn wish_parse_payload(params: &Value) -> Value {
    let text = params
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let draft = crate::governance::WishParser::parse(text);
    let dag = crate::governance::decompose(&draft);
    json!({
        "is_wish": draft.is_wish,
        "kind": draft.kind.as_str(),
        "kind_label": draft.kind.label_zh(),
        "brief": draft.brief,
        "assumptions": draft.assumptions,
        "constraints": draft.constraints,
        "vague_terms": draft.vague_terms,
        "unknown_slots": draft.unknown_slots,
        "prefer_slice_first": draft.prefer_slice_first,
        "dag": {
            "nodes": dag.nodes,
            "needs_slice": dag.needs_slice,
            "count": dag.len(),
        },
        "hint": format!("{}{}", draft.to_hint(), dag.to_hint()),
    })
}

#[cfg(test)]
mod wish_rpc_tests {
    use super::*;

    #[test]
    fn wish_parse_payload_returns_contract_and_dag() {
        let v = wish_parse_payload(&json!({
            "text": "帮我做个高中化学分子的 3D 网站，要炫酷，不要用占位符"
        }));
        assert_eq!(v["is_wish"], true);
        assert_eq!(v["kind"], "three_d");
        assert!(v["brief"]["goal"].as_str().unwrap_or("").contains("分子"));
        // 假设必须明示（前端要能显示「我替你决定了什么」）
        assert!(!v["assumptions"].as_array().unwrap().is_empty());
        // 硬约束必须被识别
        assert!(v["constraints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["raw"].as_str().unwrap_or("").contains("占位符")));
        // DAG 必须带验收标准
        let nodes = v["dag"]["nodes"].as_array().unwrap();
        assert!(!nodes.is_empty());
        assert!(nodes.iter().all(|n| !n["acceptance"]
            .as_array()
            .map(|a| a.is_empty())
            .unwrap_or(true)));
        // hint 与 chat 注入同源
        assert!(v["hint"].as_str().unwrap_or("").contains("执行契约"));
    }

    #[test]
    fn wish_parse_payload_is_inert_for_chitchat() {
        let v = wish_parse_payload(&json!({ "text": "你好" }));
        assert_eq!(v["is_wish"], false);
        assert_eq!(v["hint"], "");
        assert_eq!(v["dag"]["count"], 0);
    }
}


/// Rust 端方法同步实现（Phase 2 状态方法 + Phase 3 空台账占位 + 融合：技能/记忆/渠道）
/// Python 方法在"没有客户端可转发"时的错误。
///
/// 关键区别（2026-10-22）：侧车**正在启动**（约 6 秒窗口）时，前端应显示
/// "初始化中"并自动重试，**而不是弹错误** —— 否则用户看到的就是
/// "一启动先报错，点重试才连上"。只有 `Ready`/`Failed` 才是真的不可用，
/// 那时必须如实回 `NOT_IMPLEMENTED`。
fn py_unavailable(method: &str) -> RpcError {
    match server::sidecar_phase() {
        server::SidecarPhase::Starting => RpcError::new(
            "SIDECAR_STARTING",
            format!("`{method}` 依赖 Python 引擎，它仍在启动中（通常 3–10 秒），请稍候"),
        ),
        _ => RpcError::not_implemented(method),
    }
}

fn dispatch_rust(ctx: &GatewayContext, method: &str, params: &Value) -> Value {
    match method {
        "health" => system::health(ctx),
        "status" => system::system_status(ctx),
        // S5：宿主（Python 侧车）退出证据。刻意放在 `dispatch_rust` 的最前面，
        // 因为它的语义要求是「**永不**代理到侧车」——侧车本身就是可能已经死掉的那个，
        // 把它代理过去等于向尸体问遗言。
        "host.exit_evidence" => system::host_exit_evidence(),
        // P1-6：侧车资源采样（RSS）。同样永不代理到侧车——它是本机观测值。
        "host.metrics" => system::host_metrics(),
        // BUG-6：`system.status` 与 `status` 同义（webui 设置页调用名），此前占位
        "system.status" => system::system_status(ctx),
        "config.get" => gateway_config::config_get(ctx),
        "config.plan.get" => gateway_config::config_plan_get(ctx),
        "config.plan.set" => gateway_config::config_plan_set(ctx, params),
        "config.patch.safe" => gateway_config::config_patch_local(ctx, params),
        "config.patch" => gateway_config::config_patch_local(ctx, params),
        "config.effective" => gateway_config::config_get(ctx),
        // ── 内容更新源（Rust 原生：GitHub Release 检查 + update-manifest.json 同步）──
        // 不进入 PY_ENGINE_PREFIXES：内容源与本地 data_dir 只有 Rust 侧知道。
        "update.config.get" => update::update_config_get(),
        "update.config.set" => update::update_config_set(params),
        "update.check" => update::update_check(),
        "update.syncContent" => update::update_sync_content(ctx, params),
        // ── 运行时模式（`mode.switch` 由异步分支处理；这里只读）──
        "mode.get" => gateway_mode::mode_get(),
        "mode.spec" => gateway_mode::mode_spec(),
        "mode.stage" => gateway_mode::mode_stage(params),
        "mode.swarm.get" => gateway_mode::mode_swarm_get(),
        "mode.swarm.set" => gateway_mode::mode_swarm_set(params),
        "mode.status" => gateway_mode::mode_get(),
        // ── 模型档案（models.json / ModelSelector）：WebUI 可视可切换 ──
        "models.config_path" => models::models_config_path(),
        "models.profiles.list" => models::models_profiles_list(),
        "models.profiles.set_active" => models::models_profiles_set_active(params),
        "models.profiles.set_copilot" => models::models_profiles_set_copilot(params),
        "models.profiles.add" => models::models_profiles_add(params),
        "models.profiles.remove" => models::models_profiles_remove(params),
        "models.profiles.set_capabilities" => models::models_profiles_set_capabilities(params),
        "model.deepseek.render" => models::deepseek_render_prompt(params),
        "model.tokenizer.status" => crate::model::tokenizer::status(),
        // 上一次请求实际发出的思考参数（只读、进程级快照）。前端用它回答
        // 「我切了档位，请求体里到底变了什么」——没有这个口子，档位就只能靠感觉。
        "model.effort.last" => crate::model::llm::last_effort_wire(),
        // DeepSeek 原始输出解析（诊断 / 评测用；不参与正常对话链路）
        "model.deepseek.parse" => models::deepseek_parse_response(params),
        // ── 本地模型中心（聚合下载源 + 智能推荐 + 一键部署/启停）──
        "models.local.catalog" => local_models::local_catalog_rpc(params),
        "models.local.recommend" => local_models::local_recommend(params),
        "models.local.list" => local_models::local_list(params),
        "models.local.download" => local_models::local_download(params),
        "models.local.deploy" => local_models::local_deploy(ctx, params),
        "models.local.start" => local_models::local_set_started(params, true),
        "models.local.stop" => local_models::local_set_started(params, false),
        "models.local.remove" => local_models::local_remove(params),
        "models.local.status" => local_models::local_status(params),
        "models.local.system" => local_models::local_system(params),
        "models.local.finetune" => local_models::local_finetune(params),
        "models.local.finetune.list" => local_models::local_finetune_list(params),
        "models.local.finetune.status" => local_models::local_finetune_status(params),
        "models.local.finetune.cancel" => local_models::local_finetune_cancel(params),
        "models.gate.check" => shipped_gate::gate_check(params),
        // P1 补齐：模型目录 Rust 回退（侧车离线时前端模型选择器不再空白）
        "models.list" => models::models_list(),
        // ── MCP 服务器管理（Rust config.yaml 持久化；probe 走异步分支）──
        "mcp.servers.list" => mcp::mcp_servers_list(),
        "mcp.servers.add" => mcp::mcp_servers_add(params),
        "mcp.servers.update" => mcp::mcp_servers_update(params),
        "mcp.servers.remove" => mcp::mcp_servers_remove(params),
        "mcp.servers.setEnabled" => mcp::mcp_servers_set_enabled(params),
        // P1 补齐：MCP 目录 Rust 回退（侧车离线时前端 MCP 面板不再空白）
        "mcp.list" => mcp::mcp_list(),
        // 内置浏览器能力探测（同步，探测 Chromium 可执行文件）
        "browser.status" => crate::browser::gateway::status(),
        "sandbox.status" => sandbox::sandbox_status(ctx),
        "sessions.list" => sessions::sessions_list(ctx, params),
        // 会话可移植性：导出/分享/导入（纯本地文件与 DB 操作，不经侧车）
        "sessions.export" => crate::gateway::sessions_portability::export(params),
        "sessions.import" => crate::gateway::sessions_portability::import(params),
        // 任务复盘：只读 sessions.db 的聚合视图
        "sessions.insights" => crate::gateway::sessions_insights::insights(params),
        // 文件改动：把台账正文算成 unified diff（前端做增绿/删红/改黄渲染）
        "sessions.fileChanges" => crate::gateway::session_changes::file_changes(params),
        // 团队成员动向：ProgressReporter 进度帧的内存投影
        "team.members" => crate::orchestration::member_registry::snapshot(
            params.get("session").and_then(Value::as_str),
        ),
        // 交付回执：一回合的改动 / 产物 / 验证 / 用量（只读既有台账）
        "task.receipt" => crate::gateway::task_receipt::task_receipt(params),
        // 用户规则：本地 Markdown 读写（Python 侧车无实现）
        "rules.list" => crate::rules::rules_list(params),
        "rules.get" => crate::rules::rules_get(params),
        "rules.save" => crate::rules::rules_save(params),
        "rules.delete" => crate::rules::rules_delete(params),
        // `@` 上下文引用：候选搜索（文件/目录/规则/历史会话），渲染在 user 消息尾部
        "context.references" => crate::gateway::context_refs::references(params),
        // P1 补齐：会话增删查 Rust 回退（侧车离线时前端无法建删会话）
        "sessions.create" => sessions::sessions_create(ctx, params),
        "sessions.delete" => sessions::sessions_delete(ctx, params),
        "sessions.get" => sessions::sessions_get(ctx, params),
        // 会话订阅 fast ack：真实项目根 + 诚实降级（Rust 没有实时流缓冲）。
        "sessions.messages.subscribe" => sessions::messages_subscribe(params),
        // snapshot / hydrate 只在 async dispatch 提供（在线代理侧车、离线诚实降级），
        // 见 ASYNC_ONLY_METHODS —— 不再保留第二份同步实现。此前这里固定回
        // `hydration_complete:true` + 空事件，等于对前端谎报「已完整水合」。
        "chat.history" => sessions::chat_history(ctx, params),
        // 工具目录：必须来自**真实注册表**（`workspace::tools_catalog`），此前误派发到
        // `tools::tools_catalog` 的硬编码 23 条列表——那里面 `file_read` / `terminal_exec`
        // / `kb_search` 等名字在真实工具面上根本不存在，前端「工具与能力」展示的是编造的
        // 工具名，且因缺 `source` 字段导致 MCP 来源徽标恒不显示。两个实现并存是漏改一行。
        "tools.catalog" => workspace::tools_catalog(),
        // 工具面 patch 层（热启停）：补丁写盘后**下一轮**新建的编排器即生效，无需重启。
        "tools.patch.get" => workspace::tools_patch_get(),
        "tools.patch.set" => workspace::tools_patch_set(params),
        // 工作区根 / 绑定状态（P0-1b）：未绑定时把「文件会落到哪、怎么绑定」
        // 如实告诉用户，而不是只丢一个泛泛失败。
        "workspace.status" => workspace::workspace_status(params),
        // ── 宝塔面板凭据（baota_login 的落盘目标；btpanel-mcp 启动/调用时自动读取）──
        "baota.credentials.set" => baota::baota_credentials_set(params),
        "baota.credentials.get" => baota::baota_credentials_get(),
        "baota.credentials.clear" => baota::baota_credentials_clear(),
        // ── 智能体（真实台账：agents.json 持久化，替换硬编码空数组）──
        "agents.list" => crate::agent::store::list(),
        "agents.get" => {
            let id = params.get("id").and_then(Value::as_str).unwrap_or_default();
            match crate::agent::store::get(id) {
                Some(v) => v,
                None => json!({ "error": "Agent not found", "code": "agent.not_found" }),
            }
        }
        // ── 一句话许愿解析（R68）──
        // 纯规则、零 LLM：把一句口语翻成六要素契约 + 任务 DAG。
        // 前端可用它在发送前「预览我要干什么」，也可用于诊断「为什么模型理解偏了」。
        "wish.parse" => wish_parse_payload(params),
        // ── 专家团队（行业领军人经验蒸馏的专家档案库 + 用户画像抽取）──
        "experts.list" => team::experts_list(params),
        "agent.profile.analyze" => team::agent_profile_analyze(params),
        "agent.profile.auto" => team::agent_profile_auto(),
        // 主智能体能力自检：真实工具/MCP/技能/内置面/人设来源。
        // 存在的理由：用户问「你知道自己能操控浏览器/终端/编辑器吗」「到底有没有
        // 加载我的智能体定义」时，答案必须是可核对的数字，而不是模型的记忆。
        "agent.selfcheck" => crate::agent::capability::selfcheck(params),
        // ── 定时任务（cron.*）：连接 schedule 模块真实 CRUD ────────────────
        // R5/R1 修复：此前 cron.list 返回硬编码空数组 { items: [] }（假性完成），
        // schedule 模块有完整 Reminder CRUD 但网关从未调用。
        // 借鉴 wemux「server 不执行代码，只做 orchestration」原则：
        // 网关层做 dispatch + 序列化，schedule 模块持有状态。
        "cron.list" => {
            // 离线兜底（侧车在线时已被上面的代理拦走）。**返回数组**：前端
            // `useCronJobs` 与 App.vue 的「受影响定时任务数」都按数组读；
            // 时间主字段是 ISO `next_run`（`next_fire_ms` 保留兼容）。
            match crate::schedule::ScheduleStore::open_default() {
                Ok(store) => {
                    let items: Vec<Value> = store.all().iter().map(|r| {
                        let next_run = ms_to_rfc3339(r.next_fire_ms);
                        let (schedule_kind, schedule_raw) = match &r.trigger {
                            crate::schedule::ReminderTrigger::After { seconds } => {
                                ("after", json!(seconds))
                            }
                            crate::schedule::ReminderTrigger::At { ts_ms } => ("at", json!(ts_ms)),
                            crate::schedule::ReminderTrigger::Every { interval_seconds } => {
                                ("every", json!(interval_seconds))
                            }
                        };
                        json!({
                            "id": r.id,
                            "name": r.message,
                            "enabled": !r.fired,
                            "session": r.session,
                            "message": r.message,
                            "next_run": next_run,
                            "last_run": if r.fired { next_run.clone() } else { Value::Null },
                            "lastStatus": if r.fired { json!("ok") } else { Value::Null },
                            "payloadKind": "reminder",
                            "payload_kind": "reminder",
                            "scheduleKind": schedule_kind,
                            "schedule_kind": schedule_kind,
                            "scheduleRaw": schedule_raw.to_string(),
                            "schedule_raw": schedule_raw.to_string(),
                            "trigger": {
                                "kind": r.trigger.kind(),
                                "value": schedule_raw,
                            },
                            "created_at_ms": r.created_at_ms,
                            "next_fire_ms": r.next_fire_ms,
                            "fired": r.fired,
                            "source": "rust_schedule_store",
                        })
                    }).collect();
                    Value::Array(items)
                }
                Err(e) => json!({ "ok": false, "error": { "code": "cron.db_error", "message": e } }),
            }
        }
        "cron.create" => {
            let session = params.get("session").and_then(Value::as_str).unwrap_or("default");
            let message = params.get("message").and_then(Value::as_str).unwrap_or("");
            let trigger = parse_cron_trigger(params);
            match trigger {
                Ok(t) => {
                    match crate::schedule::ScheduleStore::open_default() {
                        Ok(mut store) => match store.register(session, message, t, now_ms_i64()) {
                            Ok(reminder) => json!({ "ok": true, "item": {
                                "id": reminder.id,
                                "session": reminder.session,
                                "message": reminder.message,
                                "trigger": { "kind": reminder.trigger.kind() },
                                "created_at_ms": reminder.created_at_ms,
                                "next_fire_ms": reminder.next_fire_ms,
                                "fired": reminder.fired,
                            }}),
                            Err(e) => json!({ "ok": false, "error": { "code": "cron.invalid_rule", "message": e.to_string() } }),
                        },
                        Err(e) => json!({ "ok": false, "error": { "code": "cron.db_error", "message": e } }),
                    }
                }
                Err(msg) => json!({ "ok": false, "error": { "code": "cron.invalid_trigger", "message": msg } }),
            }
        }
        "cron.update" => {
            // 更新提醒：先删后建（schedule 模块无 update，cancel + register 组合）
            let id = params.get("id").and_then(Value::as_str).unwrap_or("");
            if id.is_empty() {
                json!({ "ok": false, "error": { "code": "cron.missing_id", "message": "id required" } })
            } else {
                let session = params.get("session").and_then(Value::as_str).unwrap_or("default");
                let message = params.get("message").and_then(Value::as_str).unwrap_or("");
                let trigger = parse_cron_trigger(params);
                match trigger {
                    Ok(t) => {
                        match crate::schedule::ScheduleStore::open_default() {
                            Ok(mut store) => {
                                store.cancel(id);
                                match store.register(session, message, t, now_ms_i64()) {
                                    Ok(reminder) => json!({ "ok": true, "item": { "id": reminder.id, "session": reminder.session } }),
                                    Err(e) => json!({ "ok": false, "error": { "code": "cron.invalid_rule", "message": e.to_string() } }),
                                }
                            }
                            Err(e) => json!({ "ok": false, "error": { "code": "cron.db_error", "message": e } }),
                        }
                    }
                    Err(msg) => json!({ "ok": false, "error": { "code": "cron.invalid_trigger", "message": msg } }),
                }
            }
        }
        "cron.remove" => {
            let id = params.get("id").and_then(Value::as_str).unwrap_or("");
            if id.is_empty() {
                json!({ "ok": false, "error": { "code": "cron.missing_id", "message": "id required" } })
            } else {
                match crate::schedule::ScheduleStore::open_default() {
                    Ok(mut store) => {
                        let removed = store.cancel(id);
                        json!({ "ok": removed, "id": id })
                    }
                    Err(e) => json!({ "ok": false, "error": { "code": "cron.db_error", "message": e } }),
                }
            }
        }
        "cron.run" => {
            // 手动触发提醒：返回到期提醒（不改变 fired 状态，仅查询）
            let session = params.get("session").and_then(Value::as_str).unwrap_or("default");
            match crate::schedule::ScheduleStore::open_default() {
                Ok(mut store) => {
                    let due = store.due(session, now_ms_i64());
                    let items: Vec<Value> = due.iter().map(|r| json!({
                        "id": r.id, "session": r.session, "message": r.message
                    })).collect();
                    json!({ "ok": true, "triggered": items })
                }
                Err(e) => json!({ "ok": false, "error": { "code": "cron.db_error", "message": e } }),
            }
        }
        "cron.runs" => {
            // 运行历史：列出已触发的提醒
            match crate::schedule::ScheduleStore::open_default() {
                Ok(store) => {
                    let fired: Vec<Value> = store.all().iter()
                        .filter(|r| r.fired)
                        .map(|r| json!({ "id": r.id, "session": r.session, "message": r.message, "fired_at_ms": r.next_fire_ms }))
                        .collect();
                    json!({ "items": fired })
                }
                Err(e) => json!({ "ok": false, "error": { "code": "cron.db_error", "message": e } }),
            }
        }
        "cron.subscribe" => {
            // 事件订阅：返回确认（事件总线已有 sessions.changed 事件）
            json!({ "ok": true, "event": "cron.changed", "subscribed": true })
        }
        "cron.unsubscribe" => {
            json!({ "ok": true, "event": "cron.changed", "subscribed": false })
        }
        // ── sessions 扩展（P0 补齐：此前完全不可达）──
        // 借鉴 wemux「shared 契约 + server orchestration」模式：dispatch 调用
        // sessions 模块，sessions 模块调用 persistence 层，职责分层清晰。
        "sessions.search" => sessions::sessions_search(ctx, params),
        "sessions.abort" => sessions::sessions_abort(ctx, params),
        "sessions.patch" => sessions::sessions_patch(ctx, params),
        "sessions.preview" => sessions::sessions_preview(ctx, params),
        "sessions.reset" => sessions::sessions_reset(ctx, params),
        "sessions.subscribe" => sessions::sessions_subscribe(ctx, params),
        "sessions.unsubscribe" => sessions::sessions_unsubscribe(ctx, params),
        "providers.status" => {
            // ⚠️ 形状契约：消费方 webui/src/views/OverviewView.vue:629 用
            // `Array.isArray(data?.providers)` 判别，返回**对象**会被直接丢弃 ⇒ 服务商表恒空。
            // 此处为**离线降级**（在线时由 PY_FIRST 走 Python 的 rpc_tools.py 真实现），
            // 因此返回空数组 + degraded 标记，而非形状不符的对象。
            json!({
                "ok": true,
                "providers": [],
                "degraded": true,
                "degradedReason": "sidecar_unavailable: 服务商状态由 Python 侧车提供",
            })
        }
        // `sandbox.resume` / `skills.deps.install` 的 Rust 假实现已删除（W2）：
        // 它们只有 Python 侧车实现（denial-pause 清除 / operator.admin 依赖安装），
        // 此前 Rust 回 {ok:true} 但没有任何副作用。现在在线走通用代理，
        // 离线由分派尾部诚实回 NOT_IMPLEMENTED（绝不假装成功）。
        // ── Onboarding 路由/集成配置（设置页模型路由保存）──
        "onboarding.router.configure" => onboarding::onboarding_router_configure(ctx, params),
        "onboarding.ensemble.configure" => onboarding::onboarding_ensemble_configure(ctx, params),
        // v3.1：legacy provider.configure 现在同时接受 webui 发送的 camelCase
        // `model` 字段（此前只认 modelName/model_name，导致「保存了但没生效」）
        "onboarding.provider.configure" => onboarding::onboarding_provider_configure(ctx, params),
        // ── 服务商档案（v3：webui 设置页保存主路径）──
        // dispatch_rust 返回 Value；upsert 校验失败时返回 ok:false 结构而非 RpcError，
        // 与 onboarding.provider.configure 的错误语义保持一致。
        "onboarding.llmProfile.upsert" => llm_profile::llm_profile_upsert_value(params),
        "onboarding.llmProfile.activate" => {
            llm_profile::llm_profile_activate_value(ctx, params)
        }
        "onboarding.llmProfile.active.remove" => {
            llm_profile::llm_profile_active_remove(ctx, params)
        }
        // ── 用量（真实台账：每次 LLM 调用经 usage::record 落库，status/query 聚合返回）──
        "usage.status" => crate::usage::status(params),
        // ── 成本估算 + peak/off-peak 时段（成本策略变化启示）──
        "usage.cost" => crate::usage::cost(params),
        // ── 前缀缓存状态（命中率/复用率，供对话页计量条）──
        "cache.status" => cache::cache_status(),
        "usage.query" => crate::usage::query(params),
        "usage.daily" => {
            crate::usage::daily(params.get("days").and_then(Value::as_u64).unwrap_or(28) as u32)
        }
        // ── M3-D2 FinOps：任务成本卡片 + 会话预算阈值（超限触发升级）──
        "usage.taskCost" => crate::usage::task_cost(params),
        "usage.budget.get" => crate::usage::budget_get(params),
        "usage.budget.set" => crate::usage::budget_set(params),
        // ── 三档模型分层 + 会话预算（5c；Rust 原生，持久化 ~/.marguerite/session_state.json）──
        // 与任务 2（会话级思考强度持久化）同生命周期、共用同一 SessionBudget 口径。
        "model.tier.get" => {
            use crate::model::tier::{ModelTier, SessionState, TierRequest, TierRules};
            let s = SessionState::load();
            let rules = TierRules::from_embedded();
            // 本次请求的档位信号（均可选）。给出后 effective_tier 才反映模式/复杂度/延迟；
            // 不给出时退化为"锁定档 + 预算"的保守结论（与旧行为一致，兼容既有调用方）。
            //
            // 为什么必须把请求信号接进来：仅靠会话态算档，会让**每个未锁定的请求都落到均衡档**
            // —— 长程科研与桌宠闲聊享受同一档位，档位分层形同虚设。
            let req = TierRequest {
                mode_hint: params
                    .get("mode")
                    .and_then(|v| v.as_str())
                    .filter(|m| !m.is_empty())
                    .map(str::to_string),
                complex: params.get("complex").and_then(Value::as_bool).unwrap_or(false),
                latency_slo_ms: params
                    .get("latency_slo_ms")
                    .and_then(Value::as_u64)
                    .map(|v| v as u32),
                ..Default::default()
            };
            let decision = s.resolve_tier_with(req, budget_snapshot_ratio());
            json!({
                "locked_tier": s.locked_tier.map(|t| t.as_str()),
                "thinking_effort": s.thinking_effort.reasoning_effort(),
                "effective_tier": decision.tier.as_str(),
                // 可解释选档：为什么落到这一档（设计稿 §3.1 ⑥ 的 RouteDecision.why）
                "tier_reasons": decision.why,
                "budget": budget_json(&s, crate::usage::tightest_budget_snapshot().as_ref()),
                "tiers": {
                    "flagship": rules.chain_for(ModelTier::Flagship),
                    "balanced": rules.chain_for(ModelTier::Balanced),
                    "turbo": rules.chain_for(ModelTier::Turbo),
                }
            })
        }
        "model.tier.set" => {
            use crate::model::tier::{ModelTier, SessionState};
            use crate::model::thinking::ThinkingEffort;
            let mut s = SessionState::load();
            if let Some(t) = params.get("tier").and_then(|v| v.as_str()) {
                s.locked_tier = match t.trim() {
                    "" | "auto" | "none" => None,
                    other => Some(ModelTier::parse(other)),
                };
            }
            if let Some(e) = params.get("thinking_effort").and_then(|v| v.as_str()) {
                s.thinking_effort = ThinkingEffort::from_spec_str(e);
            }
            let _ = s.save();
            json!({
                "ok": true,
                "locked_tier": s.locked_tier.map(|t| t.as_str()),
                "thinking_effort": s.thinking_effort.reasoning_effort(),
                "effective_tier": s.effective_tier().as_str(),
            })
        }
        // 用量台账（会话预算四要素：时间/用量/余额/状态；消灭「积分对不上」）
        "model.ledger" => {
            use crate::model::tier::SessionState;
            let s = SessionState::load();
            let snap = crate::usage::tightest_budget_snapshot();
            budget_json(&s, snap.as_ref())
        }
        "session.budget.get" => {
            use crate::model::tier::SessionState;
            let s = SessionState::load();
            let snap = crate::usage::tightest_budget_snapshot();
            budget_json(&s, snap.as_ref())
        }
        "session.budget.set" => {
            use crate::model::tier::SessionState;
            let mut s = SessionState::load();
            if let Some(tc) = params.get("tokens_cap").and_then(Value::as_u64) {
                s.budget.tokens_cap = tc;
            }
            if let Some(uc) = params.get("usd_cap").and_then(Value::as_f64) {
                s.budget.usd_cap = uc;
            }
            if let Some(tu) = params.get("tokens_used").and_then(Value::as_u64) {
                s.budget.tokens_used = tu;
            }
            if let Some(uu) = params.get("usd_used").and_then(Value::as_f64) {
                s.budget.usd_used = uu.max(0.0);
            }
            let _ = s.save();
            json!({
                "ok": true,
                "remaining_ratio": s.budget.remaining_ratio(),
                "state": s.budget.state().as_str(),
            })
        }
        // ── M3-D1 行为事件：最近活动时间线（审计日志同源，WebUI 时间线视图数据源）──
        "activity.recent" => crate::events::product::recent(
            params.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize,
        ),
        // ── 知识图谱（kb.graph.*，全局图单例：实体-关系可视化）──
        "kb.graph.view" => kb::kb_graph_view(),
        "kb.graph.related" => kb::kb_graph_related(params),
        // ── 项目级关系图谱（graph.project.*，一项目一图）──
        "graph.project.list" => kb::graph_project_list(),
        "graph.project.view" => kb::graph_project_view(params),
        "graph.project.save" => kb::graph_project_save(params),
        "graph.project.delete" => kb::graph_project_delete(params),
        "graph.project.analyze" => kb::graph_project_analyze(params),
        // ── 鲸鱼娘桌宠（pet.*，持久化 data_dir/pet_state.json · V2.1 5 方法补齐）──
        "pet.status" => {
            let stored = crate::workbench::pet::status();
            let skin = gateway_pet::get_skin();
            let mood = if agent_is_running() {
                gateway_pet::PetMood::Working
            } else {
                gateway_pet::get_mood()
            };
            let status = gateway_pet::PetStatus {
                skin: skin.as_str().to_string(),
                kind: skin.kind(),
                mood: mood.as_str().to_string(),
                energy: stored.energy,
                affection: stored.affection,
                running: agent_is_running(),
                accent_hex: format!("#{:06x}", skin.accent_hex()),
            };
            serde_json::to_value(status).unwrap_or(json!({}))
        }
        "pet.list" => {
            let list: Vec<gateway_pet::PetSkinEntry> = gateway_pet::SKIN_CATALOG
                .iter()
                .map(|id| gateway_pet::PetSkinEntry::from(*id))
                .collect();
            json!({ "ok": true, "skins": list })
        }
        "pet.setSkin" => {
            let req: gateway_pet::SetSkinReq = match serde_json::from_value(params.clone()) {
                Ok(r) => r,
                Err(e) => return json!({ "ok": false, "error": e.to_string() }),
            };
            let skin = gateway_pet::PetSkinId::parse(&req.skin);
            gateway_pet::set_skin(skin);
            json!({ "ok": true, "skin": skin.as_str() })
        }
        "pet.setMood" => {
            let req: gateway_pet::SetMoodReq = match serde_json::from_value(params.clone()) {
                Ok(r) => r,
                Err(e) => return json!({ "ok": false, "error": e.to_string() }),
            };
            let mood = gateway_pet::PetMood::parse(&req.mood);
            gateway_pet::set_mood(mood);
            json!({ "ok": true, "mood": mood.as_str() })
        }
        "pet.triggerAction" => {
            let req: gateway_pet::TriggerActionReq = match serde_json::from_value(params.clone()) {
                Ok(r) => r,
                Err(e) => return json!({ "ok": false, "error": e.to_string() }),
            };
            let action = match gateway_pet::PetAction::parse(&req.action) {
                Some(a) => a,
                None => return json!({ "ok": false, "error": format!("未知 pet action: {}", req.action) }),
            };
            // 动作 → 情绪映射（单调扩展：允许后续与 workbench::pet 结合推进能量）
            match action {
                gateway_pet::PetAction::Pet | gateway_pet::PetAction::Feed => {
                    // 走原有 workbench::pet 交互链（能量/好感度持久化）；保留兼容入口：
                    // 把 action 字符串喂给 interact（仅接受 pet/feed，对其余返回 Err）
                    let res = crate::workbench::pet::interact(action.as_str());
                    return match res {
                        Ok(v) => {
                            gateway_pet::set_mood(gateway_pet::PetMood::Happy);
                            // R5 契约三同步：前端 `trigger` 以 res.ok 判定成败。
                            // interact 返回的 PetView 无 ok 字段 → 此处统一补齐 ok:true，
                            // 保证 pet.triggerAction 响应 = { ok, mood, energy, affection }。
                            let mut out = serde_json::to_value(v).unwrap_or(json!({}));
                            if let Some(obj) = out.as_object_mut() {
                                obj.insert("ok".to_string(), json!(true));
                            }
                            out
                        }
                        Err(msg) => json!({ "ok": false, "error": msg }),
                    };
                }
                gateway_pet::PetAction::Celebrate => {
                    gateway_pet::set_mood(gateway_pet::PetMood::Excited);
                }
                gateway_pet::PetAction::Wake => {
                    gateway_pet::set_mood(gateway_pet::PetMood::Normal);
                }
                gateway_pet::PetAction::Doze => {
                    gateway_pet::set_mood(gateway_pet::PetMood::Sleepy);
                }
            }
            json!({ "ok": true, "action": action.as_str(), "mood": gateway_pet::get_mood().as_str() })
        }
        "pet.action" => native::pet_action(params),
        // ── 德州扑克游戏大厅（poker.*，全局大厅单例）──
        "poker.state" => native::poker_state(params),
        "poker.create" => native::poker_create(params),
        "poker.join" => native::poker_join(params),
        "poker.start" => native::poker_start(params),
        "poker.action" => native::poker_do_action(params),
        "poker.auto" => native::poker_auto(params),
        // ── 内置小游戏（games.*，Rust 引擎，WebUI/脚本可直接游玩）──
        "games.list" => native::games_list(),
        "games.play" => native::games_play(params),
        // ── 重型运行时（ASR/本地模型）诚实状态：下载源 + 选型指引，不预下载 ──
        "runtime.status" => system::runtime_status(),
        // ── 技能（SkillLoader 枚举本地技能目录，映射 WebUI Skill 契约）──
        "skills.list" => skills::skills_list(ctx),
        "skills.get" => skills::skills_get(ctx, params),
        "skills.reload" => skills::skills_reload(ctx),
        "skills.search" => skills::skills_search(ctx, params),
        "skills.uninstall" => skills::skills_uninstall(ctx, params),
        "skills.create" => skills::skills_create(ctx, params),
        "skills.update" => skills::skills_update(ctx, params),
        "skills.setEnabled" => skills::skills_set_enabled(ctx, params),
        // 技能覆盖：递归扫 `SKILL.md`，如实回报 loaded / skipped
        "skills.coverage" => skills::skills_coverage(ctx),
        // ── Harness 运行台账（SQLite `harness.db` 只读观测面）──
        // 数据源是 `harness::ledger` 的真实表，不做内存假数据；
        // `harness.` 是新前缀，必须走 Rust 分支，绝不代理到 Python 侧车。
        "harness.ledger.status" => harness_rpc::harness_ledger_status(),
        "harness.ledger.query" => harness_rpc::harness_ledger_query(params),
        "harness.ledger.replay" => harness_rpc::harness_ledger_replay(params),
        "harness.ledger.verdict" => harness_rpc::harness_ledger_verdict(params),
        // harness 运行面快照（只读；不构造模型、不落盘写业务数据）
        "harness.status" => harness_rpc::harness_status(),
        // ── 项目工作区（新建任务引导打开/创建项目文件夹；会话绑定项目根目录）──
        "workspaces.list" => crate::gateway::workspaces::list(params),
        "workspaces.open" => crate::gateway::workspaces::open(params),
        "workspaces.create" => crate::gateway::workspaces::create(params),
        // 文件变更台账（前端「文件更改卡片」数据源：Agent 最近真实改动）
        "workspace.file_changes.list" => workspace::file_changes_list(params),
        "artifacts.list" => artifacts::artifacts_list(params),
        // ── 产物台账（Epic A）：Rust 原生实现，不代理 Python 侧车 ──
        "artifacts.get" => artifacts::artifacts_get(params),
        "artifacts.register" => artifacts::artifacts_register(params),
        "artifacts.open_in_system" => artifacts::artifacts_open_in_system(params),
        "artifacts.edit_inline" => artifacts::artifacts_edit_inline(params),
        // ── 桌面自动化（Epic C）：Rust 原生实现（不代理 Python 侧车）──
        // ⚠️ W6.5 的 desktop.a11y / desktop.action / desktop.windows **不在这里**：
        // 它们是阻塞 UIA 调用，只在异步 dispatch 注册（spawn_blocking），
        // 并列入 ASYNC_ONLY_METHODS —— 同步路径不得再留第二份实现。
        "desktop.list_apps" => desktop::desktop_list_apps(),
        "desktop.launch" => desktop::desktop_launch(params),
        // W6.2：附着已打开的应用（handle 可选化后的统一入口），返回可复用句柄
        "desktop.attach" => desktop::desktop_attach(params),
        "desktop.snapshots" => desktop::desktop_snapshots(params),
        "desktop.stop" => desktop::desktop_stop(),
        // 文件变更回滚（前端「文件更改卡片」的拒绝操作：恢复 before 快照）
        "workspace.file_changes.revert" => workspace::file_changes_revert(params),
        // ── 内置工作区编辑器（多文件）：读取 / 写入文本文件 ──
        "workspace.file.read" => workspace::workspace_file_read(params),
        "workspace.file.write" => workspace::workspace_file_write(params),
        // 项目树右键菜单：重命名 / 删除 / 在系统文件管理器中定位
        "workspace.file.rename" => workspace::workspace_file_rename(params),
        "workspace.file.delete" => workspace::workspace_file_delete(params),
        "workspace.file.reveal" => workspace::workspace_file_reveal(params),
        // ── 企业协同与 SSH：主机台账（list/add/remove；probe 走 async）──
        "collab.ssh.list" => collab::collab_ssh_list(),
        "collab.ssh.add" => collab::collab_ssh_add(params),
        "collab.ssh.remove" => collab::collab_ssh_remove(params),
        "workspaces.update" => crate::gateway::workspaces::update(params),
        "workspaces.pin" => crate::gateway::workspaces::pin(params),
        "workspaces.remove" => crate::gateway::workspaces::remove(params),
        "workspaces.history.delete" => crate::gateway::workspaces::history_delete(params),
        // ── 记忆层（ExperienceLedger 直连，替代空台账占位）──
        "memory.search" => gateway_memory::memory_search(params),
        "memory.save" => gateway_memory::memory_save(params),
        "memory.list" => gateway_memory::memory_list(params),
        // 记忆治理：验证 / 真删 / 管理面分页（前端设置页「记忆条目」区块）
        "memory.validate" => gateway_memory::memory_validate(params),
        "memory.delete" => gateway_memory::memory_delete(params),
        "memory.entries" => gateway_memory::memory_entries(params),
        // 记忆管控概览（各层真实计数 + 待验证 + 已软删）
        "memory.stats" => gateway_memory::memory_stats(params),
        // 7 层记忆其余四层（sensory/short_term/long_term/recall）真实 RPC
        "memory.sensory.search" => crate::memory::gateway::sensory_search(params),
        "memory.short_term.search" => crate::memory::gateway::short_term_search(params),
        "memory.long_term.search" => crate::memory::gateway::long_term_search(params),
        "memory.recall.search" => crate::memory::gateway::recall_search(params),
        // 7 层记忆端到端审计视图（真实计数 + RPC 可达性）
        "memory.layers.status" => crate::memory::gateway::layers_status(params),
        // 记忆楼层开关（7 层记忆 + 经验 = 8 层）：输入框旁的「楼层」弹窗读写这里
        "memory.layers.switches.get" => crate::memory::layers::switches_get(params),
        "memory.layers.switches.set" => crate::memory::layers::switches_set(params),
        // 联想记忆网络（突触图）状态（隐私安全：仅计数 + 哈希 id + 权重）
        "memory.synapse.status" => crate::memory::gateway::synapse_status(params),
        // 分层稀疏记忆（flyhash + tiered）只读观测面（W-O3：执行侧在用、此前无观测入口）
        "memory.tiered.status" => crate::memory::gateway::tiered_status(params),
        // 上一轮记忆注入快照：命中几条 / 各层贡献 / 实际注入 token / 是否被预算裁掉。
        // 「记忆到底有没有进模型上下文」此前是黑箱，用户只能凭感觉说它像玩具。
        "memory.inject.last" => crate::gateway::chat::inject_last_snapshot(),
        // 3×3 权限矩阵只读观测面（W-O3：档位 × 工具自主档位的放行/确认/拒绝全表）
        "guard.permission_matrix.status" => crate::guard::permission_matrix::matrix_status(),
        // ── 渠道（最小台账：Rust 无渠道适配器，返回空台账避免 UI 报错）──
        //
        // ⚠️ 这三条只在**侧车不可用**时才会命中（`channels.` 在 PY_ENGINE_PREFIXES 里，
        // 侧车在线时由 Python 的 `rpc_channels.py` 应答）。所以走到这里 == 降级态。
        // 此前「空台账」与「真的没有任何渠道」在响应上**不可区分** ⇒ 用户会以为
        // 渠道配置丢了。这里补**自描述字段**（纯增量，不改既有字段，前端可无感）：
        // `degraded: true` + `degradedReason`，让 UI/排障能一眼看出是离线降级而非真为空。
        "channels.status" => json!({
            "channels": [],
            "boot_id": uuid::Uuid::new_v4().to_string(),
            "degraded": true,
            "degradedReason": "sidecar_unavailable: Rust 无渠道适配器，渠道状态由 Python 侧车提供",
        }),
        "channels.pairings" => json!({
            "pairings": [],
            "degraded": true,
            "degradedReason": "sidecar_unavailable: 配对信息由 Python 侧车提供",
        }),
        "channels.list" => json!({
            "channels": [],
            "degraded": true,
            "degradedReason": "sidecar_unavailable: 渠道清册由 Python 侧车提供",
        }),
        // ── 沙箱策略（Rust-first：侧车不可用时确定性可用，对齐 webui SandboxPolicy 契约）──
        "sandbox.policy.get" => policy_store::policy_get(),
        "sandbox.policy.defaults" => policy_store::policy_defaults(),
        "sandbox.run_mode.preference.get" => policy_store::run_mode_get(),
        "sandbox.run_mode.preference.set" => policy_store::run_mode_set(
            params
                .get("runMode")
                .and_then(Value::as_str)
                .unwrap_or("full"),
        ),
        "sandbox.capability.status" => sandbox::sandbox_capability_status(),
        "sandbox.setup.status" => sandbox::sandbox_setup_status(),
        // `sandbox.setup.ensure`：webui 沙箱「进入安全模式」调用。Rust 未内置沙箱
        // 运行时，直接返回 unavailable（与 setup.status 同源）；不代理 Python——
        // 侧车对该方法挂起 → WS 代理 3s 超时报「Python 侧车响应超时」。
        "sandbox.setup.ensure" => sandbox::sandbox_setup_status(),
        // `sandbox.run_context.set`：webui 会话级运行模式持久化（useChatRunModePreference）。
        // 复用 run_mode 落盘逻辑（safe/full 二态），与 preference.set 一致；不代理 Python。
        "sandbox.run_context.set" => {
            let mode = params
                .get("runMode")
                .and_then(Value::as_str)
                .unwrap_or("full");
            let mut v = policy_store::run_mode_set(mode);
            if let Some(s) = params.get("sessionKey").and_then(Value::as_str) {
                v["sessionKey"] = Value::String(s.to_string());
            }
            v
        }
        // ── 上下文管理 / 压缩（确定性实现，发布 session.event.compaction）──
        // 参数名两种都认：`sessionKey` 是网关惯例，`session` 是前端运行态面板实际发的。
        // 此前只读 `sessionKey` → 前端传 `session` 时静默回落到 "default"，
        // 运行态面板永远显示「上下文占用 0%（0/128k）」（真机截图复现）。
        "context.status" => {
            let key = params
                .get("sessionKey")
                .or_else(|| params.get("session"))
                .and_then(Value::as_str)
                .unwrap_or("default");
            crate::context::session::context_status(key)
        }
        "context.rollback" => {
            let key = params
                .get("sessionKey")
                .or_else(|| params.get("session"))
                .and_then(Value::as_str)
                .unwrap_or("default");
            crate::context::session::context_rollback(key)
        }
        // ── 跨项目会话记忆（检索本机 AI 工具记忆目录）──
        "memory.cross_project.search" => cross_project::cross_project_search(params),
        "memory.cross_project.expand" => cross_project::cross_project_expand(params),
        // ── 行为事件捕获（B3：summary/timeline/observation 三层）──
        "memory.behavior.list" => behavior::behavior_list(params),
        "memory.behavior.summary" => behavior::behavior_summary(params),
        // ── 任务拆解规划（长难复杂任务自动拆解）──
        "plans.analyze" => plan::plans_analyze(params),
        "plans.create" => plan::plans_create(params),
        "plans.list" => plan::plans_list(),
        "plans.run" => plan::plans_run(params),
        "plans.cancel" => plan::plans_cancel(params),
        "plans.revise" => plan::plans_revise(params),
        // M2-C2：计划卡执行前步骤增删改（派生同 plan 新修订，影响后续执行）
        "plans.step.add" => plan::plans_step_add(params),
        "plans.step.remove" => plan::plans_step_remove(params),
        "plans.step.update" => plan::plans_step_update(params),
        // 编写器 Plan 模式开关：Rust 引擎无独立协作模式，按请求确认模式并递增
        // revision，返回协作快照让前端 UI 正常切换（不再代理 Python 死链路）
        "plans.setMode" => {
            let mode = params
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("default");
            let expected = params
                .get("expectedRevision")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            json!({
                "collaboration": {
                    "mode": if mode == "plan" { "plan" } else { "default" },
                    "revision": expected + 1,
                }
            })
        }
        "plans.capabilities" => cache::plans_capabilities(),
        // ── 规划升级（B4：执行前清单 / 动态重规划）──
        // （plans.parallel.run v19 真执行化后由异步 dispatch 独占，不再走同步映射）
        "plans.checklist" => plan::plans_checklist(params),
        "plans.run.advance" => plan::plans_run_advance(params),
        "plans.replan" => plan::plans_replan(params),
        // ── 长时任务（Bloom 长续航：心跳 / 检查点 / 暂停 / 续跑）──
        "long_running.list" => long_running::long_running_list(),
        "long_running.pause" => long_running::long_running_pause(params),
        "long_running.resume" => long_running::long_running_resume(params),
        "long_running.checkpoint" => long_running::long_running_checkpoint(params),
        // ── 信任校准（M2-C5）：三档信任查看/授予/撤销（设置界面）──
        "trust.list" => crate::gateway::trust_rpc::trust_list(),
        "trust.grant" => crate::gateway::trust_rpc::trust_grant(params),
        "trust.revoke" => crate::gateway::trust_rpc::trust_revoke(params),
        "trust.revoke_all" => crate::gateway::trust_rpc::trust_revoke_all(),
        // ── 升级确认（M2-C4）：四场景 escalation 事件两端「需要确认」──
        "escalation.ack" => escalation_rpc::escalation_ack(params),
        "escalation.pending" => escalation_rpc::escalation_pending(),
        // ── 持久任务队列（M2-C6）：跨会话 Backlog 排队/暂停/恢复/取消 ──
        "queue.list" => crate::gateway::queue_rpc::queue_list(),
        "queue.enqueue" => crate::gateway::queue_rpc::queue_enqueue(params),
        "queue.start" => crate::gateway::queue_rpc::queue_start(params),
        "queue.pause" => crate::gateway::queue_rpc::queue_pause(params),
        "queue.resume" => crate::gateway::queue_rpc::queue_resume(params),
        "queue.cancel" => crate::gateway::queue_rpc::queue_cancel(params),
        "queue.move" => crate::gateway::queue_rpc::queue_move(params),
        // ── P2-11 长任务 checkpoint（Rust 侧本地实现，侧车离线也可用）──
        "tasks.checkpoint.list" => crate::gateway::checkpoint_rpc::checkpoint_list(),
        "tasks.checkpoint.abandon" => crate::gateway::checkpoint_rpc::checkpoint_abandon(params),
        "tasks.checkpoint.replay" => crate::gateway::checkpoint_rpc::checkpoint_replay(params),
        // ── 沙箱交互式终端（真实 PTY：read/key/resize/status/shells 同步读取内存态会话）──
        "sandbox.terminal.read" => crate::tools::terminal::terminal_read(params),
        "sandbox.terminal.list" => crate::tools::terminal::terminal_list(),
        "sandbox.terminal.key" => crate::tools::terminal::terminal_key(params),
        "sandbox.terminal.resize" => crate::tools::terminal::terminal_resize(params),
        "sandbox.terminal.status" => crate::tools::terminal::terminal_status(params),
        "sandbox.terminal.shells" => crate::tools::terminal::terminal_shells(),
        // ── 跨平台终端后端探测与一次性执行（WSL / Docker / Native）──
        "terminal.backends.status" => crate::tools::terminal_backend::backends_status(params),
        "terminal.backend.exec" => crate::tools::terminal_backend::backend_exec(params),
        "terminal.backend.auto" => crate::tools::terminal_backend::backend_auto(params),
        // ── AI 智能融合路由（Bloom：编排形态自动决策，离线纯规则）──
        "router.decision" => gateway_router::router_decision(params),
        "router.selflearning.status" => cache::router_selflearning_status(ctx),
        // ── 知识库（KB 2.0：BM25 检索 / 状态 / 灌入，把 kb/ 引擎暴露给 Web UI）──
        "kb.status" => crate::kb::gateway::status(),
        "kb.search" => crate::kb::gateway::search(params),
        "kb.search.hybrid" => crate::kb::gateway::search_hybrid(params),
        "kb.ingest" => crate::kb::gateway::ingest(params),
        "kb.list" => crate::kb::gateway::list(),
        "kb.delete" => crate::kb::gateway::delete(params),
        "kb.ingest_file" => crate::kb::gateway::ingest_file(params),
        // ── 代码智能（CodeIntel：项目索引 + 符号/文本混合检索）──
        "codeintel.status" => crate::codeintel::gateway::status(params),
        "codeintel.index" => crate::codeintel::gateway::index(params),
        "codeintel.search" => crate::codeintel::gateway::search(params),
        "codeintel.symbols" => crate::codeintel::gateway::symbols(params),
        // ── 引导目录 / 状态（Rust 离线回退）──
        "onboarding.catalog" => onboarding::onboarding_catalog(),
        "onboarding.status" => onboarding::onboarding_status(ctx),
        // ── 模型舰队（Fleet 多角色路由）──
        "fleet.list" => fleet::fleet_list(),
        "fleet.route" => fleet::fleet_route(params),
        // ── 自进化历史 ──
        "evolve.history" => fleet::evolve_history(),
        // ── 插件槽位 ──
        "plugins.list" => fleet::plugins_list(),
        "plugins.status" => plugin_status::status_json(),
        "plugins.coverage" => plugins_rpc::plugins_coverage(),
        "plugins.setEnabled" => plugins_rpc::plugins_set_enabled(params),
        "plugins.reload" => plugins_rpc::plugins_reload(params),
        "plugins.detail" => plugins_rpc::plugins_detail(params),
        "plugins.delete" => plugins_rpc::plugins_delete(params),
        // ── DSH 插件市场（本地 16 清单 + 技能市场）──
        "market.list" => market::market_list(),
        "pages.list" => fleet::pages_list(params),
        "pages.add" => fleet::pages_add(params),
        "pages.search" => fleet::pages_search(params),
        "market.search" => market::market_search(params),
        "market.install" => market::market_install(params),
        "market.upgrade" => market::market_upgrade(params),
        "market.audit" => market::market_audit(params),
        "market.import" => market::market_import(params),
        "market.export" => market::market_export(params),
        "market.install.batch" => market::market_install_batch(params),
        "market.sources" => market::market_sources(),
        // deepseek harness 生态浏览目录 + 分类 facet
        "market.harness" => market::market_harness(),
        "market.categories" => market::market_categories(),
        "team.compositions.list" => team::team_compositions_list(),
        "team.compositions.save" => team::team_compositions_save(params),
        "team.compositions.remove" => team::team_compositions_remove(params),
        // 其余写类方法或未分类方法统一 NOT_IMPLEMENTED
        _ => Value::Null,
    }
}

/// 兼容旧同步调用（如单测直接调用）：同步函数直接调用 Rust 部分
pub fn dispatch_sync(
    ctx: &GatewayContext,
    method: &str,
    params: &Value,
) -> Result<Value, RpcError> {
    // 运维只读 ops.*：同步路径同样经 transport-neutral adapter。
    if let Some(result) = crate::gateway::ops_dispatch::dispatch_with_config(method, params, &ctx.config) {
        return result;
    }
    // 画像导入 8 RPC（闭环 INFO/START/STATUS/CANCEL/RETRY/APPLY/UNDO/DISCARD）
    // 同步路径也可调用（测试/CLI），用 match 放到最顶部统一转换 Err(code,msg)。
    match method {
        // 产物列表：同步 Rust 实现（读会话产物目录），同步路径也应可达
        "artifacts.list" => return Ok(artifacts::artifacts_list(params)),
        "memory.import.info" => return Ok(crate::gateway::memory_import::info(params)),
        "memory.import.status" => {
            return crate::gateway::memory_import::status(params)
                .map_err(|(c, m)| RpcError::new(&c, &m));
        }
        "memory.import.cancel" => {
            return crate::gateway::memory_import::cancel(params)
                .map_err(|(c, m)| RpcError::new(&c, &m));
        }
        "memory.import.start" => {
            return crate::gateway::memory_import::start(params)
                .map_err(|(c, m)| RpcError::new(&c, &m));
        }
        "memory.import.retry" => {
            return crate::gateway::memory_import::retry(params)
                .map_err(|(c, m)| RpcError::new(&c, &m));
        }
        "memory.import.apply" => {
            return crate::gateway::memory_import::apply(params)
                .map_err(|(c, m)| RpcError::new(&c, &m));
        }
        "memory.import.undo" => {
            return crate::gateway::memory_import::undo(params)
                .map_err(|(c, m)| RpcError::new(&c, &m));
        }
        "memory.import.discard" => return Ok(crate::gateway::memory_import::discard(params)),
        "session.thinking.get" => return crate::gateway::sessions::thinking_get(params),
        "session.thinking.set" => return crate::gateway::sessions::thinking_set(params),
        _ => {}
    }
    // Goal Round Driver：同步调用同样经独立 adapter，保持错误码一致。
    if method.starts_with("goal.") {
        return crate::gateway::goal::dispatch(method, params.clone());
    }
    // Rust 实现优先（channels.* 与 Python 前缀重叠）
    if RUST_FIRST_METHODS.contains(&method) {
        let v = dispatch_rust(ctx, method, params);
        if !v.is_null() {
            return Ok(v);
        }
    }
    if is_python_method(method) {
        return Err(RpcError::not_implemented(method));
    }
    // 异步方法族（chat.* / 远程安装 / 视觉取证 / 浏览器）仅由异步 dispatch 提供；
    // sync 路径返回结构化 ASYNC_ONLY 错误（而非 NOT_IMPLEMENTED），避免客户端误判「功能缺失」。
    if ASYNC_ONLY_METHODS.contains(&method) {
        return Err(RpcError::new(
            "ASYNC_ONLY",
            format!("方法 {method} 仅由异步 dispatch（WS RPC）提供，sync 路径不可用"),
        ));
    }
    let v = dispatch_rust(ctx, method, params);
    if v.is_null() {
        Err(RpcError::not_implemented(method))
    } else {
        Ok(v)
    }
}

/// `skills.install` 网关入口 → 代理到 skills 模块（分治：审计 / 下载 / 导入 SKILL.md）。
async fn skills_install(ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    skills::skills_install(ctx, params).await
}

/// `mcp.servers.probe` 网关入口 → 代理到 mcp 模块（真实连接 URL MCP 服务器并拉取工具目录）。
async fn mcp_servers_probe(params: &Value) -> Result<Value, RpcError> {
    mcp::mcp_servers_probe(params).await
}

/// 运行时/模型/游戏的契约测试（分治后 symbols 已在 models/system/native 模块）。
#[cfg(test)]
mod games_tests {
    use crate::gateway::models::models_profiles_list;
    use crate::gateway::native::{games_list, games_play};
    use crate::gateway::system::runtime_status;
    use serde_json::json;

    #[test]
    fn games_list_contract() {
        let v = games_list();
        assert_eq!(v["total"], 3);
        let games = v["games"].as_array().unwrap();
        assert_eq!(games.len(), 3);
        for g in games {
            assert_eq!(g["status"], "implemented");
            assert!(!g["id"].as_str().unwrap_or("").is_empty());
        }
    }

    #[test]
    fn models_profiles_list_contract() {
        let v = models_profiles_list();
        assert_eq!(v["ok"], true, "模型档案应可读取: {v}");
        let arr = v["profiles"].as_array().unwrap();
        assert!(!arr.is_empty());
        assert!(arr.iter().any(|p| p["provider"] == "deepseek"));
    }

    #[test]
    fn runtime_status_contract() {
        let v = runtime_status();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2, "应上报 ASR + 本地量化模型两项");
        for item in arr {
            assert_eq!(item["bundled"], false, "重运行时不得预打包");
            assert_eq!(item["status"], "not_installed", "未安装必须如实上报");
            assert!(item["download"].as_str().unwrap_or("").contains("http"));
            assert!(!item["guidance"].as_str().unwrap_or("").is_empty());
        }
    }

    #[test]
    fn games_play_renders_and_errors() {
        let ok = games_play(&json!({ "game": "tictactoe", "moves": ["0", "3", "1", "4", "2"] }));
        assert_eq!(ok["ok"], true, "合法玩法应成功: {ok}");
        assert!(ok["render"].as_str().unwrap_or("").contains("X 获胜"));
        let bad = games_play(&json!({ "game": "nope", "moves": [] }));
        assert_eq!(bad["ok"], false, "未知游戏应据实失败");
        let missing = games_play(&json!({}));
        assert_eq!(missing["ok"], false);
    }
}

/// 本地模型中心 / 成本观测方法族契约：必须经 `dispatch_sync` 可达（注册于 dispatch_rust 且非空）。
#[cfg(test)]
mod local_hub_contract_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    /// 每个用例调用拿到**独立的**临时配置目录。
    ///
    /// 这里原来是一个 `OnceLock<Mutex<PathBuf>>`，语义是「同一进程内所有用例共用同一个目录」，
    /// 而每个 `ctx()` 都会先删掉该目录下的 `pet_state.json`。于是
    /// `pet_trigger_pet_roundtrip_increments_affection` 在「+1 之后重新读」那一步
    /// （本文件 pet.status 断言处）可能刚好被兄弟用例的删除插进来 → `load_store()` 回落到
    /// 默认值 0 → 断言失败。实测：整套并行跑偶发 1 条失败（2535 passed / 1 failed），
    /// 而 `--exact` 单独跑**必过**（0.00s）——这是共享可变状态的竞态，不是产品缺陷。
    /// 改成按调用递增的唯一目录后，用例之间不再共享任何持久化状态。
    fn ctx() -> GatewayContext {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mgt-lh-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::create_dir_all(&dir);
        // 线程局部覆盖须在**每个**用例线程生效，否则非首个线程会回落到真实 ~/.marguerite，
        // 既造成跨用例串扰，也会把用户的桌宠状态写坏。
        Config::set_config_dir_override(dir.clone());
        // 目录是全新的；这两行保留是为了防御「目录名被复用」的情况。
        let _ = std::fs::remove_file(dir.join("models.json"));
        let _ = std::fs::remove_file(dir.join("pet_state.json"));
        GatewayContext {
            config: Config::default(),
            token: "t".into(),
            started_at: Instant::now(),
            bus: Arc::new(EventBus::default()),
        }
    }

    #[test]
    fn offline_cron_list_returns_the_wire_array_with_iso_next_run() {
        let c = ctx();
        let value = dispatch_sync(&c, "cron.list", &json!({})).expect("cron.list");
        let items = value
            .as_array()
            .unwrap_or_else(|| panic!("cron.list 必须是数组（前端按数组读）: {value}"));
        for job in items {
            assert!(job.get("next_run").is_some(), "缺少 ISO next_run: {job}");
            assert!(job.get("scheduleKind").is_some(), "缺少 scheduleKind: {job}");
            assert_eq!(job["payloadKind"], "reminder");
            assert_eq!(job["source"], "rust_schedule_store");
        }
    }

    #[test]
    fn sidecar_only_commands_are_no_longer_faked_and_are_not_rust_first() {
        // W2 回归：这两个方法此前在 Rust 侧回 `{ok:true}` 但**没有任何副作用**
        // （sandbox.resume 不清 denial-pause；skills.deps.install 不装依赖）。
        // 现在它们必须：不在 RUST_FIRST、只在异步代理可达、dispatch_rust 返回 Null
        // （→ 在线代理到侧车，离线诚实 NOT_IMPLEMENTED）。
        for method in ["sandbox.resume", "skills.deps.install"] {
            assert!(
                !RUST_FIRST_METHODS.contains(&method),
                "{method} 不应再被 Rust 抢先实现"
            );
            assert!(
                ASYNC_ONLY_METHODS.contains(&method),
                "{method} 必须只经异步代理可达"
            );
            assert!(
                dispatch_rust(&ctx(), method, &json!({})).is_null(),
                "{method} 的 Rust 假实现必须已删除（返回 Null 才会走代理）"
            );
        }
        // 会话订阅仍走 Rust fast ack，但必须如实声明「未完整水合」。
        let sub = dispatch_sync(
            &ctx(),
            "sessions.messages.subscribe",
            &json!({ "key": "agent:main:webchat:w2" }),
        )
        .expect("subscribe");
        assert_eq!(sub["hydration_complete"], false, "{sub}");
        assert_eq!(sub["degraded"], true, "{sub}");
    }

    #[test]
    fn local_hub_and_cost_methods_reachable() {
        let c = ctx();
        for (m, p) in [
            ("models.local.catalog", json!({})),
            (
                "models.local.recommend",
                json!({ "intent": "reasoning", "ramGB": 32, "gpu": true }),
            ),
            ("models.local.status", json!({})),
            ("usage.cost", json!({})),
        ] {
            let v = dispatch_sync(&c, m, &p).expect(m);
            assert!(!v.is_null(), "{m} 返回空");
        }
        // 幂等只读：本地模型中心目录应含 shipped gate
        let cat = dispatch_sync(&c, "models.local.catalog", &json!({})).unwrap();
        assert!(cat["items"].as_array().unwrap().len() >= 8);
        let first = &cat["items"][0];
        assert!(first["shipped_gate"]["weights"] == true);
        assert!(!first["shipped_gate"]["license"]
            .as_str()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn pet_trigger_pet_roundtrip_increments_affection() {
        // V2.2-E2E 后端验证：pet.triggerAction("pet") 应返回 {mood,energy,affection}
        // 且好感度 +1；随后 pet.status 能读到同一持久化状态（前端「顺毛抚摸」往返依赖此契约）。
        let c = ctx();
        let before = dispatch_sync(&c, "pet.status", &json!({})).unwrap();
        let base_affection = before["affection"].as_u64().unwrap_or(0);

        let v = dispatch_sync(&c, "pet.triggerAction", &json!({ "action": "pet" })).unwrap();
        assert_eq!(v["ok"], true, "triggerAction 应 ok，got {v}");
        assert!(v["energy"].as_u64().is_some(), "返回应含 energy");
        assert_eq!(
            v["affection"].as_u64(),
            Some(base_affection + 1),
            "好感度应 +1（持久化），got {v}"
        );

        let after = dispatch_sync(&c, "pet.status", &json!({})).unwrap();
        assert_eq!(
            after["affection"].as_u64(),
            Some(base_affection + 1),
            "pet.status 应读到同一持久化好感度"
        );
    }

    #[test]
    fn pet_trigger_unknown_action_returns_ok_false() {
        let c = ctx();
        let v = dispatch_sync(&c, "pet.triggerAction", &json!({ "action": "fly" })).unwrap();
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap_or("").contains("未知"));
    }

    /// Phase 2 双写桥回归：侧车在线时 `config.patch` 由 Python 写 `config.toml`，
    /// 必须把同一载荷镜像进 Rust 执行侧的 `config.yaml` —— 否则设置页「保存成功」，
    /// 计划/记忆/命名等引擎行为毫无变化（用户口径的「开了没生效」）。
    #[test]
    fn config_patch_mirrors_python_write_into_config_yaml() {
        let c = ctx();
        // 形状与 pyengine `rpc_config._handle_config_patch` 的成功返回一致
        let py = Ok(json!({
            "patched": ["naming.enabled"],
            "changed": true,
            "restartRequired": false,
        }));
        let params = json!({ "patches": { "naming.enabled": false } });
        let res = mirror_config_write(&c, py, &params).expect("镜像不应失败");
        assert_eq!(res["mirrored"], true, "{res}");
        let cfg = Config::load().expect("config.yaml 应可读");
        assert!(!cfg.naming.enabled, "config.yaml 未镜像: {cfg:?}");
    }

    /// 镜像结果的 `restartRequired` / `linked` 必须与 Python 结果**合并**（取并集），
    /// 因为「需要重启」的进程级缓存只有 Rust 侧知道。
    #[test]
    fn config_patch_mirror_merges_restart_required_and_linked() {
        let c = ctx();
        // 前置隔离：本用例依赖 `false → true` 的真变更才能产生 restartRequired。
        // 用户真实 config.yaml 可能已经是 true（用户或其它用例写过），不复位就会在
        // 这类机器上恒假红（2026-10-25 实测：本机 router.enabled 已是 true）。
        let _ = crate::gateway::config::config_patch_local(
            &c,
            &json!({ "patches": { "squilla_router.self_learning.enabled": false } }),
        );
        // Python 只回自己的联动（memory.dream.enabled），不带 restartRequired
        let py = Ok(json!({
            "patched": ["squilla_router.self_learning.enabled"],
            "linked": ["memory.dream.enabled"],
            "changed": true,
        }));
        let params = json!({ "patches": { "squilla_router.self_learning.enabled": true } });
        let res = mirror_config_write(&c, py, &params).unwrap();
        assert_eq!(res["restartRequired"], true, "{res}");
        let linked: Vec<&str> = res["linked"]
            .as_array()
            .expect("linked 应存在")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(linked.contains(&"memory.dream.enabled"), "{linked:?}");
        assert!(
            linked.contains(&"memory.dream.auto_schedule"),
            "Rust 侧联动缺失: {linked:?}"
        );
        let mut uniq = linked.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), linked.len(), "linked 应去重: {linked:?}");
    }

    /// Python 拒绝（校验失败/超时）时**不得**镜像：两套配置要么一起前进，要么都不动。
    #[test]
    fn config_patch_python_failure_does_not_touch_config_yaml() {
        let c = ctx();
        let params = json!({ "patches": { "naming.enabled": false } });
        let res = mirror_config_write(&c, Err(RpcError::new("BOOM", "python 拒绝")), &params);
        assert!(res.is_err());
        let cfg = Config::load().expect("config.yaml 应可读");
        assert!(cfg.naming.enabled, "Python 失败时不得镜像: {cfg:?}");
    }
}

#[cfg(test)]
mod r72_config_overlay_tests {
    use super::*;

    /// R72：`config.get` 读侧必须把 **Rust 引擎自有段**（model / model_routing / bloom）
    /// 覆盖回 Python 视图。双源实测：侧车 config.toml 没有这三段 → 设置页
    /// 「模型路由不显示」「添加模型保存了却看不见」。
    #[test]
    fn config_get_overlays_engine_owned_sections_over_python_view() {
        let dir = std::env::temp_dir().join(format!("marg_overlay_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        crate::persistence::config::Config::set_config_dir_override(dir.clone());
        // Rust 真源：带 model + model_routing（与侧车 toml 缺失形成对照）
        std::fs::write(
            dir.join("config.yaml"),
            "model:\n  provider: deepseek\n  model_name: test-model-x\nmodel_routing:\n  mode: omni\n",
        )
        .unwrap();

        let ctx = GatewayContext {
            config: crate::persistence::config::Config::load().unwrap(),
            token: "t".into(),
            started_at: std::time::Instant::now(),
            bus: std::sync::Arc::new(crate::events::EventBus::default()),
        };
        // Python 视图：没有 model/model_routing，只有它自己的段
        let py = json!({ "squilla_router": { "enabled": true } });
        let out = overlay_engine_owned_sections(&ctx, Ok(py)).unwrap();
        // 契约：读侧必须与「引擎将加载的配置」自洽（测试进程可能带 .env 注入，
        // 故断言对象是 Config::load() 的结果而非字面量——那才是引擎真正会用的值）。
        let expected = crate::persistence::config::Config::load().unwrap();
        assert_eq!(
            out["model"]["model_name"],
            serde_json::to_value(&expected.model).unwrap_or_default()["model_name"],
            "model 段必须被 Rust 真源覆盖"
        );
        assert_eq!(
            out["model_routing"]["mode"],
            serde_json::to_value(&expected.model_routing).unwrap_or_default()["mode"],
            "model_routing 段必须出现（此前恒缺 → 模型路由不显示）"
        );
        assert!(
            out["modelRouting"].is_object(),
            "camelCase 兼容键必须同步提供"
        );
        // Python 自有段不得被破坏
        assert_eq!(out["squilla_router"]["enabled"], true);

        // 错误透传：Python 失败绝不能被吞成空成功
        let err = overlay_engine_owned_sections(
            &ctx,
            Err(RpcError::new("PY_DOWN", "sidecar offline")),
        );
        assert_eq!(err.unwrap_err().code, "PY_DOWN");

        crate::persistence::OVERRIDE_CONFIG_DIR.with(|c| *c.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod r5_ledger_guard_tests {
    use super::*;
    use crate::gateway::protocol::SUPPORTED_METHODS;

    /// 本次新增的 12 个方法（`skills.coverage` + `harness.ledger.*` + `plugins.*` + `task.receipt`）
    const R5_METHODS: &[&str] = &[
        "skills.coverage",
        "harness.ledger.status",
        "harness.ledger.query",
        "harness.ledger.replay",
        "plugins.coverage",
        "plugins.setEnabled",
        "plugins.reload",
        "plugins.mcp.start",
        "plugins.mcp.stop",
        "plugins.mcp.list",
        "plugins.mcp.call",
        // 交付回执：声明 / Rust 优先 / dispatch 臂三处必须同时存在
        "task.receipt",
    ];

    /// R5 三同步守卫：① `SUPPORTED_METHODS`（前端可见）② `RUST_FIRST_METHODS`
    /// （不被代理到 Python 侧车）③ `dispatch_rust` 的 match 臂（真正可达）。
    ///
    /// 只查声明不查实现，就会出现「宣称支持但调用返回 NOT_IMPLEMENTED」的假交付。
    #[test]
    fn r5_methods_declared_rust_first_and_reachable() {
        let src = include_str!("methods.rs");
        for method in R5_METHODS {
            assert!(
                SUPPORTED_METHODS.contains(method),
                "{method} 未在 SUPPORTED_METHODS 声明（前端 supportsMethod 会判 false）"
            );
            assert!(
                RUST_FIRST_METHODS.contains(method),
                "{method} 未列入 RUST_FIRST_METHODS（会被代理到 Python 侧车）"
            );
            let arm = format!("\"{method}\" =>");
            assert!(
                src.contains(&arm),
                "{method} 在 dispatch_rust 缺少匹配臂（宣称≠可达）"
            );
        }
    }

    /// 守卫自身不得退化成空集
    #[test]
    fn r5_guard_covers_all_new_methods() {
        assert_eq!(R5_METHODS.len(), 12);
        assert!(RUST_FIRST_METHODS.len() > R5_METHODS.len());
    }
}

/// W6.5 桌面 RPC 的分派契约守卫。
///
/// 三处必须同时成立：① `SUPPORTED_METHODS`（前端 `supportsMethod` 可见）
/// ② `ASYNC_ONLY_METHODS`（sync 路径明确拒绝，而不是阻塞执行 UIA）
/// ③ 异步 dispatch 臂（真正可达）。另外钉住「`dispatch_rust` 不再有同步副本」——
/// UIA 单次最长 10s，留在 tokio worker 上会卡住整条 WS 通道。
#[cfg(test)]
mod desktop_rpc_contract_tests {
    use super::*;
    use crate::gateway::protocol::SUPPORTED_METHODS;

    const DESKTOP_ASYNC_METHODS: &[(&str, &str)] = &[
        ("desktop.a11y", "desktop_a11y_rpc"),
        ("desktop.action", "desktop_action_rpc"),
        ("desktop.windows", "desktop_windows_rpc"),
    ];

    fn ctx() -> GatewayContext {
        GatewayContext {
            config: Config::default(),
            token: String::new(),
            started_at: Instant::now(),
            bus: Arc::new(EventBus::default()),
        }
    }

    #[test]
    fn desktop_rpcs_are_declared_async_only_and_wired_to_the_async_dispatch() {
        let src = include_str!("methods.rs");
        for &(method, wrapper) in DESKTOP_ASYNC_METHODS {
            assert!(
                SUPPORTED_METHODS.contains(&method),
                "{method} 未在 SUPPORTED_METHODS 声明（前端 supportsMethod 会判 false）"
            );
            assert!(
                ASYNC_ONLY_METHODS.contains(&method),
                "{method} 未列入 ASYNC_ONLY_METHODS（sync 路径会静默执行阻塞 UIA）"
            );
            assert!(
                !RUST_FIRST_METHODS.contains(&method),
                "{method} 不应由 RUST_FIRST 的同步实现抢先应答"
            );
            // 用运行期拼接的针，避免测试源码里的字面量把断言自己喂绿。
            let arm = format!("\"{method}\" => return {wrapper}(");
            assert!(
                src.contains(&arm),
                "{method} 缺少异步 dispatch 臂（应形如 {arm}）"
            );
            assert!(
                dispatch_rust(&ctx(), method, &json!({})).is_null(),
                "{method} 仍在 dispatch_rust 保留同步实现（会阻塞 async worker）"
            );
        }
    }

    #[test]
    fn desktop_rpcs_reject_the_sync_path_with_async_only() {
        for &(method, _) in DESKTOP_ASYNC_METHODS {
            let err = dispatch_sync(&ctx(), method, &json!({})).expect_err("sync 路径必须拒绝");
            assert_eq!(err.code, "ASYNC_ONLY", "{method} 的 sync 拒绝码不对: {err:?}");
        }
    }
}
