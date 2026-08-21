//! 网关方法路由与首批实现
//!
//! 方法名对齐 Marguerite 协议（HelloOk `features.methods` 只暴露已实现者）。
//! Phase 2 交付「状态 / 只读」方法族（健康、配置、沙箱、会话、工具目录）；
//! Phase 3 交付「§5.2 Python 方法代理」（channels/router/meta/onboarding/wizard/secrets/memory/search/artifacts 等）：
//! - 若 Python 侧车可用（[`server::py_client`] 返回 Some），转发调用
//! - 否则返回 `NOT_IMPLEMENTED`，UI 层据此 `markMethodUnavailable`

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use anyhow::Result;
use serde_json::{json, Value};

use crate::events::{Envelope, EventBus, EventType};
use crate::gateway::chat;
use crate::gateway::protocol::{RpcError, PROTOCOL_VERSION};
use crate::gateway::server;
use crate::mcp::registry::{ToolInfo, ToolRegistry};
use crate::memory::behavior;
use crate::memory::cross_project;
use crate::memory::ExperienceLedger;
use crate::orchestration::Orchestrator;
use crate::persistence::config::Config;
use crate::persistence::session::SessionManager;
use crate::plan;
use crate::pyengine::client::PyGatewayClient;
use crate::sandbox::policy_store;
use crate::sandbox::whitelist;
use crate::skills::loader::SkillLoader;
use crate::tools::builtin::register_builtin_tools;
// 桌面自动化（Epic C）：启动 / 动作 / 录制 / 闸门。仅类型导入，COM 实现隔离在
// `desktop_automation::com`（cfg(windows)），非 Windows 下 AppRegistry 不注册 WpsPpt。
use crate::desktop_automation::{AppKind, AppRegistry, DesktopError, Policy, Recorder, UiAction};

/// AI 最近一次活动时间戳（毫秒）：chat.send 时标记，供 `pet.status` 判定「工作中」态
static LAST_AGENT_ACTIVITY_MS: AtomicI64 = AtomicI64::new(0);

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
];

fn is_python_method(method: &str) -> bool {
    PY_ENGINE_PREFIXES.iter().any(|pre| method.starts_with(pre))
}

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
];

/// 网关上下文（每个网关进程一份，跨连接共享）
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
    // 目录浏览 / 选择 / 新建（pick 为 async 子进程对话框；list/create-directory
    // 虽为 std::fs 同步操作，但统一在 async dispatch 中提供，避免 sync/async 双份实现）
    "sandbox.path.pick",
    "sandbox.path.list",
    "sandbox.path.create-directory",
    // 电脑操控：computer_use.action 是 async-only（await 真实屏幕感知/键鼠控制），
    // 仅异步 dispatch 可达；dispatch_sync 无 async 能力，故归入 async-only，
    // 由 test_supported_methods_subset_of_dispatch 跳过同步校验（客户端走 WS 异步 RPC）。
    "computer_use.action",
    // modlens 视觉取证为 async（await 视觉模型），仅异步 dispatch 可达
    "modlens.analyze",
];

/// Rust 实现优先的方法：即使命中 Python 前缀，也先走 Rust 实现。
///
/// 融合后 `channels.status / pairings / list` 在 Rust 网关内提供最小台账，
/// 不再被 `channels.` 前缀代理到（不可用的）Python 侧车。
const RUST_FIRST_METHODS: &[&str] = &[
    "channels.status",
    "channels.pairings",
    "channels.list",
    "router.decision",
    // 自学习状态（Rust 离线基线：显示 enabled/gate/样本占位；真训练由
    // Python 侧车 + dream cadence 推进；Rust 版本不报 NOT_IMPLEMENTED，
    // 让「高级选项 › 记忆与自学习」组 UI 可用）
    "router.selflearning.status",
    // 沙箱策略 / 会话快照：Rust 确定性实现，避免 Python 侧车慢响应阻塞 WS 事件循环
    "sandbox.policy.get",
    "sandbox.policy.defaults",
    "sandbox.policy.update",
    "sandbox.run_mode.preference.get",
    "sandbox.run_mode.preference.set",
    "sandbox.capability.status",
    "sandbox.setup.status",
    "sessions.messages.snapshot",
    "sessions.messages.hydrate",
    // 上下文管理 / 压缩（Rust 确定性实现，离线可用；compact 走 async LLM 分支）
    "context.status",
    "context.rollback",
    // 跨项目会话记忆（检索本机 AI 工具记忆目录；B3 升级：向量语义 + 渐进披露 + 行为捕获）
    "memory.cross_project.search",
    "memory.cross_project.expand",
    "memory.behavior.list",
    "memory.behavior.summary",
    // 任务拆解规划（长难复杂任务自动拆解）
    "plans.analyze",
    "plans.create",
    "plans.list",
    "plans.run",
    "plans.cancel",
    "plans.implement",
    "plans.revise",
    "plans.setMode",
    // 规划升级（B4）：执行前清单 / 动态重规划 / 并行子任务
    "plans.checklist",
    "plans.run.advance",
    "plans.replan",
    "plans.parallel.run",
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
    // 引导目录 / 状态（Rust 离线回退：侧车不可用时返回内置 provider 目录与最小状态，
    // 让设置页 / 渠道编辑器在无 Python 侧车时仍可渲染）
    "onboarding.catalog",
    "onboarding.status",
    "onboarding.router.configure",
    "onboarding.ensemble.configure",
    "onboarding.provider.probe",
    "onboarding.provider.configure",
    "agent.profile.auto",
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
    "pet.status",
    "pet.action",
    "poker.create",
    "poker.join",
    "poker.start",
    "poker.action",
    "poker.state",
    "poker.auto",
];

/// 方法分派入口（异步）：按方法名路由到 Rust 实现或 Python 侧车代理
pub async fn dispatch(
    ctx: &GatewayContext,
    method: &str,
    params: Value,
) -> Result<Value, RpcError> {
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
        _ => {}
    }
    // 异步方法族：需后台任务 / 网络，直接在异步上下文实现
    match method {
        "chat.send" => {
            mark_agent_active();
            return chat::send(ctx, &params).await;
        }
        "chat.abort" => return chat::abort(ctx, &params).await,
        "skills.install" => return skills_install(ctx, &params).await,
        // modlens 视觉取证（图片 base64 → 结构化 JSON；无视觉模型降级元数据兜底）
        "modlens.analyze" => return Ok(modlens_analyze(&params).await),
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
            return Ok(crate::tools::terminal::terminal_spawn(&params).await);
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
        // 方案实施：既有修订按步骤真实执行（前端「实施计划」按钮）。数据源为
        // Rust 计划系统 plans.json，而非 Python 侧车 plan_revisions 表。
        "plans.implement" => return plans_implement(ctx, &params).await,
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
        "mode.switch" => return mode_switch(ctx, &params).await,
        // 模型路由模式切换：Rust 原生实现（direct/router/ensemble/bloom）。
        // 此前仅 Python 侧车实现（PY_ENGINE_PREFIXES 含 models.routing.），侧车离线时
        // 返回 NOT_IMPLEMENTED → webui 报「切换模型路由失败，请检查网关连接」。现改为
        // Rust 原生，持久化 config.yaml 的 model_routing.mode，无需侧车即可切换。
        "models.routing.set" => return models_routing_set(ctx, &params).await,
        "models.routing.get" => return Ok(models_routing_get(ctx)),
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
        // ── 电脑操控（computer_use：三层架构屏幕感知/键鼠控制/GUI定位）──
        "computer_use.action" => {
            let mut args: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            if let Some(obj) = params.as_object() {
                for (k, v) in obj {
                    if let Some(s) = v.as_str() {
                        args.insert(k.clone(), s.to_string());
                    } else {
                        args.insert(k.clone(), v.to_string().trim_matches('"').to_string());
                    }
                }
            }
            return match crate::tools::computer_use::dispatch(args, None).await {
                Ok(crate::agent::tool_cycle::ToolResult::Success { data, .. }) => {
                    Ok(json!({ "ok": true, "data": data }))
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
        _ => {}
    }
    // Python 优先（读写对称）：config.* 全量读写由 Python 侧车提供完整字段。
    // 侧车在线时先代理，保证 config.patch.safe（写）与 config.get（读）同源，
    // 修复设置页开关保存后自动回弹；离线时回退下方 Rust 精简实现 / NOT_IMPLEMENTED。
    if PY_FIRST_METHODS.contains(&method) {
        if let Some(c) = server::py_client() {
            return py_call_timeout(c, method.to_string(), params).await;
        }
    }
    // Rust 实现优先（channels.* 与 Python 前缀重叠）
    if RUST_FIRST_METHODS.contains(&method) {
        // onboarding.provider.probe 是 async（HTTP 探测），在 async 上下文中直接调用
        if method == "onboarding.provider.probe" {
            return Ok(onboarding_provider_probe(&params).await);
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
        return Err(RpcError::not_implemented(method));
    }
    let res = dispatch_rust(ctx, method, &params);
    if res.is_null() {
        // Rust 未实现但 Python 侧车在线 → 自动全量代理。
        // 这打通 skills./memory./agents./sandbox./usage./cron./chat./tools.* 等
        // webui 依赖但未显式列入 PY_ENGINE_PREFIXES 的全部 Python 方法组。
        if let Some(c) = server::py_client() {
            return py_call_timeout(c, method.to_string(), params).await;
        }
        return Err(RpcError::not_implemented(method));
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
        crate::context::session::record_turn(
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

/// `sandbox.path.pick`：桌面端原生目录选择器（Rust 本地实现）。
///
/// 历史问题：该 RPC 未在 Rust 实现 → dispatch 回退代理 Python 侧车；侧车在
/// Windows 上 spawn 一个 tkinter 子进程弹「选择目录」对话框，且以
/// CREATE_NO_WINDOW 方式创建，在无窗口会话下永久阻塞 → WS 代理超时
/// （「Python 侧车响应超时（sandbox.path.pick）」），目录选择器无法打开。
///
/// 现改为 Rust 直接以 PowerShell + System.Windows.Forms.FolderBrowserDialog
/// 弹原生目录对话框（STA 线程，与 system_control 同源的纯系统 API）：
/// - 不依赖 Python 侧车（离线也可用）；
/// - 异步 await 子进程，不阻塞网关事件循环；
/// - 用户取消 / 无桌面会话 → 返回 `path: null`，WebUI 回退到路径浏览。
async fn sandbox_path_pick(params: &Value) -> Result<Value, RpcError> {
    if cfg!(not(target_os = "windows")) {
        return Ok(json!({ "path": null, "kind": "workspace" }));
    }
    let initial = params
        .get("initialPath")
        .and_then(Value::as_str)
        .unwrap_or("");
    let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
Add-Type -AssemblyName System.Windows.Forms | Out-Null
$d = New-Object System.Windows.Forms.FolderBrowserDialog
$d.Description = 'Marguerite · 选择项目目录'
$d.ShowNewFolderButton = $true
$initial = $env:MARG_PICK_INITIAL
if ($initial) { try { $d.SelectedPath = $initial } catch {} }
if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    # 以 UTF-8 裸字节写出选中路径，规避 PowerShell 控制台默认 GBK(codepage 936)
    # 编码导致的中文路径乱码：Rust 侧用 from_utf8_lossy 按 UTF-8 读取 stdout。
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($d.SelectedPath + [char]10)
    $stdout = [System.Console]::OpenStandardOutput()
    $stdout.Write($bytes, 0, $bytes.Length)
    $stdout.Flush()
}
"#;
    let mut cmd = tokio::process::Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-STA",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    cmd.env("MARG_PICK_INITIAL", initial);
    // 目录对话框可能长时间打开（用户人工选择），上限 15 分钟
    let out = tokio::time::timeout(std::time::Duration::from_secs(900), cmd.output())
        .await
        .map_err(|_| RpcError::new("PICK_TIMEOUT", "目录选择超时（15 分钟内未完成选择）"))?
        .map_err(|e| RpcError::new("PICK_SPAWN", format!("无法启动目录选择器: {e}")))?;
    if !out.status.success() {
        return Ok(json!({ "path": null, "kind": "workspace" }));
    }
    let selected = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(json!({
        "path": if selected.is_empty() { Value::Null } else { Value::String(selected) },
        "kind": "workspace",
    }))
}

/// 解析 `sandbox.path.*` 的目标根目录：优先 HOME/USERPROFILE，回退进程 cwd。
fn sandbox_path_root() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("USERPROFILE") {
        if !p.is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    if let Ok(p) = std::env::var("HOME") {
        if !p.is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    std::path::PathBuf::from(".")
}

/// `sandbox.path.list`：Rust 本地目录浏览（项目工作区选择器「平台目录」加载）。
///
/// 请求：`{ sessionKey, kind?, path?, basePath? }`
/// 响应：`{ currentPath, path, parentPath, systemPickerAvailable, entries[] }`
/// 其中 `entries[]` 每项含 `{ name, path, kind: 'directory'|'file', selectable, hidden? }`，
/// 对齐 webui `SandboxPathListResponse` / `SandboxPathEntry`（`webui/src/types/rpc.ts`）。
///
/// 此前该 RPC 未在 Rust 实现 → dispatch 回退 NOT_IMPLEMENTED，UI 报
/// 「无法加载平台目录：方法 `sandbox.path.list` 尚未在 Rust 网关实现」。
async fn sandbox_path_list(params: &Value) -> Result<Value, RpcError> {
    let raw_path = params
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let raw_base = params
        .get("basePath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let target: std::path::PathBuf =
        if !raw_path.is_empty() && std::path::Path::new(raw_path).is_absolute() {
            std::path::PathBuf::from(raw_path)
        } else if !raw_path.is_empty() && !raw_base.is_empty() {
            std::path::Path::new(raw_base).join(raw_path)
        } else if !raw_base.is_empty() {
            std::path::PathBuf::from(raw_base)
        } else if !raw_path.is_empty() {
            sandbox_path_root().join(raw_path)
        } else {
            sandbox_path_root()
        };

    if !target.exists() {
        return Err(RpcError::new(
            "PATH_NOT_FOUND",
            format!("目录不存在：{}", target.to_string_lossy()),
        ));
    }
    if !target.is_dir() {
        return Err(RpcError::new(
            "NOT_A_DIRECTORY",
            format!("路径不是目录：{}", target.to_string_lossy()),
        ));
    }

    let mut entries: Vec<Value> = Vec::new();
    match std::fs::read_dir(&target) {
        Ok(rd) => {
            for entry in rd.filter_map(Result::ok) {
                let fname = entry.file_name().to_string_lossy().to_string();
                let fpath = entry.path();
                let is_dir = fpath.is_dir();
                entries.push(json!({
                    "name": fname,
                    "path": fpath.to_string_lossy(),
                    "kind": if is_dir { "directory" } else { "file" },
                    "selectable": is_dir,
                    "hidden": fname.starts_with('.'),
                }));
            }
        }
        Err(e) => {
            return Err(RpcError::new(
                "READ_DIR_FAILED",
                format!("无法读取目录 {}：{e}", target.to_string_lossy()),
            ));
        }
    }
    // 目录优先、再按名称排序（webui 亦会自行排序，此处仅提升首屏体验）
    entries.sort_by(|a, b| {
        let da = a.get("kind").and_then(Value::as_str) == Some("directory");
        let db = b.get("kind").and_then(Value::as_str) == Some("directory");
        match (da, db) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a
                .get("name")
                .and_then(Value::as_str)
                .cmp(&b.get("name").and_then(Value::as_str)),
        }
    });

    let current = target.to_string_lossy().to_string();
    let parent = target.parent().map(|p| p.to_string_lossy().to_string());
    Ok(json!({
        "currentPath": current,
        "path": current,
        "parentPath": parent,
        "systemPickerAvailable": cfg!(target_os = "windows"),
        "entries": entries,
    }))
}

/// `sandbox.path.create-directory`：Rust 本地新建子目录（工作区选择器「新建文件夹」）。
///
/// 请求：`{ sessionKey, parentPath?, name, kind? }` → 响应：`{ path }`
async fn sandbox_path_create_directory(params: &Value) -> Result<Value, RpcError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        return Err(RpcError::new("INVALID_REQUEST", "name 不可为空"));
    }
    let raw_parent = params
        .get("parentPath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let base: std::path::PathBuf = if raw_parent.is_empty() {
        sandbox_path_root()
    } else {
        std::path::PathBuf::from(raw_parent)
    };
    let created = base.join(name);
    std::fs::create_dir_all(&created).map_err(|e| {
        RpcError::new(
            "MKDIR_FAILED",
            format!("创建目录失败 {}：{e}", created.to_string_lossy()),
        )
    })?;
    Ok(json!({ "path": created.to_string_lossy().to_string() }))
}

/// `sandbox.capability.status`（对齐 webui `SandboxCapabilityReport`）
///
/// 如实区分布（根绝「安全模式不可用」的误导文案）：
/// - **策略层（policy_store）**：安全模式的文件/命令/网络护栏由 Rust 网关提供，**恒可用** → `available=true`、`code="ready"`。
/// - **OS 级隔离运行时**（Windows Job 对象 / 浏览器沙箱）：未内置 → 单独 `isolation="not_bundled"` 标注，不混淆。
/// - **内置浏览器桥**（CDP，需本机 Chromium）：探测后纳入 `capabilities`（诚实标注）。
///
/// 旧实现恒 `available=false`，导致 UI 徽标误显示「安全模式不可用」——策略层实际可用。
fn sandbox_capability_status() -> Value {
    let browser_ready = crate::browser::BrowserBridge::new().is_available();
    let mut capabilities = vec!["policy".to_string()];
    if browser_ready {
        capabilities.push("browser".to_string());
    }
    json!({
        "available": true,
        "backend": "policy-only",
        "platform": std::env::consts::OS,
        "code": "ready",
        "isolation": "not_bundled",
        "reason": if browser_ready {
            "安全模式策略层（文件/命令/网络护栏）已就绪；OS 级隔离运行时未内置（内置浏览器桥可用，见 browser.status）"
        } else {
            "安全模式策略层（文件/命令/网络护栏）已就绪；OS 级隔离运行时未内置"
        },
        "setupSupported": false,
        "restartRequired": false,
        "probeVersion": 1,
        "capabilities": capabilities,
    })
}

/// `sandbox.setup.status`（对齐 webui `SandboxSetupStatusPayload`）
fn sandbox_setup_status() -> Value {
    json!({
        "state": "unavailable",
        "platform": std::env::consts::OS,
        "message": "Rust 网关未内置沙箱运行时（策略层可用）",
        "requiresAdmin": false,
        "detail": "sandbox.policy.* / sandbox.run_mode.* 已由 Rust 网关提供",
    })
}

/// 宝塔面板凭据文件路径：`~/.marguerite/btpanel-credentials.json`
/// （与 btpanel-mcp `BaotaConfig.resolved_credentials_path` 默认一致）。
fn baota_credentials_path() -> PathBuf {
    Config::config_dir().join("btpanel-credentials.json")
}

/// 智能体台账错误码 → RpcError（客户端 `error.code` 据此识别 agent.exists 等）。
fn agent_rpc_err(code: String) -> RpcError {
    let msg = match code.as_str() {
        "agent.exists" => "Agent already exists",
        "agent.not_found" => "Agent not found",
        "agent.builtin_immutable" => "Builtin agent is immutable",
        _ => &code,
    };
    RpcError::new(&code, msg)
}

/// `baota.credentials.set`：保存面板地址/账号/密码到凭据文件。
///
/// btpanel-mcp 启动或首次调用工具时读取该文件并自动登录，WebUI 由此实现
/// 「调用前配置宝塔地址与账号密码」。密码不回读（get 不暴露）。
fn baota_credentials_set(params: &Value) -> Value {
    let base_url = params
        .get("base_url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let username = params
        .get("username")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let password = params.get("password").and_then(Value::as_str).unwrap_or("");
    if base_url.is_empty() || username.is_empty() || password.is_empty() {
        return json!({ "ok": false, "msg": "base_url、username、password 均不能为空" });
    }
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return json!({ "ok": false, "msg": "base_url 需以 http:// 或 https:// 开头" });
    }
    let path = baota_credentials_path();
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            return json!({ "ok": false, "msg": format!("创建凭据目录失败: {e}") });
        }
    }
    let data = json!({
        "base_url": base_url,
        "username": username,
        "password": password,
        "cookies": {},
        "win_token": "",
        "linux_token": "",
        "linux_time": "",
    });
    match std::fs::write(&path, serde_json::to_string(&data).unwrap_or_default()) {
        Ok(_) => json!({
            "ok": true,
            "msg": "凭据已保存，宝塔工具调用时将自动登录",
            "path": path.to_string_lossy(),
        }),
        Err(e) => json!({ "ok": false, "msg": format!("凭据保存失败: {e}") }),
    }
}

/// `baota.credentials.get`：凭据状态（不含密码），供 UI 展示是否已配置。
fn baota_credentials_get() -> Value {
    let path = baota_credentials_path();
    if !path.exists() {
        return json!({
            "configured": false,
            "base_url": "",
            "has_username": false,
            "path": path.to_string_lossy(),
        });
    }
    let v = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or(Value::Null);
    json!({
        "configured": true,
        "base_url": v.get("base_url").and_then(Value::as_str).unwrap_or(""),
        "has_username": !v.get("username").and_then(Value::as_str).unwrap_or("").is_empty(),
        "path": path.to_string_lossy(),
    })
}

/// `baota.credentials.clear`：删除凭据文件（登出）。
fn baota_credentials_clear() -> Value {
    let path = baota_credentials_path();
    match std::fs::remove_file(&path) {
        Ok(_) => json!({ "ok": true, "msg": "宝塔凭据已清除" }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            json!({ "ok": true, "msg": "宝塔凭据不存在（无需清除）" })
        }
        Err(e) => json!({ "ok": false, "msg": format!("清除凭据失败: {e}") }),
    }
}

/// Rust 端方法同步实现（Phase 2 状态方法 + Phase 3 空台账占位 + 融合：技能/记忆/渠道）
fn dispatch_rust(ctx: &GatewayContext, method: &str, params: &Value) -> Value {
    match method {
        "health" => health(ctx),
        "status" => system_status(ctx),
        "doctor.status" => doctor_status(ctx),
        "config.get" => config_get(ctx),
        // config.patch.safe / config.patch：侧车在线时 PY_FIRST 优先代理 Python；
        // 侧车离线时这里兜底做 Rust 本地持久化（纯 Rust 离线设置可用）。
        "config.patch.safe" => config_patch_local(ctx, params),
        "config.patch" => config_patch_local(ctx, params),
        "config.effective" => json!({ "fields": {} }),
        // ── 运行时模式（`mode.switch` 由异步分支处理；这里只读）──
        "mode.get" => mode_get(),
        "mode.status" => mode_get(),
        // 内置浏览器能力探测（同步，探测 Chromium 可执行文件）
        "browser.status" => crate::browser::gateway::status(),
        "sandbox.status" => sandbox_status(ctx),
        "sessions.list" => sessions_list(ctx, params),
        "sessions.messages.subscribe" => json!({
            "key": params.get("key").and_then(Value::as_str).unwrap_or_default(),
            "subscribed": true,
            "hydration_complete": true,
            "current_stream_seq": 0,
            "active_task_group_ids": [],
        }),
        // 会话快照 / 水合：Rust 最小实现（webui 订阅流程依赖，避免侧车代理阻塞 WS）
        "sessions.messages.snapshot" => json!({
            "key": params.get("key").and_then(Value::as_str).unwrap_or_default(),
            "events": [],
            "current_stream_seq": 0,
        }),
        "sessions.messages.hydrate" => json!({
            "key": params.get("key").and_then(Value::as_str).unwrap_or_default(),
            "hydration_complete": true,
        }),
        "chat.history" => chat_history(ctx, params),
        "tools.catalog" => tools_catalog(),
        // ── 宝塔面板凭据（baota_login 的落盘目标；btpanel-mcp 启动/调用时自动读取）──
        "baota.credentials.set" => baota_credentials_set(params),
        "baota.credentials.get" => baota_credentials_get(),
        "baota.credentials.clear" => baota_credentials_clear(),
        // ── 智能体（真实台账：agents.json 持久化，替换硬编码空数组）──
        "agents.list" => crate::agent::store::list(),
        "agents.get" => {
            let id = params.get("id").and_then(Value::as_str).unwrap_or_default();
            match crate::agent::store::get(id) {
                Some(v) => v,
                None => json!({ "error": "Agent not found", "code": "agent.not_found" }),
            }
        }
        // ── 专家团队（行业领军人经验蒸馏的专家档案库 + 用户画像抽取）──
        "experts.list" => experts_list(params),
        "agent.profile.analyze" => agent_profile_analyze(params),
        "agent.profile.auto" => agent_profile_auto(),
        "cron.list" => json!({ "items": [] }),
        // ── Onboarding 路由/集成配置（设置页模型路由保存）──
        "onboarding.router.configure" => onboarding_router_configure(ctx, params),
        "onboarding.ensemble.configure" => onboarding_ensemble_configure(ctx, params),
        "onboarding.provider.configure" => onboarding_provider_configure(ctx, params),
        // ── 用量（真实台账：每次 LLM 调用经 usage::record 落库，status/query 聚合返回）──
        "usage.status" => crate::usage::status(params),
        "usage.query" => crate::usage::query(params),
        "usage.daily" => crate::usage::daily(params.get("days").and_then(Value::as_u64).unwrap_or(28) as u32),
        // ── 知识图谱（kb.graph.*，全局图单例：实体-关系可视化）──
        "kb.graph.view" => kb_graph_view(),
        "kb.graph.related" => kb_graph_related(params),
        // ── 鲸鱼娘桌宠（pet.*，持久化 data_dir/pet_state.json）──
        "pet.status" => {
            let mut v = crate::workbench::pet::status();
            if agent_is_running() {
                v.mood = "working".to_string();
            }
            serde_json::to_value(v).unwrap_or(json!({}))
        }
        "pet.action" => pet_action(params),
        // ── 德州扑克游戏大厅（poker.*，全局大厅单例）──
        "poker.state" => poker_state(params),
        "poker.create" => poker_create(params),
        "poker.join" => poker_join(params),
        "poker.start" => poker_start(params),
        "poker.action" => poker_do_action(params),
        "poker.auto" => poker_auto(params),
        // ── 技能（SkillLoader 枚举本地技能目录，映射 WebUI Skill 契约）──
        "skills.list" => skills_list(ctx),
        "skills.get" => skills_get(ctx, params),
        "skills.reload" => skills_reload(ctx),
        "skills.search" => skills_search(ctx, params),
        "skills.uninstall" => skills_uninstall(ctx, params),
        "skills.create" => skills_create(ctx, params),
        // ── 项目工作区（新建任务引导打开/创建项目文件夹；会话绑定项目根目录）──
        "workspaces.list" => crate::gateway::workspaces::list(params),
        "workspaces.open" => crate::gateway::workspaces::open(params),
        "workspaces.create" => crate::gateway::workspaces::create(params),
        // 文件变更台账（前端「文件更改卡片」数据源：Agent 最近真实改动）
        "workspace.file_changes.list" => file_changes_list(params),
        "artifacts.list" => artifacts_list(params),
        // ── 产物台账（Epic A）：Rust 原生实现，不代理 Python 侧车 ──
        "artifacts.get" => artifacts_get(params),
        "artifacts.register" => artifacts_register(params),
        "artifacts.open_in_system" => artifacts_open_in_system(params),
        "artifacts.edit_inline" => artifacts_edit_inline(params),
        // ── 桌面自动化（Epic C）：Rust 原生实现（launch/action 走同步 COM，不代理）──
        "desktop.list_apps" => desktop_list_apps(),
        "desktop.launch" => desktop_launch(params),
        "desktop.action" => desktop_action(ctx, params),
        "desktop.snapshots" => desktop_snapshots(params),
        "desktop.stop" => desktop_stop(),
        // 文件变更回滚（前端「文件更改卡片」的拒绝操作：恢复 before 快照）
        "workspace.file_changes.revert" => file_changes_revert(params),
        "workspaces.update" => crate::gateway::workspaces::update(params),
        "workspaces.pin" => crate::gateway::workspaces::pin(params),
        "workspaces.remove" => crate::gateway::workspaces::remove(params),
        "workspaces.history.delete" => crate::gateway::workspaces::history_delete(params),
        // ── 记忆层（ExperienceLedger 直连，替代空台账占位）──
        "memory.search" => memory_search(params),
        "memory.save" => memory_save(params),
        "memory.list" => memory_list(params),
        // 7 层记忆其余四层（sensory/short_term/long_term/recall）真实 RPC
        "memory.sensory.search" => crate::memory::gateway::sensory_search(params),
        "memory.short_term.search" => crate::memory::gateway::short_term_search(params),
        "memory.long_term.search" => crate::memory::gateway::long_term_search(params),
        "memory.recall.search" => crate::memory::gateway::recall_search(params),
        // 7 层记忆端到端审计视图（真实计数 + RPC 可达性）
        "memory.layers.status" => memory_layers_status(params),
        // ── 渠道（最小台账：Rust 无渠道适配器，返回空台账避免 UI 报错）──
        "channels.status" => json!({ "channels": [], "boot_id": uuid::Uuid::new_v4().to_string() }),
        "channels.pairings" => json!({ "pairings": [] }),
        "channels.list" => json!({ "channels": [] }),
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
        "sandbox.capability.status" => sandbox_capability_status(),
        "sandbox.setup.status" => sandbox_setup_status(),
        // `sandbox.setup.ensure`：webui 沙箱「进入安全模式」调用。Rust 未内置沙箱
        // 运行时，直接返回 unavailable（与 setup.status 同源）；不代理 Python——
        // 侧车对该方法挂起 → WS 代理 3s 超时报「Python 侧车响应超时」。
        "sandbox.setup.ensure" => sandbox_setup_status(),
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
        "context.status" => crate::context::session::context_status(
            params
                .get("sessionKey")
                .and_then(Value::as_str)
                .unwrap_or("default"),
        ),
        "context.rollback" => crate::context::session::context_rollback(
            params
                .get("sessionKey")
                .and_then(Value::as_str)
                .unwrap_or("default"),
        ),
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
        "plans.capabilities" => plans_capabilities(),
        // ── 规划升级（B4：执行前清单 / 动态重规划 / 并行子任务）──
        "plans.checklist" => plan::plans_checklist(params),
        "plans.run.advance" => plan::plans_run_advance(params),
        "plans.replan" => plan::plans_replan(params),
        "plans.parallel.run" => plan::plans_parallel_run(params),
        // ── 长时任务（Bloom 长续航：心跳 / 检查点 / 暂停 / 续跑）──
        "long_running.list" => long_running_list(),
        "long_running.pause" => long_running_pause(params),
        "long_running.resume" => long_running_resume(params),
        "long_running.checkpoint" => long_running_checkpoint(params),
        // ── 沙箱交互式终端（read/list 同步读取内存态会话）──
        "sandbox.terminal.read" => crate::tools::terminal::terminal_read(params),
        "sandbox.terminal.list" => crate::tools::terminal::terminal_list(),
        // ── AI 智能融合路由（Bloom：编排形态自动决策，离线纯规则）──
        "router.decision" => router_decision(params),
        "router.selflearning.status" => router_selflearning_status(ctx),
        // ── 知识库（KB 2.0：BM25 检索 / 状态 / 灌入，把 kb/ 引擎暴露给 Web UI）──
        "kb.status" => crate::kb::gateway::status(),
        "kb.search" => crate::kb::gateway::search(params),
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
        "onboarding.catalog" => onboarding_catalog(),
        "onboarding.status" => onboarding_status(ctx),
        // ── 模型舰队（Fleet 多角色路由）──
        "fleet.list" => fleet_list(),
        "fleet.route" => fleet_route(params),
        // ── 自进化历史 ──
        "evolve.history" => evolve_history(),
        // ── 插件槽位 ──
        "plugins.list" => plugins_list(),
        // 其余写类方法或未分类方法统一 NOT_IMPLEMENTED
        _ => Value::Null,
    }
}

/// `onboarding.catalog`（Rust 离线回退）：返回内置 LLM / 搜索 / 图像 / 嵌入 provider 目录。
///
/// Python 侧车在线时由 `dispatch` 的 `is_python_method` 分支代理到侧车完整目录；
/// 侧车离线时由此函数提供最小可用目录，让设置页 / 渠道编辑器仍可渲染 provider 选择。
/// 字段对齐 webui `OnboardingCatalog` / `ProviderSpec`（`useSetupCatalog.ts`）。
fn onboarding_catalog() -> Value {
    // 通用 OpenAI 兼容 endpoint 字段
    let openai_compatible_fields = json!([
        { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
        { "name": "base_url", "label": "Base URL", "type": "text", "required": false, "placeholder": "https://api.example.com/v1" },
        { "name": "model", "label": "Model", "type": "text", "required": true, "placeholder": "model-name" },
    ]);

    let providers = json!([
        {
            "providerId": "deepseek",
            "label": "DeepSeek",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "DEEPSEEK_API_KEY",
            "defaultBaseUrl": "https://api.deepseek.com/v1",
            "defaultDirectModel": "deepseek-chat",
            "suggestedModels": ["deepseek-chat", "deepseek-reasoner"],
            "whatYouNeed": ["DeepSeek API Key"],
            "fields": [
                { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
                { "name": "model", "label": "Model", "type": "text", "required": false, "default": "deepseek-chat" },
            ],
        },
        {
            "providerId": "openai",
            "label": "OpenAI",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "OPENAI_API_KEY",
            "defaultBaseUrl": "https://api.openai.com/v1",
            "defaultDirectModel": "gpt-4o",
            "suggestedModels": ["gpt-4o", "gpt-4o-mini", "o1", "o3-mini"],
            "whatYouNeed": ["OpenAI API Key"],
            "fields": &openai_compatible_fields,
        },
        {
            "providerId": "anthropic",
            "label": "Anthropic",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "ANTHROPIC_API_KEY",
            "defaultBaseUrl": "https://api.anthropic.com/v1",
            "defaultDirectModel": "claude-sonnet-4-20250514",
            "suggestedModels": ["claude-sonnet-4-20250514", "claude-opus-4-20250514", "claude-haiku-4-20250514"],
            "whatYouNeed": ["Anthropic API Key"],
            "fields": [
                { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
                { "name": "model", "label": "Model", "type": "text", "required": false, "default": "claude-sonnet-4-20250514" },
            ],
        },
        {
            "providerId": "qwen",
            "label": "Qwen (Alibaba Cloud)",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "DASHSCOPE_API_KEY",
            "defaultBaseUrl": "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "defaultDirectModel": "qwen-max",
            "suggestedModels": ["qwen-max", "qwen-plus", "qwen-turbo", "qwen3-235b-a22b"],
            "whatYouNeed": ["DashScope API Key"],
            "fields": &openai_compatible_fields,
        },
        {
            "providerId": "zhipu",
            "label": "Zhipu (Z.AI)",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "ZHIPU_API_KEY",
            "defaultBaseUrl": "https://open.bigmodel.cn/api/paas/v4",
            "defaultDirectModel": "glm-4-plus",
            "suggestedModels": ["glm-4-plus", "glm-4-air", "glm-4-flash"],
            "whatYouNeed": ["Zhipu API Key"],
            "fields": &openai_compatible_fields,
        },
        {
            "providerId": "moonshot",
            "label": "Moonshot AI (Kimi)",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "MOONSHOT_API_KEY",
            "defaultBaseUrl": "https://api.moonshot.cn/v1",
            "defaultDirectModel": "moonshot-v1-128k",
            "suggestedModels": ["moonshot-v1-8k", "moonshot-v1-32k", "moonshot-v1-128k"],
            "whatYouNeed": ["Moonshot API Key"],
            "fields": &openai_compatible_fields,
        },
        {
            "providerId": "siliconflow",
            "label": "SiliconFlow",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "SILICONFLOW_API_KEY",
            "defaultBaseUrl": "https://api.siliconflow.cn/v1",
            "defaultDirectModel": "deepseek-ai/DeepSeek-V3",
            "suggestedModels": ["deepseek-ai/DeepSeek-V3", "deepseek-ai/DeepSeek-R1", "Qwen/Qwen2.5-72B-Instruct"],
            "whatYouNeed": ["SiliconFlow API Key"],
            "fields": &openai_compatible_fields,
        },
        {
            "providerId": "openrouter",
            "label": "OpenRouter",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "OPENROUTER_API_KEY",
            "defaultBaseUrl": "https://openrouter.ai/api/v1",
            "defaultDirectModel": "deepseek/deepseek-chat",
            "whatYouNeed": ["OpenRouter API Key"],
            "fields": &openai_compatible_fields,
        },
        {
            "providerId": "custom",
            "label": "Custom OpenAI-compatible endpoint",
            "runtimeSupported": true,
            "routerSupported": false,
            "acceptsApiKey": true,
            "requiresApiKey": false,
            "deployment": "custom",
            "whatYouNeed": ["OpenAI-compatible API endpoint", "API Key (if required)"],
            "fields": &openai_compatible_fields,
        },
    ]);

    let search_providers = json!([
        {
            "providerId": "tavily",
            "label": "Tavily Search",
            "runtimeSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "TAVILY_API_KEY",
            "fields": [
                { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
            ],
        },
        {
            "providerId": "serper",
            "label": "Serper.dev",
            "runtimeSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "SERPER_API_KEY",
            "fields": [
                { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
            ],
        },
    ]);

    let image_providers = json!([
        {
            "providerId": "siliconflow",
            "label": "SiliconFlow Image Generation",
            "runtimeSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "SILICONFLOW_API_KEY",
            "fields": [
                { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
            ],
        },
    ]);

    let embedding_providers = json!([
        {
            "providerId": "siliconflow",
            "label": "SiliconFlow Embeddings",
            "runtimeSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "SILICONFLOW_API_KEY",
            "fields": [
                { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
            ],
        },
    ]);

    let channels = json!([
        {
            "type": "slack",
            "label": "Slack",
            "description": "Slack 频道（Bot Token + App Token）",
            "transport": "websocket",
            "requiresPublicUrl": false,
            "restartRequired": true,
            "canProbe": true,
            "fields": [
                { "name": "name", "label": "Channel name", "type": "text", "required": true },
                { "name": "bot_token", "label": "Bot Token (xoxb-)", "type": "password", "required": true, "secret": true },
                { "name": "app_token", "label": "App Token (xapp-)", "type": "password", "required": true, "secret": true },
                { "name": "enabled", "label": "Enabled", "type": "bool", "required": false, "default": true, "advanced": true },
            ],
        },
        {
            "type": "discord",
            "label": "Discord",
            "description": "Discord 机器人（Gateway WebSocket）",
            "transport": "websocket",
            "requiresPublicUrl": false,
            "restartRequired": true,
            "canProbe": true,
            "fields": [
                { "name": "name", "label": "Channel name", "type": "text", "required": true },
                { "name": "bot_token", "label": "Bot Token", "type": "password", "required": true, "secret": true },
                { "name": "enabled", "label": "Enabled", "type": "bool", "required": false, "default": true, "advanced": true },
            ],
        },
        {
            "type": "telegram",
            "label": "Telegram",
            "description": "Telegram 机器人（Long Polling）",
            "transport": "polling",
            "requiresPublicUrl": false,
            "restartRequired": true,
            "canProbe": true,
            "fields": [
                { "name": "name", "label": "Channel name", "type": "text", "required": true },
                { "name": "bot_token", "label": "Bot Token", "type": "password", "required": true, "secret": true },
                { "name": "enabled", "label": "Enabled", "type": "bool", "required": false, "default": true, "advanced": true },
            ],
        },
        {
            "type": "feishu",
            "label": "飞书 / Lark",
            "description": "飞书自建应用（事件回调）",
            "transport": "webhook",
            "requiresPublicUrl": true,
            "restartRequired": true,
            "canProbe": true,
            "fields": [
                { "name": "name", "label": "Channel name", "type": "text", "required": true },
                { "name": "app_id", "label": "App ID", "type": "text", "required": true },
                { "name": "app_secret", "label": "App Secret", "type": "password", "required": true, "secret": true },
                { "name": "verification_token", "label": "Verification Token", "type": "password", "required": false, "secret": true },
                { "name": "enabled", "label": "Enabled", "type": "bool", "required": false, "default": true, "advanced": true },
            ],
        },
        {
            "type": "wecom",
            "label": "企业微信",
            "description": "企业微信自建应用（回调）",
            "transport": "webhook",
            "requiresPublicUrl": true,
            "restartRequired": true,
            "canProbe": true,
            "fields": [
                { "name": "name", "label": "Channel name", "type": "text", "required": true },
                { "name": "corp_id", "label": "Corp ID", "type": "text", "required": true },
                { "name": "agent_id", "label": "Agent ID", "type": "text", "required": true },
                { "name": "secret", "label": "Secret", "type": "password", "required": true, "secret": true },
                { "name": "token", "label": "Callback Token", "type": "password", "required": false, "secret": true },
                { "name": "encoding_aes_key", "label": "Encoding AES Key", "type": "password", "required": false, "secret": true },
                { "name": "enabled", "label": "Enabled", "type": "bool", "required": false, "default": true, "advanced": true },
            ],
        },
        {
            "type": "dingtalk",
            "label": "钉钉",
            "description": "钉钉机器人（Stream 模式）",
            "transport": "websocket",
            "requiresPublicUrl": false,
            "restartRequired": true,
            "canProbe": true,
            "fields": [
                { "name": "name", "label": "Channel name", "type": "text", "required": true },
                { "name": "client_id", "label": "Client ID", "type": "text", "required": true },
                { "name": "client_secret", "label": "Client Secret", "type": "password", "required": true, "secret": true },
                { "name": "enabled", "label": "Enabled", "type": "bool", "required": false, "default": true, "advanced": true },
            ],
        },
    ]);

    let audio_providers = json!([
        {
            "providerId": "elevenlabs",
            "label": "ElevenLabs Audio",
            "runtimeSupported": true,
            "requiresApiKey": true,
            "envKey": "ELEVENLABS_API_KEY",
            "defaultBaseUrl": "https://api.elevenlabs.io",
            "defaultTtsModel": "eleven_multilingual_v2",
            "defaultTtsVoice": "21m00Tcm4TlvDq8ikWAM",
            "deployment": "cloud",
            "blocking": false,
            "canProbe": true,
            "whatYouNeed": ["ElevenLabs API Key"],
            "fields": [
                { "name": "enabled", "label": "Enabled", "type": "bool", "required": false, "default": false },
                { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
            ],
        },
    ]);

    json!({
        "providers": providers,
        "channels": channels,
        "searchProviders": search_providers,
        "routerProfiles": { "profiles": [], "defaultTier": "auto" },
        "memoryEmbeddingProviders": embedding_providers,
        "imageGenerationProviders": image_providers,
        "audioProviders": audio_providers,
    })
}

/// `onboarding.status`（Rust 离线回退）：返回最小引导状态。
///
/// 侧车在线时由 Python 提供完整 sectionDetails / credential 状态；
/// 离线时返回基于 config.yaml 的确定性状态，让设置页可渲染。
fn onboarding_status(ctx: &GatewayContext) -> Value {
    // 重读磁盘而非内存快照：保存（provider.configure / config.patch）只落盘、不回写
    // GatewayContext.config，若此处读内存快照，刚保存的 api_key 不会生效，前端
    // onboarding.status 仍返回 needsOnboarding → 引导 banner 永不消失，用户误以为保存失败。
    // 每次查询重读 config.yaml（首次启动已创建，仅读不写），开销可忽略。
    let live = Config::load().unwrap_or_else(|_| ctx.config.clone());
    let llm_configured = !live.model.api_key.is_empty();
    let llm_source = if llm_configured { "config" } else { "none" };
    let data_dir = Config::data_dir().to_string_lossy().to_string();
    json!({
        "needsOnboarding": !llm_configured,
        "hasConfig": llm_configured,
        "llmConfigured": llm_configured,
        "llmSource": llm_source,
        "channelCount": 0,
        "channelsConfigured": false,
        "searchConfigured": false,
        "imageGenerationConfigured": false,
        "configPath": data_dir,
        "sections": {
            "llm": if llm_configured { "complete" } else { "incomplete" },
            "search": "incomplete",
            "imageGeneration": "incomplete",
            "memory": "incomplete",
        },
        "sectionDetails": {},
        "warnings": if llm_configured { Value::Array(vec![]) } else { json!(["LLM API Key 未配置（离线降级可用）"]) },
    })
}

/// 兼容旧同步调用（如单测直接调用）：同步函数直接调用 Rust 部分
pub fn dispatch_sync(
    ctx: &GatewayContext,
    method: &str,
    params: &Value,
) -> Result<Value, RpcError> {
    // 画像导入 8 RPC（闭环 INFO/START/STATUS/CANCEL/RETRY/APPLY/UNDO/DISCARD）
    // 同步路径也可调用（测试/CLI），用 match 放到最顶部统一转换 Err(code,msg)。
    match method {
        // 产物列表：同步 Rust 实现（读会话产物目录），同步路径也应可达
        "artifacts.list" => return Ok(artifacts_list(params)),
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
        _ => {}
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

fn health(ctx: &GatewayContext) -> Value {
    json!({
        "status": "ok",
        "engine": "rust",
        "protocol": PROTOCOL_VERSION,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_ms": ctx.started_at.elapsed().as_millis(),
        "data_dir": Config::data_dir().to_string_lossy(),
        "api_key_configured": !ctx.config.model.api_key.is_empty(),
    })
}

fn system_status(ctx: &GatewayContext) -> Value {
    json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_ms": ctx.started_at.elapsed().as_millis(),
        "provider": &ctx.config.model.provider,
        "model": &ctx.config.model.model_name,
        "engine": "rust",
    })
}

fn doctor_status(ctx: &GatewayContext) -> Value {
    // 回环可绑定探测：确认网关端口域未被占用（仅作诊断，不真正绑定）
    let loopback_bindable = std::net::TcpListener::bind(("127.0.0.1", 0)).is_ok();
    let checks = json!([
        { "name": "config", "ok": true, "detail": "config.yaml 可读" },
        { "name": "loopback", "ok": loopback_bindable, "detail": "回环端口可绑定" },
        {
            "name": "model",
            "ok": !ctx.config.model.api_key.is_empty(),
            "detail": if ctx.config.model.api_key.is_empty() {
                "未配置 API Key（离线降级可用）"
            } else {
                &ctx.config.model.model_name
            },
        },
    ]);
    let degraded = !ctx.config.model.api_key.is_empty()
        && checks.as_array().is_some_and(|c| {
            c.iter()
                .any(|c| c.get("ok").and_then(Value::as_bool) != Some(true))
        });
    json!({
        "status": if degraded { "degraded" } else { "ok" },
        "checks": checks,
        "data_dir": Config::data_dir().to_string_lossy(),
    })
}

/// `workspace.file_changes.list`：返回 Agent 最近对工作区文件的真实改动台账。
///
/// 供前端「文件更改卡片」读取（展示 / 折叠 / diff / 提问交互的前置数据层）。
/// 数据源为进程级 `FileChangeLedger`（内置文件工具写盘时记录），最新在前。
fn file_changes_list(params: &Value) -> Value {
    let limit: usize = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .min(200) as usize;
    let items = crate::tools::file_change::list_file_changes(limit);
    json!({
        "items": items.iter().map(|c| json!({
            "path": c.path,
            "kind": c.kind,
            "before": c.before,
            "after": c.after,
            "tool": c.tool,
            "ts": c.ts,
        })).collect::<Vec<_>>(),
    })
}

/// `artifacts.list`：返回指定会话中 Agent 生成的产物文件列表（游标分页）。
///
/// 参数：`sessionKey`/`session_key`/`session`（三选一）、`cursor`（上一页末条 ts）、`limit`（默认 50）。
/// 返回 `{artifacts, has_more, next_cursor}`，兼容既有前端调用（无 cursor/limit 时退化为全量首页）。
fn artifacts_list(params: &Value) -> Value {
    let session = params
        .get("sessionKey")
        .or_else(|| params.get("session_key"))
        .or_else(|| params.get("session"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let cursor = params.get("cursor").and_then(Value::as_u64);
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(50);
    let (items, has_more, next_cursor) =
        crate::tools::file_change::list_artifacts(session, cursor, limit);
    json!({
        "artifacts": items.iter().map(|a| json!({
            "id": a.id,
            "name": a.name,
            "path": a.path,
            "kind": a.kind,
            "size": a.size,
            "ts": a.ts,
            "mime": a.mime,
            "session_key": a.session_key,
            "sha256": a.sha256,
        })).collect::<Vec<_>>(),
        "has_more": has_more,
        "next_cursor": next_cursor,
    })
}

// ───────────────────────────────────────────────────────────────────────────
// Epic A：产物台账 RPC（Value 信封 `{code, message, data}`）
// 错误码：ARTIFACT_NOT_FOUND=1404 / ARTIFACT_OPEN_FAILED=1502 / SESSION_UNKNOWN=1401
// ───────────────────────────────────────────────────────────────────────────

/// 成功信封（code=0）。
fn rpc_ok(data: Value) -> Value {
    json!({ "code": 0, "message": "", "data": data })
}

/// 失败信封（携带业务错误码）。
fn rpc_err(code: u32, message: &str) -> Value {
    json!({ "code": code, "message": message, "data": Value::Null })
}

/// `artifacts.get`：按 id 返回完整产物条目（含 mime / session_key）。
fn artifacts_get(params: &Value) -> Value {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return rpc_err(1401, "缺少 artifact id");
    }
    match crate::tools::file_change::get_artifact(id) {
        Some(a) => rpc_ok(json!({
            "id": a.id,
            "name": a.name,
            "path": a.path,
            "kind": a.kind,
            "size": a.size,
            "ts": a.ts,
            "mime": a.mime,
            "session_key": a.session_key,
            "sha256": a.sha256,
        })),
        None => rpc_err(1404, "产物不存在"),
    }
}

/// `artifacts.register`：显式重新登记产物（R1 回退通道：调用方传入 session_key + path）。
fn artifacts_register(params: &Value) -> Value {
    let session_key = params
        .get("session_key")
        .and_then(Value::as_str)
        .unwrap_or("");
    let path = params.get("path").and_then(Value::as_str).unwrap_or("");
    let kind = params
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("registered");
    if session_key.is_empty() {
        return rpc_err(1401, "缺少 session_key");
    }
    if path.is_empty() || !std::path::Path::new(path).is_file() {
        return rpc_err(1404, "路径不存在或不是文件");
    }
    crate::tools::file_change::record_artifact_with_session(session_key, path, kind);
    let id = format!("artifact:{}:{}", session_key, path);
    match crate::tools::file_change::get_artifact(&id) {
        Some(a) => rpc_ok(json!({
            "id": a.id,
            "path": a.path,
            "kind": a.kind,
            "mime": a.mime,
            "session_key": a.session_key,
            "sha256": a.sha256,
        })),
        None => rpc_err(1404, "登记失败"),
    }
}

/// 用系统默认程序打开文件（无 tauri 依赖，纯 std::process）。
#[cfg(windows)]
fn open_in_system(path: &str) -> bool {
    std::process::Command::new("cmd")
        .args(["/c", "start", "", path])
        .spawn()
        .is_ok()
}

#[cfg(not(windows))]
fn open_in_system(path: &str) -> bool {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(cmd).arg(path).spawn().is_ok()
}

/// `artifacts.open_in_system`：调用 OS 默认程序打开已登记产物。
/// 无 id 时退化为直接按 `path` 打开（前端降级路径）。
fn artifacts_open_in_system(params: &Value) -> Value {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    let path = params.get("path").and_then(Value::as_str).unwrap_or("");
    let target = if !id.is_empty() {
        match crate::tools::file_change::get_artifact(id) {
            Some(a) => a.path,
            None => return rpc_err(1404, "产物不存在"),
        }
    } else if !path.is_empty() {
        if !std::path::Path::new(path).is_file() {
            return rpc_err(1404, "路径不存在或不是文件");
        }
        path.to_string()
    } else {
        return rpc_err(1401, "缺少 artifact id 或 path");
    };
    if open_in_system(&target) {
        rpc_ok(json!({ "ok": true, "launched": true, "path": target }))
    } else {
        rpc_err(1502, "无法用系统默认程序打开该文件")
    }
}

/// `artifacts.edit_inline`：原地覆写已登记产物内容（带产物归属登记 + 竞态防护）。
///
/// 竞态防护（Epic A 前端契约）：前端在打开内联编辑器时记录文件 `sha256`，保存时通过
/// `expectedSha256` 回传。写入前比对磁盘当前内容哈希——若不一致，说明文件在用户编辑
/// 期间已被其它 turn 重新生成，直接写会静默覆盖模型新产物。此时返回可识别冲突错误
/// （code 1409，消息含 `sha256`/`conflict` 关键字），由前端提示「请重新载入」。
fn artifacts_edit_inline(params: &Value) -> Value {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    let content = params
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("");
    // 前端回传其最后所见内容的 sha256（camelCase 或 snake_case 均可）。
    let expected_sha = params
        .get("expectedSha256")
        .or_else(|| params.get("expected_sha256"))
        .and_then(Value::as_str)
        .map(str::to_string);
    if id.is_empty() {
        return rpc_err(1401, "缺少 artifact id");
    }
    let art = match crate::tools::file_change::get_artifact(id) {
        Some(a) => a,
        None => return rpc_err(1404, "产物不存在"),
    };
    // 竞态校验：文件在编辑期间被其它 turn 重新生成 → 拒绝静默覆盖，返回可识别冲突错误。
    if let Some(exp) = expected_sha.as_deref() {
        let current = crate::tools::file_change::sha256_of(&art.path);
        if current.as_deref() != Some(exp) {
            return rpc_err(
                1409,
                &format!(
                    "sha256 冲突（conflict）：文件 '{}' 在您编辑期间已被更新，请重新载入后再保存",
                    art.name
                ),
            );
        }
    }
    // 安全：仅允许覆写已登记产物（路径明确归属），不接收任意外部路径。
    if let Err(e) = std::fs::write(&art.path, content) {
        return rpc_err(1502, &format!("写入失败: {e}"));
    }
    let sess = art.session_key.clone();
    crate::tools::file_change::record_artifact_with_session(&sess, &art.path, "modified");
    match crate::tools::file_change::get_artifact(id) {
        Some(a) => rpc_ok(json!({
            "id": a.id,
            "path": a.path,
            "kind": a.kind,
            "size": a.size,
            "mime": a.mime,
            "sha256": a.sha256,
        })),
        None => rpc_err(1404, "登记失败"),
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Epic C：桌面自动化 RPC（Value 信封 `{code, message, data}`）
// 错误码：DESKTOP_APP_UNKNOWN=2401 / DESKTOP_ACTION_DENIED=2403 / DESKTOP_COM_ERROR=2505
// ───────────────────────────────────────────────────────────────────────────

/// `desktop.list_apps`：已注册 + 运行中应用列表。
fn desktop_list_apps() -> Value {
    let apps = AppRegistry::global().list_apps();
    rpc_ok(json!({ "apps": apps }))
}

/// `desktop.launch`：按 app_kind 启动应用，返回句柄。
fn desktop_launch(params: &Value) -> Value {
    let app_kind = params
        .get("app_kind")
        .and_then(Value::as_str)
        .unwrap_or("");
    let kind = match AppKind::from_str(app_kind) {
        Some(k) => k,
        None => return rpc_err(2401, &format!("未知桌面应用: {app_kind}")),
    };
    match AppRegistry::global().launch(kind, params) {
        Ok(h) => rpc_ok(json!({
            "handle": h.id,
            "kind": h.kind.as_str(),
            "launched_at": h.launched_at,
        })),
        Err(e) => match e {
            DesktopError::AppUnknown(_) => rpc_err(2401, &e.to_string()),
            DesktopError::ComError(_) => rpc_err(2505, &e.to_string()),
            _ => rpc_err(2505, &e.to_string()),
        },
    }
}

/// `desktop.action`：对句柄执行 UI 动作（敏感动作先查 Policy 二次确认闸门）。
fn desktop_action(_ctx: &GatewayContext, params: &Value) -> Value {
    let handle = params.get("handle").and_then(Value::as_str).unwrap_or("");
    if handle.is_empty() {
        return rpc_err(1401, "缺少 handle");
    }
    let action: UiAction = match serde_json::from_value(
        params.get("action").cloned().unwrap_or(Value::Null),
    ) {
        Ok(a) => a,
        Err(e) => return rpc_err(2403, &format!("动作参数解析失败: {e}")),
    };
    // 安全闸门：敏感动作需二次确认（前端携带 approved=true 才放行）
    if Policy::require_confirmation(&action)
        && !params
            .get("approved")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return rpc_err(2403, "该动作需要二次确认（前端请携带 approved=true）");
    }
    // 注入扫描：命中潜在注入特征直接拒绝（后端兜底）
    if let Some(reason) = Policy::scan_injection(&action) {
        return rpc_err(2403, &reason);
    }
    let session = params
        .get("session_key")
        .and_then(Value::as_str)
        .unwrap_or("");
    match AppRegistry::global().run_action(handle, &action, session) {
        Ok(receipt) => {
            let receipt_val = serde_json::to_value(&receipt).unwrap_or(Value::Null);
            // 广播 desktop.action_done（前端审计回执）
            if let Some(bus) = crate::events::global_bus() {
                bus.publish(Envelope::desktop_action_done(
                    session,
                    &receipt.handle,
                    &receipt.action,
                    &receipt_val,
                    receipt.ts,
                ));
            }
            rpc_ok(receipt_val)
        }
        Err(e) => {
            let code = match e {
                DesktopError::AppUnknown(_) => 2401,
                DesktopError::ActionDenied(_) => 2403,
                _ => 2505,
            };
            rpc_err(code, &e.to_string())
        }
    }
}

/// `desktop.snapshots`：列出某会话的审计快照。
fn desktop_snapshots(params: &Value) -> Value {
    let session = params
        .get("session_key")
        .and_then(Value::as_str)
        .unwrap_or("");
    let snaps = Recorder::list_snapshots(session);
    rpc_ok(json!({ "snapshots": snaps }))
}

/// `desktop.stop`：急停所有运行中应用。
fn desktop_stop() -> Value {
    let _ = AppRegistry::global().stop_all();
    rpc_ok(json!({ "ok": true }))
}

/// `workspace.file_changes.revert`：回滚一条文件变更（前端「拒绝」操作）。
///
/// 依据台账记录的 `before` 内容恢复：create→删除文件、modify/delete→写回
/// `before`。工作区根与内置文件工具一致（进程 `current_dir`），路径经
/// `safe_resolve` 校验不逃逸工作区。诚实边界：恢复的是「记录时的快照」，若
/// 同一文件之后又被其他操作改动，此回滚会以记录的 before 覆盖之。
fn file_changes_revert(params: &Value) -> Value {
    let path = match params.get("path").and_then(Value::as_str) {
        Some(p) if !p.trim().is_empty() => p,
        _ => return json!({ "error": "missing 'path'", "code": "invalid_params" }),
    };
    let kind = params
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("modify");
    let before = params.get("before").and_then(Value::as_str).unwrap_or("");

    let ws = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let abs = match crate::tools::fs_path::safe_resolve(&ws, path) {
        Ok(p) => p,
        Err(e) => {
            return json!({ "ok": false, "reverted": false, "error": e.to_string(), "code": "path_escape" })
        }
    };

    let outcome = match kind {
        // 新建被拒绝 → 删除该文件（若仍存在；不存在视为已回滚）。
        "create" => match std::fs::remove_file(&abs) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        },
        // 删除 / 修改被拒绝 → 恢复 before 内容。
        _ => {
            if let Some(parent) = abs.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&abs, before)
        }
    };

    match outcome {
        Ok(()) => json!({ "ok": true, "reverted": true, "path": path, "kind": kind }),
        Err(e) => {
            json!({ "ok": false, "reverted": false, "error": e.to_string(), "code": "revert_failed" })
        }
    }
}

/// `experts.list`：列出行业领军人经验蒸馏的专家档案库。
///
/// 供前端「专家团队」面板展示：每条档案含 lineage（经验来源）、mindset（思维
/// 模型）、expertise（专长）、skills（专属技能）、playbook（分步路径）。
fn experts_list(_params: &Value) -> Value {
    let experts: Vec<Value> = crate::agent::expertise::experts_list()
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "label": e.label,
                "lineage": e.lineage,
                "mindset": e.mindset,
                "expertise": e.expertise,
                "skills": e.skills,
                "playbook": e.playbook,
            })
        })
        .collect();
    json!({ "experts": experts })
}

/// `agent.profile.analyze`：从对话历史确定性抽取用户画像（P-11 端到端暴露）。
///
/// params: `{ messages: [{ role: "user"|"assistant", content: string }] }`
/// → `{ role, tone, expertise }`（抽取式，不烧 token、不编造）。
fn agent_profile_analyze(params: &Value) -> Value {
    let Some(messages) = params.get("messages").and_then(Value::as_array) else {
        return json!({ "error": "missing 'messages'", "code": "invalid_params" });
    };
    let history: Vec<crate::domain::Message> = messages
        .iter()
        .filter_map(|m| {
            let role = match m.get("role").and_then(Value::as_str).unwrap_or("") {
                "user" => crate::domain::Role::User,
                "assistant" => crate::domain::Role::Assistant,
                "system" => crate::domain::Role::System,
                _ => return None,
            };
            let content = m.get("content").and_then(Value::as_str).unwrap_or("");
            Some(crate::domain::Message {
                role,
                content: content.to_string(),
                meta: None,
            })
        })
        .collect();
    let persona = crate::agent::profile::auto_profile(&history);
    json!({
        "role": persona.role,
        "tone": persona.tone,
        "expertise": persona.expertise,
    })
}

/// `agent.profile.auto`：自动从记忆与会话历史中抽取用户画像，无需手动粘贴。
/// 聚合最近的上下文台账条目 + 经验台账，生成角色/语气/专长画像。
fn agent_profile_auto() -> Value {
    let mut all_entries = Vec::new();
    // 从上下文台账取最近会话的条目
    for cs in crate::context::session::context_sessions()
        .into_iter()
        .take(20)
    {
        let entries = crate::context::session::context_entries(&cs.session);
        all_entries.extend(entries.into_iter().take(20));
    }
    if all_entries.is_empty() {
        return json!({
            "role": "通用用户",
            "tone": "自然",
            "expertise": "暂无足够数据生成画像，多对话后自动更新",
            "auto": true,
            "source_count": 0,
        });
    }
    let history: Vec<crate::domain::Message> = all_entries
        .iter()
        .filter_map(|e| {
            let role = match e.role.as_str() {
                "user" => crate::domain::Role::User,
                "assistant" => crate::domain::Role::Assistant,
                _ => crate::domain::Role::User,
            };
            Some(crate::domain::Message {
                role,
                content: e.text.clone(),
                meta: None,
            })
        })
        .collect();
    let persona = crate::agent::profile::auto_profile(&history);
    json!({
        "role": persona.role,
        "tone": persona.tone,
        "expertise": persona.expertise,
        "auto": true,
        "source_count": all_entries.len(),
    })
}

/// `memory.layers.status`：7 层记忆端到端审计视图（真实计数 + RPC 可达性）。
///
/// 委托给 [`crate::memory::gateway::layers_status`]：sensory / short_term /
/// long_term / recall 返回真实计数，experience / behavior / cross_project 标注可达性。
fn memory_layers_status(params: &Value) -> Value {
    crate::memory::gateway::layers_status(params)
}

fn config_get(ctx: &GatewayContext) -> Value {
    // 每次读取都从磁盘重载：config.patch.safe 落盘后立即可见（内存 Config 是
    // 启动快照 clone，不可变），保证「写→读」闭环一致，修复设置开关回弹。
    let cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    // 前端设置页读 `llm`/`llm_profiles`/`squilla_router.tiers`/`llm_ensemble`
    // （Model Service 候选/路由模板数据源）。Rust 离线兜底也用同一份
    // model 配置物化这些字段，避免侧车离线时「AI 路由候选」整链为空。
    let llm_profile = json!({
        "model": cfg.model.model_name,
        "base_url": cfg.model.base_url,
    });
    let llm_profiles = {
        let mut m = serde_json::Map::new();
        m.insert(cfg.model.provider.clone(), llm_profile.clone());
        m
    };
    json!({
        "model": {
            "provider": cfg.model.provider,
            "model_name": cfg.model.model_name,
            "base_url": cfg.model.base_url,
            "api_key_configured": !cfg.model.api_key.is_empty(),
        },
        // 与 Python 侧车 config.get 契约对齐：`llm` 为当前主模型、
        // `llm_profiles` 为已持久化的模型档案（离线时仅当前模型）。
        "llm": {
            "provider": cfg.model.provider,
            "model": cfg.model.model_name,
            "base_url": cfg.model.base_url,
            "api_key": cfg.model.api_key,
            "api_key_configured": !cfg.model.api_key.is_empty(),
        },
        "llm_profiles": llm_profiles,
        "llm_request_timeout_seconds": 60,
        "mcp": {
            "servers": cfg.mcp.servers.iter().map(|s| json!({
                "name": s.name,
                "command": s.command,
                "args": s.args,
            })).collect::<Vec<_>>(),
        },
        "memory": {
            "sensory_path": cfg.memory.sensory_path,
            "short_term_path": cfg.memory.short_term_path,
            "auto_capture_enabled": cfg.memory.auto_capture_enabled,
            "dream": {
                "enabled": cfg.dream.enabled,
                "auto_schedule": cfg.dream.auto_schedule,
            },
        },
        "squilla_router": {
            "self_learning": {
                "enabled": cfg.squilla_router.enabled,
            },
            // 离线兜底提供空路由模板（tiers 空对象 → 前端渲染空态提示，不报错）
            "enabled": cfg.squilla_router.enabled,
            "preset_binding": "follow_primary",
            "default_tier": "",
            "visual_mode": "",
            "cross_provider_tiers": false,
            "tiers": {},
        },
        "llm_ensemble": {
            "enabled": false,
            "model_options": [],
            "candidates": [],
        },
        "skills": {
            "skills_dir": cfg.skills.skills_dir,
        },
        "log": {
            "level": cfg.log.level,
            "file": cfg.log.file,
        },
        "naming": {
            "enabled": cfg.naming.enabled,
        },
        "privacy": {
            "disable_network_observability": cfg.privacy.disable_network_observability,
            "network_observability_disabled_effective": cfg.privacy.disable_network_observability,
        },
        // 护栏能力开关（与 config_patch_local 写入的键对称，保证离线模式读回一致，
        // 避免 guard 开关保存后「回弹」）。
        "guard": {
            "syntax": cfg.guard.syntax,
            "semantic": cfg.guard.semantic,
            "fact": cfg.guard.fact,
            "contract": cfg.guard.contract,
        },
        // Bloom 并行编排开关（与 config_patch_local 写入对称）。
        "bloom": {
            "parallel_enabled": cfg.bloom.parallel_enabled,
        },
    })
}

/// `onboarding.router.configure`：保存模型路由配置（多模型融合/智能路由/固定模型）。
/// 接收前端 routerForm.payload()，将关键字段落盘到 config.yaml 的 router 段。
fn onboarding_router_configure(ctx: &GatewayContext, params: &Value) -> Value {
    // 重读磁盘：ctx.config 是启动快照，多次保存若只 clone 旧快照会丢失其它进程
    // 写入的最新值；与 config_patch_local / config_get 一致，保存前从磁盘重载。
    let mut cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    if let Some(mode) = params.get("mode").and_then(Value::as_str) {
        // disabled → direct(固定模型), recommended → router(智能路由), custom → ensemble(多模型融合)
        cfg.model_routing.mode = match mode {
            "disabled" => "direct".to_string(),
            "recommended" => "router".to_string(),
            other => other.to_string(),
        };
    }
    if let Some(fusion) = params.get("fusionModel").and_then(Value::as_str) {
        cfg.model_routing.fusion_model = fusion.to_string();
    }
    if let Some(candidates) = params.get("candidateModels").and_then(Value::as_array) {
        cfg.model_routing.candidate_models = candidates
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }
    if let Some(fixed) = params.get("fixedModel").and_then(Value::as_str) {
        cfg.model_routing.fixed_model = fixed.to_string();
    }
    if let Some(v) = params.get("crossProviderTiers").and_then(Value::as_bool) {
        cfg.model_routing.cross_provider_tiers = v;
    }
    // 路由模式（direct/router/ensemble/bloom）在运行时由启动快照构建的 ModelRouter
    // 决定，落盘后需重启网关才生效；如实返回 restartRequired 让前端提示重启，
    // 避免「已保存但不生效」造成的误解（此前误报 restart_required: false）。
    match cfg.save() {
        Ok(()) => json!({ "ok": true, "saved": true, "restartRequired": true }),
        Err(e) => json!({ "ok": false, "error": format!("保存路由配置失败: {e}") }),
    }
}

/// `onboarding.ensemble.configure`：保存 Bloom 集成配置（候选模型/选择策略/失败策略）。
fn onboarding_ensemble_configure(ctx: &GatewayContext, params: &Value) -> Value {
    let mut cfg = ctx.config.clone();
    if let Some(v) = params.get("enabled").and_then(Value::as_bool) {
        cfg.bloom.parallel_enabled = v;
    }
    if let Some(v) = params.get("selectionMode").and_then(Value::as_str) {
        cfg.bloom.selection_mode = v.to_string();
    }
    if let Some(v) = params.get("allFailedPolicy").and_then(Value::as_str) {
        cfg.bloom.all_failed_policy = v.to_string();
    }
    if let Some(candidates) = params.get("candidates") {
        cfg.bloom.candidates = candidates.clone();
    }
    match cfg.save() {
        Ok(()) => json!({ "ok": true, "saved": true }),
        Err(e) => json!({ "ok": false, "error": format!("保存集成配置失败: {e}") }),
    }
}

/// `onboarding.provider.configure`：保存模型服务商配置（BYOK API Key / base_url / model）。
fn onboarding_provider_configure(ctx: &GatewayContext, params: &Value) -> Value {
    let mut cfg = ctx.config.clone();
    let mut changed = false;
    if let Some(v) = params
        .get("provider")
        .or_else(|| params.get("providerId"))
        .and_then(Value::as_str)
    {
        cfg.model.provider = v.to_string();
        changed = true;
    }
    if let Some(v) = params.get("modelName").and_then(Value::as_str) {
        cfg.model.model_name = v.to_string();
        changed = true;
    }
    if let Some(v) = params.get("model_name").and_then(Value::as_str) {
        cfg.model.model_name = v.to_string();
        changed = true;
    }
    if let Some(v) = params.get("apiKey").and_then(Value::as_str) {
        cfg.model.api_key = v.to_string();
        changed = true;
    }
    if let Some(v) = params.get("api_key").and_then(Value::as_str) {
        cfg.model.api_key = v.to_string();
        changed = true;
    }
    if let Some(v) = params.get("baseUrl").and_then(Value::as_str) {
        cfg.model.base_url = v.to_string();
        changed = true;
    }
    if let Some(v) = params.get("base_url").and_then(Value::as_str) {
        cfg.model.base_url = v.to_string();
        changed = true;
    }
    if !changed {
        return json!({ "ok": false, "error": "没有需要保存的字段" });
    }
    match cfg.save() {
        Ok(()) => json!({ "ok": true, "saved": true, "restart_required": false }),
        Err(e) => json!({ "ok": false, "error": format!("保存服务商配置失败: {e}") }),
    }
}

/// modlens.analyze：图片(base64) → 结构化 JSON 文档（文字/阅读顺序/语义/版面）。
/// 无视觉模型时降级为元数据兜底（source=metadata-fallback），绝不静默失败。
async fn modlens_analyze(params: &Value) -> Value {
    use base64::Engine as _;
    let b64 = params
        .get("imageBase64")
        .or_else(|| params.get("image_base64"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let hint = params.get("hint").and_then(Value::as_str).unwrap_or("");
    let bytes = match base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
        Ok(b) => b,
        Err(e) => return json!({ "ok": false, "error": format!("图片 base64 解码失败: {e}") }),
    };
    if bytes.is_empty() {
        return json!({ "ok": false, "error": "图片数据为空" });
    }
    let doc = vision_singleton().analyze_document(&bytes, hint).await;
    match serde_json::from_str::<Value>(&doc.to_json()) {
        Ok(document) => json!({ "ok": true, "document": document }),
        Err(_) => json!({ "ok": false, "error": "视觉文档序列化失败" }),
    }
}

/// 进程级视觉能力单例（离线默认；后续可注入真实视觉模型 transport）。
fn vision_singleton() -> &'static crate::vision::VisionCapability {
    static V: std::sync::OnceLock<crate::vision::VisionCapability> = std::sync::OnceLock::new();
    V.get_or_init(crate::vision::VisionCapability::offline_default)
}

/// `onboarding.provider.probe`：验证 API Key 并尝试连接模型服务商。
async fn onboarding_provider_probe(params: &Value) -> Value {
    let api_key = params
        .get("apiKey")
        .or_else(|| params.get("api_key"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let base_url = params
        .get("baseUrl")
        .or_else(|| params.get("base_url"))
        .and_then(Value::as_str)
        .unwrap_or("https://api.deepseek.com")
        .to_string();
    let model_name = params
        .get("modelName")
        .or_else(|| params.get("model_name"))
        .and_then(Value::as_str)
        .unwrap_or("deepseek-chat")
        .to_string();
    let provider = params
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("deepseek")
        .to_string();

    if api_key.is_empty() {
        return json!({ "ok": false, "error": "API 密钥不能为空" });
    }

    // 构造一个最小的 chat completion 请求来验证连接
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model_name,
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 5,
        "stream": false,
    });

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": format!("HTTP 客户端创建失败: {e}") }),
    };

    match client
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                json!({
                    "ok": true,
                    "connected": true,
                    "provider": provider,
                    "model": model_name,
                    "message": "连接成功"
                })
            } else {
                let text = resp.text().await.unwrap_or_default();
                let detail = if text.len() > 200 {
                    &text[..200]
                } else {
                    &text
                };
                json!({
                    "ok": false,
                    "connected": false,
                    "error": format!("服务商返回错误 {}: {}", status.as_u16(), detail)
                })
            }
        }
        Err(e) => json!({
            "ok": false,
            "connected": false,
            "error": format!("连接失败: {e}")
        }),
    }
}

/// `config.patch` / `config.patch.safe` 的 Rust 本地实现（纯 Rust 离线模式兜底）。
///
/// 侧车在线时由 [`PY_FIRST_METHODS`] 优先代理 Python（读写同源）；侧车离线时
/// webui 保存设置走这里：将 dot-path 补丁落到内存 Config 并持久化到
/// `~/.marguerite/config.yaml`。当前支持 webui 设置页实际写入的键：
/// - `naming.enabled`
/// - `privacy.disable_network_observability`
/// - `memory.auto_capture_enabled`
/// - `memory.dream.enabled` / `memory.dream.auto_schedule`
/// - `squilla_router.self_learning.enabled`（启用时 linked 语义：同步打开 dream）
/// - `model.provider` / `model.model_name` / `model.base_url` / `model.api_key`（BYOK）
/// - `guard.syntax` / `guard.semantic` / `guard.fact` / `guard.contract`
/// - `bloom.parallel_enabled`
/// - `log.level`
///
/// 未知键静默忽略（保持向后兼容，不把不认识的内容写进 YAML）。
fn config_patch_local(ctx: &GatewayContext, params: &Value) -> Value {
    // 保存前从磁盘重载：ctx.config 是启动快照 clone（mode.switch 改写磁盘但不回写
    // ctx.config），若直接 clone 快照会因「值未变」误判 unchanged 而跳过落盘，导致
    // config.get（读磁盘）读到旧值、开关「回弹」。与 mode_switch / models.routing.set /
    // onboarding.router.configure 一致：保存前一律 Config::load() 取最新落盘态。
    let mut cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    let changed = Cell::new(false);
    let mut linked: Vec<String> = Vec::new();
    let mut restart_required = false;

    let apply_bool = |patches: &serde_json::Map<String, Value>, key: &str, slot: &mut bool| {
        if let Some(v) = patches.get(key).and_then(Value::as_bool) {
            if *slot != v {
                *slot = v;
                changed.set(true);
            }
        }
    };
    let apply_str = |patches: &serde_json::Map<String, Value>, key: &str, slot: &mut String| {
        if let Some(v) = patches.get(key).and_then(Value::as_str) {
            if *slot != v {
                *slot = v.to_string();
                changed.set(true);
            }
        }
    };

    // 兼容两种负载形态：`{patches: {...}}`（webui config.patch.safe 契约）与 `{patch: {...}}`
    let patches = params
        .get("patches")
        .or_else(|| params.get("patch"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    if let Some(naming) = patches.get("naming").and_then(Value::as_object) {
        apply_bool(naming, "enabled", &mut cfg.naming.enabled);
    }
    if let Some(privacy) = patches.get("privacy").and_then(Value::as_object) {
        apply_bool(
            privacy,
            "disable_network_observability",
            &mut cfg.privacy.disable_network_observability,
        );
    }
    if let Some(memory) = patches.get("memory").and_then(Value::as_object) {
        apply_bool(
            memory,
            "auto_capture_enabled",
            &mut cfg.memory.auto_capture_enabled,
        );
        if let Some(dream) = memory.get("dream").and_then(Value::as_object) {
            apply_bool(dream, "enabled", &mut cfg.dream.enabled);
            apply_bool(dream, "auto_schedule", &mut cfg.dream.auto_schedule);
        }
    }
    if let Some(router) = patches.get("squilla_router").and_then(Value::as_object) {
        if let Some(sl) = router.get("self_learning").and_then(Value::as_object) {
            let prev = cfg.squilla_router.enabled;
            apply_bool(sl, "enabled", &mut cfg.squilla_router.enabled);
            // 不对称 linked：开启 self-learning 自动打开 dream 链；
            // 关闭 dream 不触碰 self-learning（前端自己不会写这个路径）。
            if !prev && cfg.squilla_router.enabled {
                if !cfg.dream.enabled {
                    cfg.dream.enabled = true;
                    changed.set(true);
                    linked.push("memory.dream.enabled".to_string());
                }
                if !cfg.dream.auto_schedule {
                    cfg.dream.auto_schedule = true;
                    changed.set(true);
                    linked.push("memory.dream.auto_schedule".to_string());
                }
                // Rust 离线模式无后台训练循环 → 标注重启提示，避免 UI 上
                // 「已开启但不训练」造成误解；linkedLive=false 强制重启提示。
                restart_required = true;
            }
        }
    }
    // 模型设置组（webui Model 分节：BYOK provider / 模型名 / 端点 / API Key）。
    // Rust 离线模式此前静默丢弃 → 离线改模型/密钥保存后「消失」。现补齐落盘。
    if let Some(model) = patches.get("model").and_then(Value::as_object) {
        apply_str(model, "provider", &mut cfg.model.provider);
        apply_str(model, "model_name", &mut cfg.model.model_name);
        apply_str(model, "base_url", &mut cfg.model.base_url);
        apply_str(model, "api_key", &mut cfg.model.api_key);
    }
    // 护栏能力开关（webui 设置页 guard 分节：语法/语义/事实/契约检查）。
    if let Some(guard) = patches.get("guard").and_then(Value::as_object) {
        apply_bool(guard, "syntax", &mut cfg.guard.syntax);
        apply_bool(guard, "semantic", &mut cfg.guard.semantic);
        apply_bool(guard, "fact", &mut cfg.guard.fact);
        apply_bool(guard, "contract", &mut cfg.guard.contract);
    }
    // Bloom 并行编排开关（webui 模型策略/对话路由可见）。
    if let Some(bloom) = patches.get("bloom").and_then(Value::as_object) {
        apply_bool(bloom, "parallel_enabled", &mut cfg.bloom.parallel_enabled);
    }
    // 模型路由（webui 路由设置：direct/router/ensemble/bloom）。
    // 此前 config.patch 静默丢弃 model_routing.* → 经 config.patch 路径保存路由时「消失」。
    // 与 onboarding.router.configure 落盘键对称，保证两条保存路径都持久化。
    if let Some(routing) = patches.get("model_routing").and_then(Value::as_object) {
        apply_str(routing, "mode", &mut cfg.model_routing.mode);
        apply_str(routing, "fusionModel", &mut cfg.model_routing.fusion_model);
        apply_str(routing, "fusion_model", &mut cfg.model_routing.fusion_model);
        apply_str(routing, "fixedModel", &mut cfg.model_routing.fixed_model);
        apply_str(routing, "fixed_model", &mut cfg.model_routing.fixed_model);
        apply_bool(routing, "crossProviderTiers", &mut cfg.model_routing.cross_provider_tiers);
        apply_bool(routing, "cross_provider_tiers", &mut cfg.model_routing.cross_provider_tiers);
        for key in ["candidateModels", "candidate_models"] {
            if let Some(candidates) = routing.get(key).and_then(Value::as_array) {
                let v: Vec<String> = candidates
                    .iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect();
                if cfg.model_routing.candidate_models != v {
                    cfg.model_routing.candidate_models = v;
                    changed.set(true);
                }
            }
        }
    }
    // 日志级别（webui 日志分节）。
    if let Some(log) = patches.get("log").and_then(Value::as_object) {
        apply_str(log, "level", &mut cfg.log.level);
    }
    // dot-path 形态（webui 部分旧调用直接发 `memory.auto_capture_enabled` 等扁平键）
    apply_bool(&patches, "naming.enabled", &mut cfg.naming.enabled);
    apply_bool(
        &patches,
        "privacy.disable_network_observability",
        &mut cfg.privacy.disable_network_observability,
    );
    apply_bool(
        &patches,
        "memory.auto_capture_enabled",
        &mut cfg.memory.auto_capture_enabled,
    );
    apply_bool(&patches, "memory.dream.enabled", &mut cfg.dream.enabled);
    apply_bool(
        &patches,
        "memory.dream.auto_schedule",
        &mut cfg.dream.auto_schedule,
    );
    // dot-path 形态：模型 / 护栏 / Bloom / 日志
    apply_str(&patches, "model.provider", &mut cfg.model.provider);
    apply_str(&patches, "model.model_name", &mut cfg.model.model_name);
    apply_str(&patches, "model.base_url", &mut cfg.model.base_url);
    apply_str(&patches, "model.api_key", &mut cfg.model.api_key);
    apply_bool(&patches, "guard.syntax", &mut cfg.guard.syntax);
    apply_bool(&patches, "guard.semantic", &mut cfg.guard.semantic);
    apply_bool(&patches, "guard.fact", &mut cfg.guard.fact);
    apply_bool(&patches, "guard.contract", &mut cfg.guard.contract);
    apply_bool(
        &patches,
        "bloom.parallel_enabled",
        &mut cfg.bloom.parallel_enabled,
    );
    // dot-path 形态：模型路由
    apply_str(&patches, "model_routing.mode", &mut cfg.model_routing.mode);
    apply_str(&patches, "model_routing.fusionModel", &mut cfg.model_routing.fusion_model);
    apply_str(&patches, "model_routing.fixedModel", &mut cfg.model_routing.fixed_model);
    apply_bool(&patches, "model_routing.crossProviderTiers", &mut cfg.model_routing.cross_provider_tiers);
    apply_str(&patches, "log.level", &mut cfg.log.level);
    {
        let prev = cfg.squilla_router.enabled;
        apply_bool(
            &patches,
            "squilla_router.self_learning.enabled",
            &mut cfg.squilla_router.enabled,
        );
        if !prev && cfg.squilla_router.enabled {
            if !cfg.dream.enabled {
                cfg.dream.enabled = true;
                changed.set(true);
                linked.push("memory.dream.enabled".to_string());
            }
            if !cfg.dream.auto_schedule {
                cfg.dream.auto_schedule = true;
                changed.set(true);
                linked.push("memory.dream.auto_schedule".to_string());
            }
            restart_required = true;
        }
    }

    if !changed.get() {
        return json!({ "ok": true, "restartRequired": false, "changed": false, "linked": [] });
    }

    match cfg.save() {
        Ok(()) => {
            let mut res = json!({
                "ok": true,
                "restartRequired": restart_required,
                "changed": true,
            });
            if !linked.is_empty() {
                res["linked"] = json!(linked);
                // linkedLive=false：告知前端 dream 链变更并非立即生效（需 Python 侧车
                // 或后续重启）；让 MemoryLearningGroup 展示 "requires restart" 提示。
                res["linkedLive"] = json!(!restart_required);
            }
            res
        }
        Err(e) => json!({ "ok": false, "error": format!("配置保存失败: {e}") }),
    }
}

/// `mode.get`（Rust 同步）：返回当前**有效**模式。
///
/// 有效模式 = 运行时模式（`mode.switch` 优先）？否则回退静态配置
/// `bloom.parallel_enabled`（启动时配置）。前端据此渲染模式选择器当前态。
fn mode_get() -> Value {
    let runtime = crate::mode::runtime_mode();
    let effective = if runtime == crate::mode::ModeKind::Bloom {
        "bloom"
    } else {
        // 静态配置兜底：`bloom.parallel_enabled=true` 但未显式 switch 时仍视为 bloom
        let cfg = Config::load()
            .map(|c| c.bloom.parallel_enabled)
            .unwrap_or(false);
        if cfg {
            "bloom"
        } else {
            runtime.as_str()
        }
    };
    json!({
        "kind": effective,
        "runtimeKind": runtime.as_str(),
        "available": ["plan", "agent", "bloom"],
    })
}

/// `mode.switch`（Rust 异步分支）：切换运行时模式并持久化到配置。
///
/// 持久化：写 `config.yaml` 的 `bloom.parallel_enabled`（bloom=true / 其余=false），
/// 保证重启后仍保持；同时更新进程级运行时模式（立即生效，本次会话即走 bias）。
/// 返回切换后有效模式与 `mode.get` 同构的快照。
async fn mode_switch(ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    let kind = params
        .get("kind")
        .and_then(Value::as_str)
        .or_else(|| params.get("mode").and_then(Value::as_str))
        .unwrap_or("agent");
    let target = crate::mode::ModeKind::from_name(kind)
        .ok_or_else(|| RpcError::new("INVALID_REQUEST", format!("无效模式: {kind}")))?;

    // 1) 更新进程级运行时模式（立即生效）
    crate::mode::set_runtime_mode(target);

    // 2) 持久化到 config.yaml（bloom ↔ parallel_enabled），重启后仍保持。
    let mut cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    cfg.bloom.parallel_enabled = target == crate::mode::ModeKind::Bloom;
    if let Err(e) = cfg.save() {
        tracing::warn!("mode.switch：配置持久化失败（运行时模式已生效，重启后可能回退）: {e}");
    }

    // 3) 广播 mode.changed 事件（前端所有客户端实时同步模式选择器）
    ctx.bus.publish(crate::events::Envelope::mode_changed(
        "core",
        "agent",
        target.as_str(),
    ));

    Ok(json!({
        "ok": true,
        "kind": target.as_str(),
        "runtimeKind": crate::mode::runtime_mode().as_str(),
    }))
}

/// `models.routing.set`（Rust 原生）：设置模型路由模式并持久化到 config.yaml。
///
/// `mode` ∈ {direct, router, ensemble, bloom}；写 `model_routing.mode`，重启后仍保持。
/// 回传快照与 `models.routing.get` 同构；非法值返回 INVALID_REQUEST。
async fn models_routing_set(ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    let mode = params
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("direct")
        .trim()
        .to_lowercase();
    if !matches!(mode.as_str(), "direct" | "router" | "ensemble" | "bloom") {
        return Err(RpcError::new(
            "INVALID_REQUEST",
            format!("无效路由模式: {mode}（应为 direct/router/ensemble/bloom）"),
        ));
    }
    let mut cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    cfg.model_routing.mode = mode.clone();
    if let Err(e) = cfg.save() {
        tracing::warn!("models.routing.set：配置持久化失败（运行时已生效，重启后可能回退）: {e}");
    }
    Ok(json!({
        "ok": true,
        "mode": mode,
    }))
}

/// `models.routing.get`（Rust 原生）：读取当前模型路由模式。
fn models_routing_get(ctx: &GatewayContext) -> Value {
    let cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    json!({
        "ok": true,
        "mode": cfg.model_routing.mode,
    })
}

/// `router.selflearning.status` Rust 基线实现
///
/// 真训练（样本捕获 / 训练 / kill-switch / receipts）由 Python 侧车驱动；
/// Rust 离线模式返回「配置启用与否 + 无数据闸门」的最小基线，让
/// MemoryLearningGroup 组件有状态可展示，而不是 RPC 501 整张卡片灰掉。
fn router_selflearning_status(ctx: &GatewayContext) -> Value {
    let cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    let sl_enabled = cfg.squilla_router.enabled;
    let dream_enabled = cfg.dream.enabled;
    let dream_auto = cfg.dream.auto_schedule;
    json!({
        "enabled": sl_enabled,
        "captureEnabled": cfg.memory.auto_capture_enabled,
        // 训练可达需要 dream 开 + 调度开；Rust 离线模式始终不可达
        "trainingReachable": false,
        "dream": {
            "enabled": dream_enabled,
            "autoSchedule": dream_auto,
            "killSwitchActive": false,
        },
        "activeModel": serde_json::Value::Null,
        "samples": {
            "total": 0,
            "highValue": 0,
            "requiredHighValue": 50,
            "complaintRate": 0.0,
            "lastCapturedAt": serde_json::Value::Null,
            "feedback": { "up": 0, "down": 0, "downSingle": 0 },
        },
        "gate": {
            "wouldTrain": false,
            "reason": if !sl_enabled {
                "self-learning 未启用"
            } else if !dream_enabled || !dream_auto {
                "Dream 未启用（自学习依赖 Dream 调度）"
            } else {
                "Rust 离线模式未提供训练循环（重启加载 Python 侧车后开始训练）"
            },
            "lastAttemptAt": serde_json::Value::Null,
            "lastTrainedAt": serde_json::Value::Null,
            "killSwitchActive": false,
        },
        "lastReceipt": serde_json::Value::Null,
        "error": serde_json::Value::Null,
    })
}

/// `plans.capabilities`：返回 domain/capabilities 注册表的「机器可校验能力清单」
///
/// 客户端 `ChatView` 用 `rpc.supportsMethod('plans.capabilities')` 作为守卫，
/// 之前因该方法缺失导致整个能力诊断入口被隐藏（即「能力看起来根本没有」）。
/// 同时暴露：
///   - `list`: 扁平能力清单（含 maturity、tier、group、description、supersedes）
///   - `groups`: 按 CapabilityGroup 分组（前端能力树渲染）
///   - `stats`: total/exclusive/enhanced/baseline/implemented/narrative
///   - `implementedIds`: 仅 Real+Partial 的 id 集合，供 `supportsMethod()` 之外的
///     UI 条件渲染快速判定。
fn plans_capabilities() -> Value {
    use crate::domain::capabilities::{by_group, registry, stats};
    let list: Vec<serde_json::Value> = registry()
        .iter()
        .map(|c| {
            serde_json::to_value(c).unwrap_or_else(|_| {
                serde_json::json!({ "id": c.id.0, "name": c.name.clone(), "description": c.description.clone() })
            })
        })
        .collect();
    let groups: Vec<serde_json::Value> = by_group()
        .into_iter()
        .map(|(g, caps)| {
            let ids: Vec<String> = caps.iter().map(|c| c.id.0.clone()).collect();
            serde_json::json!({
                "group": serde_json::to_value(g).unwrap_or(serde_json::json!("tooling")),
                "groupLabel": g.label(),
                "ids": ids,
            })
        })
        .collect();
    let s = stats();
    let implemented_ids: Vec<String> = registry()
        .iter()
        .filter(|c| c.maturity.is_implemented())
        .map(|c| c.id.0.clone())
        .collect();
    let narrative_ids: Vec<String> = registry()
        .iter()
        .filter(|c| !c.maturity.is_implemented())
        .map(|c| c.id.0.clone())
        .collect();
    serde_json::json!({
        "protocol": 1,
        "list": list,
        "groups": groups,
        "stats": {
            "total": s.total,
            "exclusive": s.exclusive,
            "enhanced": s.enhanced,
            "baseline": s.baseline,
            "implemented": s.implemented,
            "narrative": s.narrative,
            "implementedRatio": if s.total == 0 { 0.0 } else { (s.implemented as f64) / (s.total as f64) },
        },
        "implementedIds": implemented_ids,
        "narrativeIds": narrative_ids,
        "generatedAtMs": chrono::Utc::now().timestamp_millis(),
    })
}

fn sandbox_status(ctx: &GatewayContext) -> Value {
    let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    json!({
        "policy": {
            "mode": "safe",
            "interpreters": whitelist::INTERPRETERS,
            "interactive": false,
        },
        "workspace": workspace.to_string_lossy(),
        "config_model": ctx.config.model.model_name,
    })
}

fn sessions_list(_ctx: &GatewayContext, params: &Value) -> Value {
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .min(200) as usize;
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    // DB 记录优先；context 台账补齐「有数据但未落库」的会话（整轮未完成时
    // sessions.db 无行，但台账每轮都写，合并后会话不丢）。
    // 台账同时作为 project_path 的兜底源：DB 行已存在但 project_path 为空时
    // （例如旧库/未绑定即落盘），用台账的归属回填，防会话重启后掉进「最近」栏。
    let ledger_projects: std::collections::HashMap<String, Option<String>> =
        crate::context::session::context_sessions()
            .into_iter()
            .map(|cs| (cs.session, cs.project_path))
            .collect();
    let mut items: Vec<(i64, Value)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let running: std::collections::HashSet<String> = crate::gateway::chat::running_sessions()
        .into_iter()
        .collect();
    if let Ok(sessions) = manager.init().and_then(|_| manager.list_sessions(limit)) {
        for s in sessions {
            seen.insert(s.session_id.clone());
            // DB 行 project_path 为空时，尝试用台账归属回填（路径归一由前端比较完成）
            let project_path = s
                .project_path
                .clone()
                .or_else(|| ledger_projects.get(&s.session_id).cloned().flatten());
            let is_running = running.contains(&s.session_id);
            items.push((
                rfc3339_to_ms(&s.updated_at),
                json!({
                    // webui useSessions 契约：key/session/sessionKey 皆可；sessions|keys 顶层字段
                    "key": s.session_id,
                    "session": s.session_id,
                    "sessionKey": s.session_id,
                    "title": s.summary,
                    "summary": s.summary,
                    "sessionKind": "chat",
                    "surface": "web",
                    "conversationKind": "chat",
                    "effectiveAgentId": "main",
                    "messageCount": s.message_count,
                    "updatedAt": rfc3339_to_ms(&s.updated_at),
                    "createdAt": rfc3339_to_ms(&s.created_at),
                    "status": if is_running { "running" } else { "idle" },
                    "runStatus": if is_running { "running" } else { "idle" },
                    "subtitle": "",
                    "projectPath": project_path,
                }),
            ));
        }
    }
    // 台账会话（context JSON）：DB 无记录也展示，防止重启后会话消失。
    for cs in crate::context::session::context_sessions() {
        if seen.contains(&cs.session) {
            continue;
        }
        let is_running = running.contains(&cs.session);
        items.push((
            cs.updated_ms as i64,
            json!({
                "key": cs.session,
                "session": cs.session,
                "sessionKey": cs.session,
                "title": cs.title,
                "summary": cs.title,
                "sessionKind": "chat",
                "surface": "web",
                "conversationKind": "chat",
                "effectiveAgentId": "main",
                "messageCount": cs.message_count,
                "updatedAt": cs.updated_ms,
                "createdAt": cs.updated_ms,
                "status": if is_running { "running" } else { "idle" },
                "runStatus": if is_running { "running" } else { "idle" },
                "subtitle": "",
                "projectPath": cs.project_path,
            }),
        ));
    }
    // 按最近更新时间降序，再按 limit 截断
    items.sort_by(|a, b| b.0.cmp(&a.0));
    items.truncate(limit);
    let items: Vec<Value> = items.into_iter().map(|(_, v)| v).collect();
    let keys: Vec<Value> = items.iter().map(|v| v["key"].clone()).collect();
    json!({ "sessions": items, "keys": keys })
}

/// RFC3339 → 毫秒时间戳（webui 会话列表按 ms 排序/展示；解析失败返回 0）
fn rfc3339_to_ms(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}

/// `chat.history`：从 sessions.db 水合会话历史（与 `chat.send` 落盘配套）。
/// DB 无记录时回退到 context 台账（整轮未完成/中途关闭的会话仍有历史可读）。
fn chat_history(_ctx: &GatewayContext, params: &Value) -> Value {
    let session = params
        .get("sessionKey")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if session.is_empty() {
        return json!({ "messages": [], "has_more": false });
    }
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    let messages = match manager.init().and_then(|_| manager.load_snapshot(session)) {
        Ok(snap) => snap.messages,
        Err(_) => Vec::new(),
    };
    let from_db = !messages.is_empty();
    // DB 无记录 → 回退 context 台账，保证「列表可见的会话」历史可读。
    let entries = if from_db {
        Vec::new()
    } else {
        crate::context::session::context_entries(session)
    };
    let out: Vec<Value> = if from_db {
        messages
            .iter()
            .enumerate()
            .map(|(idx, m)| {
                let (role, text) = match m {
                    crate::types::Message::User { content, .. } => ("user", content.clone()),
                    crate::types::Message::Assistant { content, .. } => {
                        ("assistant", content.clone())
                    }
                    crate::types::Message::System { content, .. } => ("system", content.clone()),
                    crate::types::Message::Tool {
                        name,
                        input,
                        output,
                        ..
                    } => ("tool", format!("[{name}] {input}\n→ {output}")),
                };
                let ts = m.timestamp().timestamp_millis();
                json!({
                    "role": role,
                    "text": text,
                    "timestamp": ts,
                    "ts": ts,
                    "id": format!("hist-{idx}"),
                    "message_id": format!("hist-{idx}"),
                })
            })
            .collect()
    } else {
        entries
            .iter()
            .enumerate()
            .map(|(idx, e)| {
                let role = if e.role == "user" {
                    "user"
                } else {
                    "assistant"
                };
                json!({
                    "role": role,
                    "text": e.text,
                    "timestamp": e.at_ms,
                    "ts": e.at_ms,
                    "id": format!("hist-{idx}"),
                    "message_id": format!("hist-{idx}"),
                })
            })
            .collect()
    };
    json!({
        "messages": out,
        "has_more": false,
        "hasMore": false,
        "canonical_available": !out.is_empty(),
        "canonicalAvailable": !out.is_empty(),
        "canonical_complete": true,
        "canonicalComplete": true,
        "returned": out.len(),
    })
}

fn tools_catalog() -> Value {
    let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut reg = ToolRegistry::new();
    let catalog: Vec<ToolInfo> =
        match register_builtin_tools(&mut reg, &workspace, None, None, None) {
            Ok(()) => reg.catalog(),
            Err(_) => Vec::new(),
        };
    json!({
        "items": catalog.iter().map(|t| json!({
            "name": t.name,
            "description": t.description,
            "source": t.source,
            "server": t.server,
        })).collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------
// 技能（SkillLoader 枚举本地技能目录，映射 WebUI Skill 契约）
// ---------------------------------------------------------------------------

/// 技能目录解析：优先配置路径；相对路径依次按 进程 cwd → 逐级上溯（对齐
/// `resolve_webui_dir` 的探测策略）解析；仍不存在时回退到数据目录 `skills/`
/// （首次自动创建）。修复「打包后双击 exe 技能页为空」：cwd 不再决定成败。
fn skills_dir(ctx: &GatewayContext) -> PathBuf {
    let configured = PathBuf::from(&ctx.config.skills.skills_dir);
    if configured.is_absolute() {
        if configured.is_dir() {
            return configured;
        }
    } else {
        // 相对路径：cwd 直连 + 逐级上溯锚定到工作区根
        if let Ok(cwd) = std::env::current_dir() {
            let direct = cwd.join(&configured);
            if direct.is_dir() {
                return direct;
            }
            let mut base = cwd.as_path();
            for _ in 0..6 {
                match base.parent() {
                    Some(p) => {
                        let candidate = p.join(&configured);
                        if candidate.is_dir() {
                            return candidate;
                        }
                        base = p;
                    }
                    None => break,
                }
            }
        }
    }
    let fallback = Config::data_dir().join("skills");
    let _ = std::fs::create_dir_all(&fallback);
    fallback
}

/// 把内核 Skill 映射为 WebUI `types/skills.ts` 的 Skill 视图
fn skill_view(s: &crate::skills::registry::Skill) -> Value {
    json!({
        "name": s.name,
        "description": s.description,
        "description_zh": s.description,
        "version": s.version,
        "author": s.author,
        "status": "ready",
        "layer": "bundled",
        "kind": "skill",
        "triggers": s.intent,
        "tools": s.tools,
        "dependencies": s.dependencies,
        "rating": s.rating,
        "uses": s.uses,
    })
}

fn skills_list(ctx: &GatewayContext) -> Value {
    let loader = SkillLoader::new(&skills_dir(ctx));
    let skills = match loader.load_all() {
        Ok(list) => list.iter().map(skill_view).collect::<Vec<_>>(),
        Err(e) => {
            tracing::warn!(err = %e, "skills.list 加载失败");
            Vec::new()
        }
    };
    json!({ "skills": skills })
}

fn skills_get(ctx: &GatewayContext, params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if name.is_empty() {
        return json!({ "error": "name 不可为空" });
    }
    let loader = SkillLoader::new(&skills_dir(ctx));
    match loader.load(name) {
        Ok(s) => skill_view(&s),
        Err(e) => {
            tracing::warn!(err = %e, "skills.get 未找到 {name}");
            json!({ "error": format!("技能 {name} 不存在") })
        }
    }
}

/// `skills.reload`：真重扫技能目录，返回变化摘要（added/removed）与最新 generation。
///
/// WebUI 手动刷新会据此展示「新增/移除/无变化」toast，随后重新拉取 `skills.list`。
fn skills_reload(ctx: &GatewayContext) -> Value {
    static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static SNAPSHOT: OnceLock<std::sync::Mutex<Vec<String>>> = OnceLock::new();
    let gen = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let loader = SkillLoader::new(&skills_dir(ctx));
    match loader.load_all() {
        Ok(list) => {
            let names: Vec<String> = list.iter().map(|s| s.name.clone()).collect();
            let snapshot = SNAPSHOT.get_or_init(|| std::sync::Mutex::new(Vec::new()));
            let mut prev = match snapshot.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            let added: Vec<String> = names
                .iter()
                .filter(|n| !prev.contains(n))
                .cloned()
                .collect();
            let removed: Vec<String> = prev
                .iter()
                .filter(|n| !names.contains(n))
                .cloned()
                .collect();
            *prev = names;
            json!({
                "success": true,
                "changed": !added.is_empty() || !removed.is_empty(),
                "partial": false,
                "generation": gen,
                "added": added,
                "removed": removed,
                "modified": [],
                "errors": [],
            })
        }
        Err(e) => {
            tracing::warn!(err = %e, "skills.reload 重扫失败");
            json!({
                "success": false,
                "changed": false,
                "partial": false,
                "generation": gen,
                "errors": [{ "message": format!("{e}") }],
            })
        }
    }
}

fn skills_search(ctx: &GatewayContext, params: &Value) -> Value {
    let query = params
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    if query.is_empty() {
        return json!({ "results": [] });
    }
    let loader = SkillLoader::new(&skills_dir(ctx));
    let results = match loader.load_all() {
        Ok(list) => list
            .iter()
            .filter(|s| {
                s.name.to_lowercase().contains(&query)
                    || s.description.to_lowercase().contains(&query)
                    || s.intent.iter().any(|i| i.to_lowercase().contains(&query))
            })
            .map(|s| {
                json!({
                    "name": s.name,
                    "description": s.description,
                    "identifier": s.name,
                    "source": "local",
                    "trust_level": "self",
                    "installed": true,
                })
            })
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    json!({ "results": results })
}

fn skills_uninstall(ctx: &GatewayContext, params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if name.is_empty() {
        return json!({ "success": false, "message": "name 不可为空" });
    }
    let loader = SkillLoader::new(&skills_dir(ctx));
    let path = loader.get_skill_path(name);
    if path.exists() {
        match std::fs::remove_file(&path) {
            Ok(()) => {
                json!({ "success": true, "name": name, "message": format!("已卸载技能 {name}") })
            }
            Err(e) => {
                json!({ "success": false, "name": name, "message": format!("卸载失败: {e}") })
            }
        }
    } else {
        json!({ "success": false, "name": name, "message": format!("技能 {name} 不存在") })
    }
}

/// 创建新技能（写入 YAML 文件到技能目录）
fn skills_create(ctx: &GatewayContext, params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let description = params
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let prompt_template = params
        .get("prompt_template")
        .and_then(Value::as_str)
        .or_else(|| params.get("promptTemplate").and_then(Value::as_str))
        .unwrap_or_default()
        .trim();
    if name.is_empty() || description.is_empty() || prompt_template.is_empty() {
        return json!({ "success": false, "message": "name / description / prompt_template 均不可为空" });
    }
    // 名称安全化
    let safe_name: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let dir = skills_dir(ctx);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return json!({ "success": false, "message": format!("创建技能目录失败: {e}") });
    }
    let path = dir.join(format!("{safe_name}.yaml"));
    if path.exists() {
        return json!({ "success": false, "message": format!("技能 '{safe_name}' 已存在") });
    }
    let triggers = params
        .get("triggers")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| format!("  - {s}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let indented: String = prompt_template
        .lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n");
    let content = format!(
        "name: {safe_name}\ndescription: {description}\nversion: \"1.0.0\"\nauthor: user\nintent:\n{triggers}\ntools: []\nprompt_template: |\n{indented}\n"
    );
    match std::fs::write(&path, content) {
        Ok(()) => {
            json!({ "success": true, "name": safe_name, "message": format!("技能 '{safe_name}' 已创建"), "path": path.display().to_string() })
        }
        Err(e) => json!({ "success": false, "message": format!("写入失败: {e}") }),
    }
}

/// 远程安装（async：真实下载 YAML 并注册到本地技能目录）
async fn skills_install(ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    let identifier = params
        .get("identifier")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let source = params
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if identifier.is_empty() {
        return Ok(json!({ "success": false, "message": "identifier 不可为空" }));
    }
    let is_url = identifier.starts_with("http://") || identifier.starts_with("https://");
    if is_url && (source.is_empty() || source == "github") {
        let loader = SkillLoader::new(&skills_dir(ctx));
        return match loader.download_and_register(identifier).await {
            Ok(s) => Ok(json!({
                "success": true,
                "name": s.name,
                "message": format!("已安装技能 {}", s.name),
            })),
            Err(e) => Ok(json!({ "success": false, "message": format!("安装失败: {e}") })),
        };
    }
    Ok(json!({
        "success": false,
        "message": "Rust 网关当前仅支持 http(s) URL 安装（registry 标识符离线无法拉取）",
    }))
}

// ---------------------------------------------------------------------------
// 长时任务（Bloom 长续航：心跳 / 检查点 / 暂停 / 续跑）
// ---------------------------------------------------------------------------

use crate::automation::long_running::{self, Stage};

/// `long_running.list`：列出全部长时任务（含检查点与心跳状态）
fn long_running_list() -> Value {
    let g = long_running::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tasks: Vec<Value> = g
        .list()
        .iter()
        .map(|t| long_running::task_view(t))
        .collect();
    json!({ "tasks": tasks })
}

/// `long_running.pause`：显式暂停（running/resumed → paused，等待续跑）
fn long_running_pause(params: &Value) -> Value {
    let task_id = params.get("taskId").and_then(Value::as_str).unwrap_or("");
    if task_id.is_empty() {
        return json!({ "error": "taskId 不可为空" });
    }
    let mut g = long_running::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match g.pause(task_id) {
        Ok(()) => match g.get(task_id) {
            Some(t) => json!({ "task": long_running::task_view(t) }),
            None => json!({ "error": "任务不存在" }),
        },
        Err(e) => json!({ "error": e.to_string() }),
    }
}

/// `long_running.resume`：从检查点续跑（paused → resumed，retry+1）
fn long_running_resume(params: &Value) -> Value {
    let task_id = params.get("taskId").and_then(Value::as_str).unwrap_or("");
    if task_id.is_empty() {
        return json!({ "error": "taskId 不可为空" });
    }
    let mut g = long_running::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match g.resume(task_id) {
        Ok(()) => match g.get(task_id) {
            Some(t) => json!({ "task": long_running::task_view(t) }),
            None => json!({ "error": "任务不存在" }),
        },
        Err(e) => json!({ "error": e.to_string() }),
    }
}

/// `long_running.checkpoint`：手动保存检查点（stage 可选，默认 execute）
fn long_running_checkpoint(params: &Value) -> Value {
    let task_id = params.get("taskId").and_then(Value::as_str).unwrap_or("");
    if task_id.is_empty() {
        return json!({ "error": "taskId 不可为空" });
    }
    let stage = match params
        .get("stage")
        .and_then(Value::as_str)
        .unwrap_or("execute")
    {
        "analyze" => Stage::Analyze,
        "plan" => Stage::Plan,
        "aggregate" => Stage::Aggregate,
        "refine" => Stage::Refine,
        _ => Stage::Execute,
    };
    let state = params.get("state").cloned().unwrap_or_else(|| json!({}));
    let mut g = long_running::global()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match g.save_checkpoint(task_id, stage, state) {
        Ok(()) => match g.get(task_id) {
            Some(t) => json!({ "task": long_running::task_view(t) }),
            None => json!({ "error": "任务不存在" }),
        },
        Err(e) => json!({ "error": e.to_string() }),
    }
}

// ---------------------------------------------------------------------------
// AI 智能融合路由（Bloom：离线纯规则自动决策编排形态）
// ---------------------------------------------------------------------------

/// `router.decision`：对输入做融合路由决策，返回自动编排形态（Solo / MAX 双带队 /
/// 专家团队 / 集群）与决策理由。与 chat 会话内的 Orchestrator 同源（analyze 纯规则，
/// 离线可用），供 Web UI 的 AI 智能融合路由面板只读展示。
fn router_decision(params: &Value) -> Value {
    let query = params
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if query.trim().is_empty() {
        return json!({ "error": "query 不可为空" });
    }
    static ROUTER: OnceLock<Orchestrator> = OnceLock::new();
    let orch = ROUTER.get_or_init(|| Orchestrator::new(Config::default()));
    let a = orch.analyze(query);
    json!({
        "strategy": a.strategy.label(),
        "intent": format!("{:?}", a.intent),
        "complexity": format!("{:?}", a.complexity),
        "needs_llm": a.needs_llm,
        "subtask_count": a.subtasks.len(),
        "subtasks": a.subtasks.iter().map(|s| json!({
            "role": s.role,
            "task": s.task,
        })).collect::<Vec<_>>(),
        "rationale": a.rationale,
    })
}

// ---------------------------------------------------------------------------
// 记忆层（ExperienceLedger 直连：语义检索 / 写入 / 列表）
// ---------------------------------------------------------------------------

fn open_ledger() -> Option<ExperienceLedger> {
    let mut ledger = ExperienceLedger::new(&Config::data_dir().join("marguerite_experience.db"));
    match ledger.init() {
        Ok(()) => Some(ledger),
        Err(e) => {
            tracing::warn!(err = %e, "记忆台账打开失败");
            None
        }
    }
}

fn experience_view(e: &crate::memory::Experience) -> Value {
    json!({
        "id": e.id,
        "kind": e.kind.as_str(),
        "title": e.title,
        "body": e.body,
        "tags": e.tags,
        "signature": e.signature,
        "validated": e.validated,
        "created_at": e.created_at,
    })
}

fn memory_search(params: &Value) -> Value {
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    if query.is_empty() {
        return json!({ "items": [] });
    }
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
    let items = match open_ledger() {
        Some(l) => match l.search(query, limit) {
            Ok(list) => list.iter().map(experience_view).collect::<Vec<_>>(),
            Err(e) => {
                tracing::warn!(err = %e, "memory.search 失败");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    json!({ "items": items })
}

fn memory_save(params: &Value) -> Value {
    let kind = crate::memory::ExperienceKind::from_kind_str(
        params
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("pitfall"),
    );
    let title = params
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let body = params
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if title.is_empty() || body.is_empty() {
        return json!({ "saved": false, "message": "title 与 body 不可为空" });
    }
    let tags: Vec<String> = params
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let signature = params
        .get("signature")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            format!("{title}{body}").hash(&mut h);
            format!("auto:{:x}", h.finish())
        });
    match open_ledger() {
        Some(mut l) => match l.record(kind, title, body, &tags, &signature) {
            Ok(id) => json!({ "saved": true, "id": id, "kind": kind.as_str() }),
            Err(e) => json!({ "saved": false, "message": format!("保存失败: {e}") }),
        },
        None => json!({ "saved": false, "message": "记忆台账不可用" }),
    }
}

fn memory_list(params: &Value) -> Value {
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
    let kind = params.get("kind").and_then(Value::as_str);
    let items = match open_ledger() {
        Some(l) => {
            let res = if let Some(k) = kind {
                l.list_by_kind(crate::memory::ExperienceKind::from_kind_str(k), limit)
            } else {
                l.list_recent(limit)
            };
            match res {
                Ok(list) => list.iter().map(experience_view).collect::<Vec<_>>(),
                Err(e) => {
                    tracing::warn!(err = %e, "memory.list 失败");
                    Vec::new()
                }
            }
        }
        None => Vec::new(),
    };
    json!({ "items": items })
}

// ───────────────────────────── 原生功能 helper（Game Hall / 图谱 / 桌宠）─────────────────────────────

/// 统一把 `Result<T, String>` 映射为 RPC 返回值：成功 `to_value`，失败 `{ok:false,error}`。
fn ok_or_err<T: serde::Serialize>(res: Result<T, String>) -> Value {
    match res {
        Ok(v) => serde_json::to_value(v).unwrap_or(json!({ "ok": false, "error": "序列化失败" })),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

fn param_str<'a>(params: &'a Value, camel: &str, snake: &str, default: &'a str) -> &'a str {
    params
        .get(camel)
        .or_else(|| params.get(snake))
        .and_then(Value::as_str)
        .unwrap_or(default)
}

fn param_i64(params: &Value, camel: &str, snake: &str, default: i64) -> i64 {
    params
        .get(camel)
        .or_else(|| params.get(snake))
        .and_then(Value::as_i64)
        .unwrap_or(default)
}

// ── 知识图谱 ──

fn kb_graph_view() -> Value {
    // 关系图谱自动化：从经验台账（历史记忆）自动抽取实体-关系索引进图，
    // 使图谱反映历史记录/上下文/会话沉淀，而非仅手动灌入。
    auto_index_experience_graph();

    let graph = match crate::kb::graph::global_graph().lock() {
        Ok(g) => g,
        Err(_) => return json!({ "ok": false, "error": "图谱锁获取失败" }),
    };
    let mut degree: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for e in graph.edges() {
        *degree.entry(e.from.clone()).or_insert(0) += 1;
        *degree.entry(e.to.clone()).or_insert(0) += 1;
    }
    let nodes: Vec<Value> = graph
        .entities()
        .into_iter()
        .map(|id| json!({ "id": id, "degree": degree.get(id).copied().unwrap_or(0) }))
        .collect();
    let edges: Vec<Value> = graph
        .edges()
        .iter()
        .map(|e| json!({ "from": e.from, "to": e.to, "kind": e.kind.label(), "weight": e.weight }))
        .collect();
    json!({ "nodes": nodes, "edges": edges, "entity_count": graph.entity_count(), "edge_count": graph.edge_count() })
}

/// 把经验台账（历史记忆）的最近经验自动索引进全局图谱（幂等，按 doc_id 去重）。
fn auto_index_experience_graph() {
    let db_path = crate::persistence::config::Config::data_dir().join("marguerite_experience.db");
    if !db_path.exists() {
        return;
    }
    let mut ledger = crate::memory::experience::ExperienceLedger::new(&db_path);
    if ledger.init().is_err() {
        return;
    }
    let exps = match ledger.list_recent(100) {
        Ok(v) => v,
        Err(_) => return,
    };
    if let Ok(mut graph) = crate::kb::graph::global_graph().lock() {
        for e in &exps {
            let doc_id = format!("exp:{}", e.id);
            if graph.entities_in_document(&doc_id).is_empty() {
                let text = format!("{} {}", e.title, e.body);
                graph.index_document(&doc_id, &text);
            }
        }
    }
}

fn kb_graph_related(params: &Value) -> Value {
    let entity = param_str(params, "entity", "entity", "");
    let graph = match crate::kb::graph::global_graph().lock() {
        Ok(g) => g,
        Err(_) => return json!({ "ok": false, "error": "图谱锁获取失败" }),
    };
    json!({ "entity": entity, "related": graph.neighbors(entity) })
}

// ── 鲸鱼娘 ──

fn pet_action(params: &Value) -> Value {
    let action = param_str(params, "action", "action", "");
    ok_or_err(crate::workbench::pet::interact(action))
}

// ── 德州扑克 ──

fn poker_state(params: &Value) -> Value {
    let room_id = param_str(params, "roomId", "room_id", "");
    let hub = match crate::poker::global_hub().lock() {
        Ok(h) => h,
        Err(_) => return json!({ "ok": false, "error": "大厅锁获取失败" }),
    };
    match hub.room(room_id) {
        Some(r) => serde_json::to_value(r).unwrap_or(json!({ "ok": false, "error": "序列化失败" })),
        None => json!({ "ok": false, "error": "房间不存在" }),
    }
}

fn poker_create(params: &Value) -> Value {
    let name = param_str(params, "name", "name", "牌局").to_string();
    let sb = param_i64(params, "smallBlind", "small_blind", 5);
    let bb = param_i64(params, "bigBlind", "big_blind", 10);
    let mut hub = match crate::poker::global_hub().lock() {
        Ok(h) => h,
        Err(_) => return json!({ "ok": false, "error": "大厅锁获取失败" }),
    };
    let id = hub.create_room(name, sb, bb);
    json!({ "ok": true, "roomId": id })
}

fn poker_join(params: &Value) -> Value {
    let room_id = param_str(params, "roomId", "room_id", "");
    let name = param_str(params, "playerName", "player_name", "玩家").to_string();
    let tokens = param_i64(params, "tokens", "tokens", 1000);
    let is_human = params
        .get("isHuman")
        .or_else(|| params.get("is_human"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut hub = match crate::poker::global_hub().lock() {
        Ok(h) => h,
        Err(_) => return json!({ "ok": false, "error": "大厅锁获取失败" }),
    };
    ok_or_err(hub.join_room(room_id, name, tokens, is_human))
}

fn poker_start(params: &Value) -> Value {
    let room_id = param_str(params, "roomId", "room_id", "");
    let mut hub = match crate::poker::global_hub().lock() {
        Ok(h) => h,
        Err(_) => return json!({ "ok": false, "error": "大厅锁获取失败" }),
    };
    ok_or_err(hub.start_room(room_id))
}

fn poker_do_action(params: &Value) -> Value {
    use crate::poker::ActionKind;
    let room_id = param_str(params, "roomId", "room_id", "");
    let seat = params
        .get("seat")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let action = param_str(params, "action", "action", "");
    let amount = params
        .get("amount")
        .and_then(Value::as_i64);
    let kind = match action {
        "fold" => ActionKind::Fold,
        "check" => ActionKind::Check,
        "call" => ActionKind::Call,
        "raise" => ActionKind::Raise,
        "all_in" => ActionKind::AllIn,
        other => return json!({ "ok": false, "error": format!("未知动作: {other}") }),
    };
    let mut hub = match crate::poker::global_hub().lock() {
        Ok(h) => h,
        Err(_) => return json!({ "ok": false, "error": "大厅锁获取失败" }),
    };
    ok_or_err(hub.act(room_id, seat, kind, amount))
}

fn poker_auto(params: &Value) -> Value {
    use crate::poker::{evaluate_7, ActionKind};
    let room_id = param_str(params, "roomId", "room_id", "");
    let mut hub = match crate::poker::global_hub().lock() {
        Ok(h) => h,
        Err(_) => return json!({ "ok": false, "error": "大厅锁获取失败" }),
    };
    for _ in 0..64 {
        let (hand_over, cur, is_human, to_call, tokens, has_pair, street_high, min_raise) = {
            let room = match hub.room(room_id) {
                Some(r) => r,
                None => return json!({ "ok": false, "error": "房间不存在" }),
            };
            let idx = room.current_seat as usize;
            let p = &room.players[idx];
            let has_pair = match (p.hole[0], p.hole[1]) {
                (Some(a), Some(b)) => evaluate_7([a, b], &room.board).category >= 1,
                _ => false,
            };
            (
                room.hand_over,
                room.current_seat,
                p.is_human,
                room.street_high - p.street_committed,
                p.tokens,
                has_pair,
                room.street_high,
                room.min_raise,
            )
        };
        if hand_over {
            return json!({ "ok": true });
        }
        if is_human {
            return json!({ "ok": true });
        }
        let (action, amount) = if to_call <= 0 {
            if has_pair && tokens > min_raise * 2 {
                (ActionKind::Raise, Some(street_high + min_raise))
            } else {
                (ActionKind::Check, None)
            }
        } else if to_call >= tokens {
            if has_pair {
                (ActionKind::AllIn, None)
            } else {
                (ActionKind::Fold, None)
            }
        } else {
            (ActionKind::Call, None)
        };
        if let Err(e) = hub.act(room_id, cur, action, amount) {
            return json!({ "ok": false, "error": e });
        }
    }
    json!({ "ok": true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::protocol::{SUPPORTED_EVENTS, SUPPORTED_METHODS};
    use std::sync::{Mutex, OnceLock};

    /// 测试配置根目录守卫（避免 ~/.marguerite 被并发测试互相覆写）
    ///
    /// 不依赖 `tempdir` 外部 crate（Cargo.toml 未引入）：用 `std::env::temp_dir()`
    /// + UUID 构造唯一路径，drop 时递归删除，模拟 TempDir。
    struct ConfigHome {
        dir: PathBuf,
    }
    impl ConfigHome {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("mgt-cfg-{}", uuid::Uuid::new_v4()));
            let _ = std::fs::create_dir_all(&dir);
            Config::set_config_dir_override(dir.clone());
            // 清空既有 config.yaml（如果被并发创建）
            let p = Config::config_path();
            let _ = std::fs::remove_file(&p);
            Self { dir }
        }
    }
    impl Drop for ConfigHome {
        fn drop(&mut self) {
            let p = Config::config_path();
            let _ = std::fs::remove_file(&p);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    // OnceLock + Mutex：让 CONF_HOME 在所有 methods 单测中共享同一个临时目录，
    // 同时避免「异步 config 写 + 并行 load」污染 ~/.marguerite。
    static CONF_HOME_GUARD: OnceLock<Mutex<ConfigHome>> = OnceLock::new();
    fn ensure_config_home() {
        CONF_HOME_GUARD.get_or_init(|| Mutex::new(ConfigHome::new()));
    }

    fn ctx() -> GatewayContext {
        ensure_config_home();
        // 防止其他线程写入中途读到中间态；用 Mutex guard 保持短锁一次。
        // 注意：须 poison-tolerant——某测试持锁 panic 会毒化 Mutex，若此处 `.unwrap()`
        // 会让后续所有 ctx() 级联崩溃（表现为 1944:56 成片 panic）。与 long_running 的
        // lock_tests 同款恢复策略。
        let _g = CONF_HOME_GUARD
            .get()
            .unwrap()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        GatewayContext {
            config: Config::default(),
            token: "test-token".to_string(),
            started_at: Instant::now(),
            bus: Arc::new(EventBus::default()),
        }
    }

    #[test]
    fn test_health_contract() {
        let v = health(&ctx());
        assert_eq!(v["status"], "ok");
        assert_eq!(v["engine"], "rust");
        assert_eq!(v["protocol"], PROTOCOL_VERSION);
    }

    #[test]
    fn test_tools_catalog_includes_new_tools() {
        // 联网检索 / 隔离代码执行 / 桌面控制增强 应自动出现在 RPC tools.catalog 中
        let v = tools_catalog();
        let names: Vec<&str> = v["items"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|i| i["name"].as_str()).collect())
            .unwrap_or_default();
        assert!(names.contains(&"web_search"), "缺 web_search");
        assert!(names.contains(&"web_research"), "缺 web_research");
        assert!(names.contains(&"code_exec"), "缺 code_exec");
        assert!(names.contains(&"system_control"), "缺 system_control");
    }

    #[test]
    fn test_dispatch_known_and_unknown() {
        let c = ctx();
        assert!(dispatch_sync(&c, "health", &Value::Null).is_ok());
        assert!(dispatch_sync(&c, "config.get", &Value::Null).is_ok());
        assert!(dispatch_sync(&c, "sessions.list", &json!({ "limit": 10 })).is_ok());
        // 融合方法族：技能 / 渠道 / 记忆在 Rust 网关内直接可用
        assert!(dispatch_sync(&c, "skills.list", &Value::Null).is_ok());
        assert!(dispatch_sync(&c, "skills.reload", &Value::Null).is_ok());
        assert!(dispatch_sync(&c, "channels.status", &Value::Null).is_ok());
        assert!(dispatch_sync(&c, "channels.pairings", &Value::Null).is_ok());
        assert!(dispatch_sync(&c, "memory.list", &json!({ "limit": 10 })).is_ok());
        // 异步方法（chat.*）走 dispatch_sync 返回结构化 ASYNC_ONLY 错误（须走异步 dispatch）
        let err = dispatch_sync(&c, "chat.send", &json!({})).unwrap_err();
        assert_eq!(err.code, "ASYNC_ONLY");
    }

    #[test]
    fn test_onboarding_save_closure_reflects_in_status() {
        // 回归：修复「新安装用户配置 DeepSeek API Key 后引导 banner 不消失 /
        // 设置页误以为保存失败」。直连 dispatch_sync（与 WS RPC 同一条 dispatch_rust 路径），
        // 用前端真实 camelCase 字段名（providerId + apiKey），验证两处修复：
        //   1) onboarding.provider.configure 必须兼容前端字段名 providerId（契约错配修复）；
        //   2) onboarding.status 必须读 Config::load()（磁盘 + .env），而非 GatewayContext
        //      内存快照——否则保存只落盘不回写内存，banner 永不消失。
        // 配置目录走 CONF_HOME_GUARD 临时覆盖，绝不触碰 ~/.marguerite。
        let mut c = ctx();
        // 显式让快照为空 key（与真实新用户未配置态一致；即便 .env 提供 key，快照仍为空）。
        // 这正是 bug 的对照关键：onboarding.status 若读「内存快照」则永远空 key；
        // 修复后读 Config::load()（含 .env）则非空——故 needsOnboarding==false 可区分修复前后。
        c.config.model.api_key.clear();
        let _g = CONF_HOME_GUARD
            .get()
            .unwrap()
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        // 用前端真实字段名保存；providerId 故意用非默认值的 qwen，以区分「字段名修复」
        let save = dispatch_sync(
            &c,
            "onboarding.provider.configure",
            &json!({
                "providerId": "qwen",
                "apiKey": "sk-TEST123",
                "baseUrl": "https://api.deepseek.com",
                "modelName": "deepseek-chat",
            }),
        )
        .unwrap();
        assert_eq!(save["ok"], true);
        assert_eq!(save["saved"], true);

        // 字段名契约修复 + 落盘：直接校验原始 config.yaml（不被 .env 污染）。
        // apply_env_overrides 会读真实 ~/.marguerite/.env 覆盖 Config::load() 的 provider/api_key，
        // 故不能断言 Config::load().provider；原始落盘文件才是 configure 行为的真凭。
        // 若后端只读 snake_case `provider`（契约错配），则落盘 provider 为默认 deepseek 而非 qwen。
        let raw = std::fs::read_to_string(Config::config_path()).unwrap();
        assert!(
            raw.contains("provider: qwen"),
            "providerId 字段未被后端识别（字段名契约错配 bug 复发）：落盘 provider 非 qwen"
        );
        assert!(
            raw.contains("sk-TEST123"),
            "api_key 未落盘到 config.yaml（保存链路断裂：用户「不能保存成功」）"
        );

        // 核心修复：onboarding.status 必须读 Config::load()（含磁盘落盘 + .env），而非内存快照。
        // 回退到快照（空 key）会让 needsOnboarding 变 true——正是用户看到的「保存失败」假象。
        // 本环境真实 ~/.marguerite/.env 含 MARGUERITE_API_KEY，恰好使 load() 恒非空：
        // 修复后 needsOnboarding==false（读 load），回退后 ==true（读空快照），故该断言可守护修复。
        let after = dispatch_sync(&c, "onboarding.status", &Value::Null).unwrap();
        assert_eq!(
            after["needsOnboarding"], false,
            "onboarding.status 未重读磁盘（内存快照 bug 复发）"
        );
        assert_eq!(after["llmSource"], "config");
        assert_eq!(after["hasConfig"], true);
        assert_eq!(after["llmConfigured"], true);
    }

    #[test]
    fn test_mode_get_and_switch_runtime() {
        // 运行时模式：mode.switch 立即生效，mode.get 与之同构；persist 到 config.yaml。
        let c = ctx();
        // 与 config.patch.* 系列共享同一 config.yaml：mode.switch 会写
        // `bloom.parallel_enabled`，而 mode.get 会读它判定终态。必须持锁串行化，
        // 否则与 test_config_patch_local_persists_model_guard_bloom_offline 等并行写
        // 会互相覆写（偶发 @1987 / @2100 失败）。ctx() 已 poison-tolerant。
        let _g = CONF_HOME_GUARD
            .get()
            .unwrap()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // 复位为 agent（避免并行测试污染全局运行时模式）
        let _ = crate::mode::set_runtime_mode(crate::mode::ModeKind::Agent);
        // 同时复位共享 config.yaml 的 bloom.parallel_enabled：mode.get 的「config 兜底」
        // 分支会读 Config::load().bloom.parallel_enabled 判定终态，仅复位运行时全局不够——
        // 若被并行/前置测试（如 config.patch bloom=true）污染为 true，runtime=Agent 时仍
        // 误报 "bloom"。此处建立干净基线，使断言与测试执行顺序无关。
        let mut cfg = c.config.clone();
        cfg.bloom.parallel_enabled = false;
        let _ = cfg.save();

        // mode.get 默认 agent
        let g = dispatch_sync(&c, "mode.get", &Value::Null).unwrap();
        assert_eq!(g["kind"], "agent");
        assert!(g["available"].as_array().unwrap().len() == 3);

        // mode.switch → bloom（异步分支）
        let rt = tokio::runtime::Runtime::new().unwrap();
        let r = rt
            .block_on(mode_switch(&c, &json!({ "kind": "bloom" })))
            .unwrap();
        assert_eq!(r["kind"], "bloom");
        // 进程级运行时模式已更新
        assert_eq!(crate::mode::runtime_mode(), crate::mode::ModeKind::Bloom);
        // 持久化到 config.yaml（重启后仍保持）
        let persisted = Config::load().unwrap();
        assert!(persisted.bloom.parallel_enabled);
        // mode.get 反映 bloom
        let g2 = dispatch_sync(&c, "mode.get", &Value::Null).unwrap();
        assert_eq!(g2["kind"], "bloom");

        // 切回 agent → parallel_enabled 复位 false
        let _ = rt
            .block_on(mode_switch(&c, &json!({ "kind": "agent" })))
            .unwrap();
        assert_eq!(crate::mode::runtime_mode(), crate::mode::ModeKind::Agent);
        assert!(!Config::load().unwrap().bloom.parallel_enabled);

        // 无效模式 → INVALID_REQUEST
        let err = rt
            .block_on(mode_switch(&c, &json!({ "kind": "bogus" })))
            .unwrap_err();
        assert_eq!(err.code, "INVALID_REQUEST");
    }

    #[test]
    fn test_bloom_mode_switch_and_config_roundtrip() {
        // 统一 Bloom（无 submode）：mode.switch 写 `bloom.parallel_enabled`（YAML），
        // mode.get / config.get 对称读回；非法模式被拒绝。
        let c = ctx();
        let _g = CONF_HOME_GUARD
            .get()
            .unwrap()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = crate::mode::set_runtime_mode(crate::mode::ModeKind::Agent);
        let mut cfg = c.config.clone();
        cfg.bloom.parallel_enabled = false;
        let _ = cfg.save();

        let rt = tokio::runtime::Runtime::new().unwrap();
        // 切到 bloom（异步分支）
        let r = rt
            .block_on(mode_switch(&c, &json!({ "kind": "bloom" })))
            .unwrap();
        assert_eq!(r["kind"], "bloom");
        assert_eq!(Config::load().unwrap().bloom.parallel_enabled, true);
        // mode.get 反映 bloom
        let g = dispatch_sync(&c, "mode.get", &Value::Null).unwrap();
        assert_eq!(g["kind"], "bloom");

        // config.patch.safe 写 bloom.parallel_enabled → config.get 对称读回
        let v = dispatch_sync(
            &c,
            "config.patch.safe",
            &json!({
                "patches": { "bloom": { "parallel_enabled": false } }
            }),
        )
        .unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(config_get(&c)["bloom"]["parallel_enabled"], false);

        // 非法模式被拒绝
        let err = rt
            .block_on(mode_switch(&c, &json!({ "kind": "bogus" })))
            .unwrap_err();
        assert_eq!(err.code, "INVALID_REQUEST");

        // 复位：agent，避免污染共享 config.yaml
        let _ = rt
            .block_on(mode_switch(&c, &json!({ "kind": "agent" })))
            .unwrap();
    }

    #[test]
    fn test_config_rpc_read_write_symmetry_listing() {
        // 设置页保存链路依赖 config.patch.safe（写）与 config.get（读）同源。
        let expected = [
            "config.get",
            "config.effective",
            "config.patch",
            "config.patch.safe",
        ];
        for m in expected {
            assert!(
                PY_FIRST_METHODS.contains(&m),
                "PY_FIRST_METHODS 缺少设置页读写链路方法: {m}"
            );
        }
    }

    #[test]
    fn test_config_patch_local_persists_naming_and_privacy() {
        // 纯 Rust 离线模式：config.patch.safe 本地落盘后 config.get 必须读回新值，
        // 避免「写 Python 全量、读 Rust 精简」导致设置开关保存后自动回弹。
        let c = ctx();
        // 全部 config 写读测试共享同一临时配置目录，patch→get 临界区必须持有
        // CONF_HOME_GUARD，否则与 test_config_patch_local_accepts_dot_path_form
        // 并行写会互相覆写（串行跑时通过、全量并行时偶发失败）。
        let _g = CONF_HOME_GUARD
            .get()
            .unwrap()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let v = dispatch_sync(
            &c,
            "config.patch.safe",
            &json!({
                "patches": {
                    "naming": { "enabled": false },
                    "privacy": { "disable_network_observability": true },
                    "memory": { "auto_capture_enabled": false },
                }
            }),
        )
        .unwrap();
        assert_eq!(v["ok"], true, "本地 patch 应成功: {v}");
        assert_eq!(v["changed"], true, "patch 应标记 changed");

        let got = config_get(&c);
        assert_eq!(got["naming"]["enabled"], false);
        assert_eq!(got["privacy"]["disable_network_observability"], true);
        assert_eq!(got["memory"]["auto_capture_enabled"], false);
    }

    #[test]
    fn test_config_patch_local_accepts_dot_path_form() {
        // webui 部分旧调用直接发扁平 dot-path 键。
        let c = ctx();
        // 与上个测试共享配置目录：临界区串行化写读。
        let _g = CONF_HOME_GUARD
            .get()
            .unwrap()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let v = dispatch_sync(
            &c,
            "config.patch",
            &json!({
                "patch": { "memory.auto_capture_enabled": false }
            }),
        )
        .unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(config_get(&c)["memory"]["auto_capture_enabled"], false);
    }

    #[test]
    fn test_config_patch_local_persists_model_guard_bloom_offline() {
        // 纯 Rust 离线模式：此前 model/guard/bloom 键被静默丢弃 → 用户改模型/API Key/
        // 护栏开关保存后「消失」（假开关）。现必须落盘且 config.get 对称读回。
        let c = ctx();
        let _g = CONF_HOME_GUARD
            .get()
            .unwrap()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let v = dispatch_sync(
            &c,
            "config.patch.safe",
            &json!({
                "patches": {
                    "model": {
                        "provider": "openai",
                        "model_name": "gpt-4o",
                        "base_url": "https://api.openai.com/v1",
                        "api_key": "sk-test-123",
                    },
                    "guard": { "syntax": false, "semantic": true, "fact": false, "contract": true },
                    "bloom": { "parallel_enabled": true },
                    "log": { "level": "debug" },
                }
            }),
        )
        .unwrap();
        assert_eq!(v["ok"], true, "本地 patch 应成功: {v}");
        assert_eq!(v["changed"], true);

        let got = config_get(&c);
        // guard 组对称读回（config_get 此前不返回 guard → 必回弹）
        assert_eq!(got["guard"]["syntax"], false);
        assert_eq!(got["guard"]["semantic"], true);
        assert_eq!(got["guard"]["fact"], false);
        assert_eq!(got["guard"]["contract"], true);
        // bloom 组对称读回
        assert_eq!(got["bloom"]["parallel_enabled"], true);
        assert_eq!(got["log"]["level"], "debug");

        // model.* 落盘验证：此前离线模式静默丢弃，现必须写入 config.yaml。
        // 注：运行时 MARGUERITE_* 环境变量按设计优先级（env > .env > YAML）覆盖 YAML，
        // 故此处仅校验「已落盘」而非「读回即生效」——落盘成功即证明不再被丢弃。
        let content = std::fs::read_to_string(crate::persistence::config::Config::config_path())
            .expect("config.yaml 应已落盘");
        assert!(
            content.contains("provider: openai"),
            "model.provider 应落盘: {content}"
        );
        assert!(
            content.contains("model_name: gpt-4o"),
            "model.model_name 应落盘"
        );
        assert!(content.contains("api.openai.com"), "model.base_url 应落盘");

        // 非 env 覆盖字段：磁盘重载与读回一致（证明写→读闭环）
        let reloaded = crate::persistence::config::Config::load().unwrap();
        assert!(!reloaded.guard.syntax);
        assert!(reloaded.guard.contract);
        assert!(reloaded.bloom.parallel_enabled);
        assert_eq!(reloaded.log.level, "debug");

        // 复位 bloom.parallel_enabled，避免污染共享 config.yaml：
        // test_mode_get_and_switch_runtime 依赖该字段终态（bloom 切 true / agent 切 false）。
        let _ = dispatch_sync(
            &c,
            "config.patch.safe",
            &json!({
                "patches": { "bloom": { "parallel_enabled": false } }
            }),
        );
    }

    #[test]
    fn test_baota_credentials_set_get_clear_roundtrip() {
        // 凭据 RPC 往返：set → get（不回显密码）→ clear。
        let c = ctx();
        let _g = CONF_HOME_GUARD
            .get()
            .unwrap()
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let set = dispatch_sync(
            &c,
            "baota.credentials.set",
            &json!({
                "base_url": "http://127.0.0.1:8888/ABCD1234",
                "username": "admin",
                "password": "secret",
            }),
        )
        .unwrap();
        assert_eq!(set["ok"], true, "凭据保存应成功: {set}");

        let got = dispatch_sync(&c, "baota.credentials.get", &Value::Null).unwrap();
        assert_eq!(got["configured"], true);
        assert_eq!(got["base_url"], "http://127.0.0.1:8888/ABCD1234");
        assert_eq!(got["has_username"], true);
        let dumped = got.to_string();
        assert!(!dumped.contains("secret"), "get 不应暴露密码: {dumped}");

        // 非法地址被拒绝
        let bad = dispatch_sync(
            &c,
            "baota.credentials.set",
            &json!({
                "base_url": "ftp://nope", "username": "a", "password": "b",
            }),
        )
        .unwrap();
        assert_eq!(bad["ok"], false);

        let clear = dispatch_sync(&c, "baota.credentials.clear", &Value::Null).unwrap();
        assert_eq!(clear["ok"], true);
        let after = dispatch_sync(&c, "baota.credentials.get", &Value::Null).unwrap();
        assert_eq!(after["configured"], false);
    }

    #[test]
    fn test_supported_methods_subset_of_dispatch() {
        // HelloOk 暴露的方法必须都能被 dispatch 处理（不留悬空清单）；
        // 异步方法族由异步 dispatch 提供，跳过同步路径校验。
        // 画像导入 start/status/cancel/retry/apply/undo 需要真实 payload；
        // 子集测试只校验「可到达」：至少返回 ok 或业务错误（Rust 实现已接入），
        // 不校验 payload 有效性。
        for m in SUPPORTED_METHODS {
            if ASYNC_ONLY_METHODS.contains(m) {
                continue;
            }
            // memory.import.* 走了 Rust 实现，需根据方法传入最小有效 payload
            let dummy = match *m {
                "memory.import.info" | "memory.import.discard" => json!({
                    "schemaVersion": 1u64,
                }),
                "memory.import.status"
                | "memory.import.cancel"
                | "memory.import.retry"
                | "memory.import.apply"
                | "memory.import.undo" => json!({
                    "schemaVersion": 1u64,
                    "jobId": "job_not_found",
                }),
                "memory.import.start" => json!({
                    "schemaVersion": 1u64,
                    "rawText": "Hello，这里是最小有效导入文本",
                    "uiLocale": "zh-CN",
                }),
                // codeintel.* 传不存在 workspace，避免 index 扫描真实 crate 目录
                "codeintel.status" | "codeintel.index" | "codeintel.search"
                | "codeintel.symbols" => json!({
                    "workspace": ".__codeintel_probe__",
                }),
                _ => Value::Null,
            };
            let res = dispatch_sync(&ctx(), m, &dummy);
            // Ok 或业务 Err 都算「已实现」；NOT_IMPLEMENTED / 才失败
            if let Err("NOT_IMPLEMENTED") = res.as_ref().map_err(|e| e.code.as_str()) {
                panic!("SUPPORTED_METHODS 含未实现方法: {m}");
            }
        }
    }

    #[test]
    fn test_experts_list_returns_substantive_profiles() {
        let v = experts_list(&Value::Null);
        let experts = v["experts"].as_array().expect("experts 数组");
        assert_eq!(experts.len(), 8, "专家库应含 8 个专家");
        for e in experts {
            assert!(e["id"].is_string(), "专家缺 id: {e}");
            assert!(e["label"].is_string(), "专家缺 label: {e}");
            assert!(
                !e["lineage"].as_str().unwrap().is_empty(),
                "专家缺 lineage: {e}"
            );
            assert!(
                !e["mindset"].as_str().unwrap().is_empty(),
                "专家缺 mindset: {e}"
            );
            assert!(
                !e["skills"].as_array().unwrap().is_empty(),
                "专家缺专属技能: {e}"
            );
            assert!(
                !e["playbook"].as_array().unwrap().is_empty(),
                "专家缺 playbook: {e}"
            );
        }
    }

    #[test]
    fn test_agent_profile_analyze_roundtrip() {
        let v = agent_profile_analyze(&json!({
            "messages": [
                { "role": "user", "content": "帮我写个函数" },
                { "role": "assistant", "content": "好的，我写一段 rust 代码并部署到后端。" }
            ]
        }));
        assert_eq!(v["role"], "工程师");
        assert!(v["tone"].is_string());
        assert!(v["expertise"].is_array());
    }

    #[test]
    fn test_memory_layers_status_marks_reachability() {
        let v = memory_layers_status(&Value::Null);
        let layers = v["layers"].as_array().expect("layers 数组");
        assert_eq!(layers.len(), 7, "7 层记忆");
        // 本轮补齐的四层必须有 RPC 且 reachable
        for id in ["sensory", "short_term", "long_term", "recall"] {
            let l = layers.iter().find(|l| l["id"] == id).expect(id);
            assert_eq!(l["reachable"], true, "{id} 应已暴露 RPC: {l}");
            assert!(
                !l["rpc"].as_array().unwrap().is_empty(),
                "{id} 应含 RPC 方法: {l}"
            );
        }
    }

    #[test]
    fn test_skills_list_maps_webui_contract() {
        let c = ctx();
        let v = skills_list(&c);
        assert!(v["skills"].is_array(), "skills.list 应返回 skills 数组");
        for s in v["skills"].as_array().unwrap() {
            assert!(s["name"].is_string());
            assert_eq!(s["status"], "ready");
            assert_eq!(s["layer"], "bundled");
        }
    }

    #[test]
    fn test_channels_status_returns_empty_ledger() {
        let c = ctx();
        let v = dispatch_sync(&c, "channels.status", &Value::Null).unwrap();
        assert!(v["channels"].is_array());
        assert!(v["channels"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_router_decision_returns_fusion_routing() {
        let c = ctx();
        // 单意图解释 → Solo Agent
        let v = dispatch_sync(&c, "router.decision", &json!({ "query": "解释闭包" })).unwrap();
        assert_eq!(v["strategy"], "Solo Agent");
        assert!(v["rationale"].as_str().is_some_and(|r| !r.is_empty()));
        assert!(v["subtasks"].is_array());
        // 代码 + 测试 → 专家团队（至少 2 个子任务）
        let v2 = dispatch_sync(
            &c,
            "router.decision",
            &json!({ "query": "实现登录并写测试" }),
        )
        .unwrap();
        assert_eq!(v2["strategy"], "Expert Team");
        assert!(v2["subtask_count"].as_u64().unwrap() >= 2);
        // 空 query → 错误
        let v3 = dispatch_sync(&c, "router.decision", &json!({})).unwrap();
        assert!(v3.get("error").is_some());
    }

    #[test]
    fn test_supported_events_non_empty() {
        let events: Vec<&str> = SUPPORTED_EVENTS.to_vec();
        assert!(!events.is_empty(), "SUPPORTED_EVENTS 不应为空");
    }

    #[test]
    fn test_sandbox_policy_get_contract() {
        // 对齐 webui `SandboxPolicy` 契约（types/sandbox.ts）：
        // dispatch 链 RUST_FIRST_METHODS → dispatch_rust → policy_store::policy_get()
        // 必须返回合法策略结构，而非 NOT_IMPLEMENTED / 空值。
        let c = ctx();
        let v = dispatch_sync(&c, "sandbox.policy.get", &Value::Null)
            .expect("sandbox.policy.get 应被 Rust 网关实现");
        assert_eq!(v["schemaVersion"], 2);
        assert!(v["policyVersion"].as_u64().unwrap_or(0) >= 1);
        for key in ["files", "commands", "network", "runtimes"] {
            assert!(v[key].is_object(), "{key} 应为对象: {v}");
        }
        // 与 webui SandboxPolicy 字段逐项对齐
        assert!(v["files"]["customDenyWritePaths"].is_array());
        assert!(v["files"]["recursiveDeleteBackupEnabled"].is_boolean());
        assert!(v["commands"]["requireApprovalPrefixes"].is_array());
        assert!(v["commands"]["systemTools"].is_string());
        assert!(v["network"]["blockAllNetwork"].is_boolean());
        assert!(v["runtimes"]["enabled"].is_boolean());
        assert!(v["runtimes"]["python"].is_boolean());
        assert!(v["runtimes"]["node"].is_boolean());
        assert!(v["runtimes"]["gitBash"].is_boolean());
    }

    #[test]
    fn test_sandbox_policy_defaults_contract() {
        let c = ctx();
        let v = dispatch_sync(&c, "sandbox.policy.defaults", &Value::Null)
            .expect("sandbox.policy.defaults 应被 Rust 网关实现");
        assert!(v["builtinDenyWritePaths"].is_array());
        assert_eq!(v["runtimeTarget"], Value::Null);
        assert!(v["runtimeVersions"].is_object());
    }

    #[test]
    fn test_onboarding_catalog_rust_fallback() {
        let c = ctx();
        let v = dispatch_sync(&c, "onboarding.catalog", &Value::Null)
            .expect("onboarding.catalog should be implemented by Rust gateway");
        let providers = v["providers"].as_array().expect("providers array");
        assert!(
            !providers.is_empty(),
            "should have at least one built-in LLM provider"
        );
        for p in providers {
            assert!(
                p["providerId"].is_string(),
                "provider missing providerId: {p}"
            );
            assert!(p["label"].is_string(), "provider missing label: {p}");
            assert!(p["fields"].is_array(), "provider missing fields: {p}");
        }
        let ids: Vec<&str> = providers
            .iter()
            .filter_map(|p| p["providerId"].as_str())
            .collect();
        for expected in ["deepseek", "openai", "anthropic", "custom"] {
            assert!(ids.contains(&expected), "missing provider: {expected}");
        }
        assert!(v["searchProviders"].is_array());
        assert!(v["imageGenerationProviders"].is_array());
        assert!(v["memoryEmbeddingProviders"].is_array());
    }

    #[test]
    fn test_onboarding_status_rust_fallback() {
        let c = ctx();
        let v = dispatch_sync(&c, "onboarding.status", &Value::Null)
            .expect("onboarding.status should be implemented by Rust gateway");
        assert!(v["needsOnboarding"].is_boolean());
        assert!(v["llmConfigured"].is_boolean());
        assert!(v["sections"].is_object());
        assert!(v["warnings"].is_array());
    }
}

// ───────────── Fleet / Evolve / Plugins 网关方法 ─────────────

/// `fleet.list`：返回 Fleet 多角色模型路由配置
fn fleet_list() -> Value {
    let config = match crate::persistence::config::Config::load() {
        Ok(c) => c,
        Err(_) => return json!({ "roles": [], "error": "配置加载失败" }),
    };
    let llm = model_config_to_llm(&config.model);
    let fleet = crate::model::fleet::FleetConfig::with_defaults(&llm);
    let roles: Vec<Value> = fleet
        .roles
        .iter()
        .map(|(id, b)| {
            json!({
                "role_id": id,
                "label": b.label,
                "kind": format!("{:?}", b.kind),
                "provider": format!("{:?}", b.config.provider),
                "model": b.config.model,
                "reasoning": format!("{:?}", b.reasoning),
                "max_concurrency": b.max_concurrency,
                "system_prompt": b.system_prompt,
            })
        })
        .collect();
    json!({
        "roles": roles,
        "default_role": fleet.default_role,
        "failover_chain": fleet.failover_chain,
        "enabled": fleet.enabled,
    })
}

/// `fleet.route`：按任务描述推荐角色
fn fleet_route(params: &Value) -> Value {
    let task = params.get("task").and_then(Value::as_str).unwrap_or("");
    if task.is_empty() {
        return json!({ "error": "task 参数不可为空" });
    }
    let config = match crate::persistence::config::Config::load() {
        Ok(c) => c,
        Err(_) => return json!({ "error": "配置加载失败" }),
    };
    let llm = model_config_to_llm(&config.model);
    let fleet = crate::model::fleet::FleetConfig::with_defaults(&llm);
    let router = crate::model::fleet::FleetRouter::new(fleet);
    match router.route_by_task(task) {
        Some(d) => json!({
            "role_id": d.role_id,
            "source": format!("{:?}", d.source),
            "provider": format!("{:?}", d.binding.config.provider),
            "model": d.binding.config.model,
            "reasoning": format!("{:?}", d.binding.reasoning),
            "label": d.binding.label,
        }),
        None => json!({ "error": "无可用路由" }),
    }
}

/// `evolve.history`：自进化历史记录
fn evolve_history() -> Value {
    let data_dir = crate::persistence::config::Config::data_dir();
    let runner = crate::harness::evolve::EvolutionRunner::new(&data_dir, "");
    match runner.load_history() {
        Ok(history) => {
            let records: Vec<Value> = history
                .iter()
                .map(|r| {
                    json!({
                        "round": r.round,
                        "timestamp": r.timestamp,
                        "diagnosis": r.diagnosis,
                        "score_before": r.score_before,
                        "score_after": r.score_after,
                        "accepted": r.accepted,
                        "rejection_reason": r.rejection_reason,
                    })
                })
                .collect();
            json!({ "records": records, "total": records.len() })
        }
        Err(e) => json!({ "records": [], "error": e.to_string() }),
    }
}

/// `plugins.list`：已注册插件/工具槽位
fn plugins_list() -> Value {
    let caps = crate::domain::capabilities::registry();
    let items: Vec<Value> = caps
        .iter()
        .filter(|c| c.maturity.is_implemented())
        .map(|c| {
            json!({
                "name": c.id.0,
                "description": c.description,
                "group": format!("{:?}", c.group),
                "tier": c.tier.label(),
                "maturity": format!("{:?}", c.maturity),
            })
        })
        .collect();
    json!({ "plugins": items, "total": items.len() })
}

/// ModelConfig → LLMConfig 转换（与 GUI Tauri 命令同源）
fn model_config_to_llm(
    m: &crate::persistence::config::ModelConfig,
) -> crate::model::llm::LLMConfig {
    use crate::model::llm::{LLMConfig, LLMProvider};
    use crate::model::thinking::{ThinkingEffort, CONTEXT_WINDOW_DEFAULT};
    let provider = match m.provider.to_lowercase().as_str() {
        "openai" => LLMProvider::OpenAI,
        "deepseek" => LLMProvider::DeepSeek,
        _ => LLMProvider::Custom {
            base_url: m.base_url.clone(),
        },
    };
    LLMConfig {
        provider,
        model: m.model_name.clone(),
        api_key: m.api_key.clone(),
        base_url: m.base_url.clone(),
        max_tokens: 4096,
        temperature: 0.7,
        timeout_secs: 120,
        thinking: ThinkingEffort::Off,
        context_window: CONTEXT_WINDOW_DEFAULT,
    }
}
