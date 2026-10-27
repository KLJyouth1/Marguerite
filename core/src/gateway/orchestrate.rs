//! 对话编排 RPC（`orchestrate.run`）—— P2·E 对话编排交互层后端
//!
//! 把「编排能力」从 CLI / 后台面板提升到对话第一界面：对话输入框就是编排入口。
//! 统一编排指令 JSON 契约（对齐计划文档 118 行）：
//!
//! ```json
//! { "form": "single|subagent|team|expert|swarm", "mode": "standard_doit",
//!   "message": "...", "sessionKey": "...", "capabilities": [], "agents": [], "swarm_on": false }
//! ```
//!
//! 按 `form` 分发：
//! - `single`    → `chat.send` 语义（透传 message / sessionKey / thinking / effort）；
//! - `subagent`  → [`SubAgentManager::spawn_with_spec`]（角色感知子智能体，P2P 唤醒基座）；
//! - `team`      → `team.run`（`Orchestrator::dispatch_team`：自动角色检测 → 成员并行 → lead 综合）；
//! - `expert`    → 同一 backend（`team.run`），额外携带专家团成员提示；
//! - `swarm`     → `team.run.background`（long_running 台账 + 心跳/检查点/完成态）+ `fleet.route` 推荐。
//!
//! 全部形态统一上报 `agent.started / agent.stopped` 事件（webui 事件订阅面可见），
//! 并在进入执行前过 P2·D 权限矩阵：蜂群 / 团队等高自主形态在高档位外默认锁定，
//! 需显式授权（`confirmed: true`）后才放开 —— 「能力↑ → 控制↑」。

use serde_json::{json, Value};

use crate::domain::UserMode;
use crate::events::{Envelope, EventType, EventBus};
use crate::gateway::_helpers::param_str;
use crate::gateway::methods::GatewayContext;
use crate::gateway::protocol::RpcError;
use crate::guard::permission_matrix::{MatrixDecision, PermissionMatrix, ToolAutonomy};
use crate::persistence::config::Config;

/// 编排形态（统一编排契约的 `form` 字段，纯数据、可单测）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchestrateForm {
    Single,
    SubAgent,
    Team,
    Expert,
    Swarm,
}

impl OrchestrateForm {
    /// 从 `form` 字符串解析（大小写不敏感；空串按 single 处理）。
    pub fn from_key(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "single" => Some(OrchestrateForm::Single),
            "subagent" | "sub_agent" | "sub-agent" => Some(OrchestrateForm::SubAgent),
            "team" => Some(OrchestrateForm::Team),
            "expert" => Some(OrchestrateForm::Expert),
            "swarm" => Some(OrchestrateForm::Swarm),
            _ => None,
        }
    }

    /// 稳定机器标识。
    pub fn as_key(&self) -> &'static str {
        match self {
            OrchestrateForm::Single => "single",
            OrchestrateForm::SubAgent => "subagent",
            OrchestrateForm::Team => "team",
            OrchestrateForm::Expert => "expert",
            OrchestrateForm::Swarm => "swarm",
        }
    }

    /// 人类可读名（webui 卡片 / 事件播报）。
    pub fn label(&self) -> &'static str {
        match self {
            OrchestrateForm::Single => "单代理",
            OrchestrateForm::SubAgent => "子智能体",
            OrchestrateForm::Team => "智能体团队",
            OrchestrateForm::Expert => "专家团队",
            OrchestrateForm::Swarm => "蜂群",
        }
    }
}

/// 纯路由元数据：测试与执行共用，保证「测试覆盖的就是执行的」。
///
/// 返回 `{form, backend, args, capabilities}` —— 各形态应走的 backend 调用与参数。
/// 实际执行（[`run`]）用真实 handler 产物覆盖 `args` 之外的同名字段，后端名与参数
/// 永不漂移。
pub fn route_meta(
    form: OrchestrateForm,
    message: &str,
    agents: &[String],
    capabilities: &[String],
    swarm_on: bool,
) -> Value {
    let (backend, args) = match form {
        OrchestrateForm::Single => (
            "chat.send",
            json!({ "message": message, "sessionKey": null }),
        ),
        OrchestrateForm::SubAgent => (
            "SubAgentManager.spawn_with_spec",
            json!({
                "task": message,
                "context": "",
                // 全部选中角色都启动：此前只取 agents[0]，勾多个角色等于没勾。
                "roles": if agents.is_empty() {
                    vec!["orchestrator".to_string()]
                } else {
                    agents.to_vec()
                },
                "intent": "General",
                "complexity": "Medium"
            }),
        ),
        OrchestrateForm::Team => ("team.run", json!({ "goal": message })),
        OrchestrateForm::Expert => (
            "team.run",
            json!({ "goal": message, "expertPick": agents }),
        ),
        OrchestrateForm::Swarm => (
            "team.run.background",
            json!({ "goal": message, "swarm_on": swarm_on }),
        ),
    };
    json!({
        "form": form.as_key(),
        "backend": backend,
        "args": args,
        "capabilities": capabilities,
    })
}

/// 权限门：按 3×3 权限矩阵（P2·D）裁决编排形态的自主档位。
///
/// - `single`（对话发送）→ 只读档：任何档位放行；
/// - `subagent`（并联子智能体）→ 写档：咨询拒、方案需确认、执行档放行；
/// - `team` / `expert` / `swarm`（多智能体协同 / 蜂群）→ 高自主：Ask/Plan 拒绝，
///   极简锁（需显式授权），标准 / 极致需显式授权后才放行。
///
/// `confirmed` 即 RPC 层的显式授权（无宿主审批链，ApprovalGate 另走 security_seam）。
pub fn gate_form(
    form: OrchestrateForm,
    user_mode: UserMode,
    confirmed: bool,
) -> Result<(), RpcError> {
    let autonomy = match form {
        OrchestrateForm::Single => ToolAutonomy::ReadOnly,
        OrchestrateForm::SubAgent => ToolAutonomy::Mutating,
        OrchestrateForm::Team | OrchestrateForm::Expert | OrchestrateForm::Swarm => {
            ToolAutonomy::HighAutonomy
        }
    };
    match PermissionMatrix::new().check(user_mode, autonomy, confirmed) {
        MatrixDecision::Allow { .. } => Ok(()),
        MatrixDecision::Confirm { reason } => Err(RpcError::new(
            "ORCHESTRATE_CONFIRM_REQUIRED",
            format!("{reason}（形态={}，档位={}）", form.as_key(), user_mode.as_key()),
        )),
        MatrixDecision::Deny { reason } => Err(RpcError::new(
            "ORCHESTRATE_DENIED",
            format!("{reason}（形态={}，档位={}）", form.as_key(), user_mode.as_key()),
        )),
    }
}

/// `orchestration.status`：返回每个 run **最近一条**编排进度帧，用于丢帧/断线后的对账。
///
/// 为什么需要它：真机事故（2026-10-24）里，进度帧经 WS 推给浏览器，`run_completed`
/// 落在一次写超时/断档窗口内就永远补不回来 —— 卡片永久停在「运行中」，而独立 WS 客户端
/// 收到了终态。事件总线本来就有 2000 条历史缓冲（`EventBus::history`），但 WS 路径
/// 从不使用它。这里把「每个 run 的最后一条编排帧」做成一次轻量查询：客户端重连后、
/// 或发现编排长时间无进展时拉一次，就能把卡片收敛到真实终态，而不依赖「刚好在线」。
///
/// 为什么按 `runId` 而不是按会话聚一条：同一会话可能先后跑过多次编排，只要较早那次
/// 丢了终态，按会话只回「最后一条」就永远补不上它（旧卡片一直转圈）。
///
/// 只返回每个 run 的最后一条（而不是整段过程）：过程帧重放会重复插入工具行，
/// 终态帧才是幂等的。`coveredSince` 是本次查询真正覆盖的时间下界，前端据此判断
/// 「服务端完全没有这个 run 的任何记录」（网关重启 / 任务 panic）而不是「只是没查全」。
pub async fn status(ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    let session = param_str(params, "sessionKey", "sessionKey", "");
    // 默认窗口取 24h：事件总线只保留最近 2000 条，真正的限制是缓冲深度而不是时间；
    // 窗口过小反而会让「已经卡了很久」的卡片查不到自己的终态帧。
    let window_secs = params
        .get("windowSecs")
        .and_then(Value::as_u64)
        .unwrap_or(86_400)
        .clamp(30, 86_400);
    // 时间下界用墙钟（`chrono`）而不是 `Instant`：Windows 上 `Instant` 锚点是开机计时，
    // `Instant::now() - 24h` 在开机不足 24h 时下溢，过滤结果会变成空集。
    let covered_since =
        chrono::Utc::now() - chrono::Duration::seconds(window_secs.min(i64::MAX as u64) as i64);

    let mut latest: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    let frames = ctx.bus.history_all();
    // 缓冲里最旧一条的时间：前端据此区分「这个 run 真的没有记录」与「它太老被挤出了缓冲」。
    // 少了这个下界，高频会话把早期帧挤出 2000 条缓冲时，仍在运行的 run 会被误判为已丢失。
    let history_oldest = frames.first().map(|env| env.ts.to_rfc3339());
    for env in frames {
        if env.ts < covered_since {
            continue;
        }
        if env.r#type != EventType::Progress {
            continue;
        }
        if !session.is_empty() && env.session != session {
            continue;
        }
        let Some(obj) = env.payload.as_object() else {
            continue;
        };
        if obj.get("kind").and_then(Value::as_str) != Some("orchestration") {
            continue;
        }
        let run_id = obj
            .get("runId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        latest.insert(
            format!("{}|{}", env.session, run_id),
            json!({
                "sessionKey": env.session,
                "phase": obj.get("phase").cloned().unwrap_or(Value::Null),
                "runId": obj.get("runId").cloned().unwrap_or(Value::Null),
                "at": env.ts.to_rfc3339(),
                "payload": env.payload,
            }),
        );
    }

    Ok(json!({
        "runs": latest.into_values().collect::<Vec<_>>(),
        "windowSecs": window_secs,
        "coveredSince": covered_since.to_rfc3339(),
        "historyOldest": history_oldest,
    }))
}

/// `orchestrate.run`：统一编排入口。
pub async fn run(ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    let form_key = param_str(params, "form", "form", "single");
    let form = OrchestrateForm::from_key(form_key).ok_or_else(|| {
        RpcError::new(
            "INVALID_REQUEST",
            format!("form='{form_key}' 非法；合法值: single|subagent|team|expert|swarm"),
        )
    })?;

    let message = param_str(params, "message", "message", "").trim().to_string();
    if message.is_empty() {
        return Err(RpcError::new(
            "INVALID_REQUEST",
            "orchestrate.run 需要 message（目标任务描述）",
        ));
    }

    let mode_key = param_str(params, "mode", "mode", "standard_doit");
    let user_mode = UserMode::from_key(mode_key).ok_or_else(|| {
        RpcError::new(
            "INVALID_REQUEST",
            format!("mode='{mode_key}' 非法；合法值: minimal_ask|...|extreme_doit（3×3 档位键）"),
        )
    })?;

    let confirmed = params
        .get("confirmed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let swarm_on = params
        .get("swarm_on")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // 并行规模：输入框的 2/3/4/6/8 档位。必须一路传到后端扇出，否则只是文案旋钮。
    // 不再 `.min(16)`：用户选多少就传多少，数量上限只在「≥1」处做健全性检查。
    let swarm_size = params
        .get("swarmSize")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;

    let session_key = param_str(params, "sessionKey", "sessionKey", "");
    let session = if session_key.is_empty() {
        format!("ws.{}", uuid::Uuid::new_v4())
    } else {
session_key.to_string()
    };

    // 【2026-10-23 修复】编排入口必须自己落一次**用户消息**。
    //
    // 实证：从对话页发「只回复两个字：收到」→ 触发蜂群确认 → `orchestrate.run`；
    // 执行结束只有 assistant 回复进了 `session_messages`（team.rs 里那条），
    // 用户消息**一条都没有**。后果是重开应用后对话里只剩答案、问题消失，
    // 且会话标题永远是 "Empty session"（`append_message` 的标题种子取首条 user 文本）。
    //
    // `chat.send` 路径在 chat.rs 里已自己做同样的事；这里是另一条入口，必须对齐。
    // 先记后跑：即使执行失败/超时，用户输入的上下文也不该丢（与 chat.rs 同口径）。
    crate::gateway::chat::record_turn_persisted(&session, "user", &message);

    let capabilities: Vec<String> = params
        .get("capabilities")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let agents: Vec<String> = params
        .get("agents")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    // 团队 / 专家团（可多选）：必须一路传到执行层，否则用户在输入框里选的团队只是装饰。
    let teams: Vec<String> = params
        .get("teams")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    // 先过权限门：未授权的多智能体/蜂群形态不进入执行（能力↑ → 控制↑）。
    gate_form(form, user_mode, confirmed)?;

    // 统一事件面：agent.started（webui 事件订阅面可见编排入口）。
    publish_agent_event(
        &ctx.bus,
        &session,
        "agent.started",
        json!({
            "form": form.as_key(),
            "form_label": form.label(),
            "goal": message,
            "mode": user_mode.as_key(),
            "swarm_on": swarm_on,
            "agents": agents,
            "capabilities": capabilities,
        }),
    );

    let request = OrchestrationRequest {
        message: &message,
        session: &session,
        agents: &agents,
        teams: &teams,
        capabilities: &capabilities,
        swarm_on,
        swarm_size,
    };
    let result = dispatch_form(ctx, form, &request).await;

    // 统一事件面：agent.stopped（执行端自身的进度/工具事件已各自上报）。
    let ok = result.is_ok();
    publish_agent_event(
        &ctx.bus,
        &session,
        "agent.stopped",
        json!({ "form": form.as_key(), "ok": ok }),
    );

    let mut out = result?;
    out["form"] = json!(form.as_key());
    out["form_label"] = json!(form.label());
    out["mode"] = json!(user_mode.as_key());
    out["capabilities"] = json!(capabilities);
    Ok(out)
}

/// 一次编排请求的载荷（`dispatch_form` 的参数包）。
///
/// 独立成结构体而不是继续加形参：并行规模这类档位字段还会增加，8 个裸参数既过不了
/// clippy（too_many_arguments），也容易在调用处错位。
struct OrchestrationRequest<'a> {
    message: &'a str,
    session: &'a str,
    agents: &'a [String],
    /// 用户选定的团队 / 专家团 id（多选）。
    teams: &'a [String],
    capabilities: &'a [String],
    swarm_on: bool,
    swarm_size: usize,
}

/// 按形态真实分发到对应 backend。
async fn dispatch_form(
    ctx: &GatewayContext,
    form: OrchestrateForm,
    req: &OrchestrationRequest<'_>,
) -> Result<Value, RpcError> {
    let OrchestrationRequest {
        message,
        session,
        agents,
        teams,
        capabilities,
        swarm_on,
        swarm_size,
    } = req;
    match form {
        // single：chat.send 语义（透传全部参数，message/thinking/effort 等字段原样生效，
        // 编排层只附加 form/backend 标记）。
        OrchestrateForm::Single => {
            let res = crate::gateway::chat::send(ctx, &json!({
                "message": message,
                "sessionKey": session,
            }))
            .await?;
            Ok(json!({ "backend": "chat.send", "result": res }))
        }
        OrchestrateForm::SubAgent => {
            // 多选必须全跑：此前只取 `agents.first()` —— 用户勾了 3 个角色实际只启动 1 个，
            // 与团队形态的静默降级同源。空选择回落单个 orchestrator（旧行为不变）。
            let picks: Vec<String> = if agents.is_empty() {
                vec!["orchestrator".to_string()]
            } else {
                agents.to_vec()
            };
            let role_prompts: Vec<(String, String)> = picks
                .into_iter()
                .map(|role| {
                    let prompt = role_prompt_for(&role);
                    (role, prompt)
                })
                .collect();
            let (ids, summaries) = run_subagents(ctx, message, session, &role_prompts).await?;
            Ok(json!({
                "backend": "SubAgentManager.spawn_with_spec",
                // 兼容旧契约：`agentId` 给第一个，完整列表在 `agentIds`。
                "agentId": ids.first().cloned().unwrap_or_default(),
                "agentIds": ids,
                "agents": agents,
                "summaries": summaries
                    .iter()
                    .map(|(role, text)| json!({
                        "role": role,
                        "summary": text.chars().take(8000).collect::<String>(),
                    }))
                    .collect::<Vec<_>>(),
            }))
        }
        OrchestrateForm::Team | OrchestrateForm::Expert => {
            // 团队 / 专家团的选择必须进执行层：`team_run_with` 依据 `teams` 解析名册，
            // 按名册逐成员分派职责（此前只转发 goal，选项被完全忽略）。
            // `members` 也必须一并下发：并行规模是用户显式选择，名册模式下它是
            // 「至少几路并行」的下限（名册成员不会被它截断）。
            let mut v = crate::gateway::team::team_run_with(
                &json!({
                    "goal": message,
                    "sessionKey": session,
                    "teams": teams,
                    "members": swarm_size,
                    "form": form.as_key(),
                }),
                Some(ctx.bus.clone()),
            )
            .await;
            v["backend"] = json!("team.run");
            v["agents"] = json!(agents);
            v["teams"] = json!(teams);
            // 真实计数（便于前端核对「选了 6 个团队 / 8 路」是否落地）。
            v["agentCount"] = json!(agents.len());
            v["teamCount"] = json!(teams.len());
            v["requestedMembers"] = json!(swarm_size);
            let _ = capabilities;
            Ok(v)
        }
        OrchestrateForm::Swarm => {
            // 必须把 sessionKey 一并交给后台任务：进程内的蜂群进度与最终结论都要
            // 回写到**用户当前这条对话**，否则前端只能拿到一个 taskId（黑盒）。
            let mut v = crate::gateway::team::team_run_background_with(
                &json!({
                    "goal": message,
                    "sessionKey": session,
                    "members": swarm_size,
                    "teams": teams,
                    "form": "swarm",
                }),
                Some(ctx.bus.clone()),
            )
            .await;
            v["backend"] = json!("team.run.background");
            v["swarm_on"] = json!(swarm_on);
            v["agents"] = json!(agents);
            v["teams"] = json!(teams);
            // 真实计数：后台任务尚在跑，成员数由终态帧回报，这里给出请求规模与选择数。
            v["agentCount"] = json!(agents.len());
            v["teamCount"] = json!(teams.len());
            v["requestedMembers"] = json!(swarm_size);
            // 蜂群附带 fleet 角色推荐（供前端并行扇出建议，best-effort）。
            let route = crate::gateway::fleet::fleet_route(&json!({ "task": message }));
            v["fleet"] = route;
            Ok(v)
        }
    }
}

/// subagent 形态：按选中顺序并行启动全部角色，复用编排器装配好的**完整工具面**。
///
/// 数量不设上限：用户勾选多少角色就启动多少个（并发由 `SubAgentConfig::max_concurrent`
/// 闸门排队控制，超出的不会被执行侧丢弃，只是稍后启动）。
///
/// 历史缺陷（用户反馈「没有文件写入 / 没有 Office / 没有浏览器」的直接根因）：
/// 本函数曾自建 `ToolRegistry::new()`（空表）与裸 `ModelRouter`，子智能体拿不到
/// 任何内置工具；更上游还有 `register_builtin_tools` 因 Blender 缺失整体失败的问题，
/// 已在 `tools::builtin` 修复为按域可选注册。这里统一走 `Orchestrator::new`
/// （`assemble`：注册表 + 技能工具 + 权限补丁 + 事件总线），与团队/蜂群同源。
async fn run_subagents(
    ctx: &GatewayContext,
    task: &str,
    session: &str,
    role_prompts: &[(String, String)],
) -> Result<(Vec<String>, Vec<(String, String)>), RpcError> {
    // 与对话页同源解析生效模型：裸 `Config::load()` 会拿 config.yaml 的 provider，
    // 而 active 档案（models.json）可能指向另一个 provider，密钥分槽不允许跨 provider
    // 复用 → api_key 为空 → 每次请求 401（AUTH_FAILED）。
    let cfg = crate::model::selector::apply_effective_model(
        &Config::load().unwrap_or_else(|_| ctx.config.clone()),
    );
    let mut orch = crate::orchestration::Orchestrator::new(cfg);
    orch.set_event_bus(ctx.bus.clone(), session.to_string());
    let (ids, summaries) = orch.spawn_subagents_parallel(task, role_prompts).await;
    // 一个都没启动起来才算接口级失败；部分失败由 summaries 如实回报（不冒充成功）。
    if ids.is_empty() {
        let reasons = summaries
            .iter()
            .map(|(role, text)| format!("{role}: {text}"))
            .collect::<Vec<_>>()
            .join("；");
        return Err(RpcError::new(
            "SUBAGENT_SPAWN_FAILED",
            format!("子智能体启动失败：{reasons}"),
        ));
    }
    Ok((ids, summaries))
}

/// 角色人设（与 gui/src/App.tsx 同一角色集的轻量版，供 spawn_with_spec 注入）。
fn role_prompt_for(role: &str) -> String {
    match role {
        "researcher" => {
            "你是资深研究员：负责检索、核证与综述，输出带来源的证据链与关键结论。".to_string()
        }
        "coder" => "你是资深工程师：负责设计与实现，输出可编译、可验证的代码与自测证据。".to_string(),
        "sandbox" => "你是沙箱执行员：负责隔离验证与回测，输出可复现的实验证据。".to_string(),
        "browser" => "你是浏览器代理：负责打开页面、收集界面事实与截图证据。".to_string(),
        "guard" => "你是安全守卫：负责审计风险、红线与权限，输出合规报告。".to_string(),
        _ => "你是 orchestrator：负责任务分解、协调子代理并综合最终结论。".to_string(),
    }
}

/// 发布 `agent.*` 编排事件（webui 事件订阅面可见）。
fn publish_agent_event(bus: &EventBus, session: &str, kind: &str, payload: Value) {
    let evt = if kind == "agent.stopped" {
        Envelope::new(EventType::Done, payload, session)
    } else {
        Envelope::new(EventType::Status, payload, session)
    };
    bus.publish(evt);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `orchestration.status` 的对账口径：每个 run 只回最后一条、按会话过滤、
    /// 且必须给出覆盖下界（`coveredSince` / `historyOldest`）。
    ///
    /// 回归背景（真机 2026-10-24）：实现曾用 `Instant::now() - 24h` 取时间下界，
    /// Windows 上开机不足 24h 时下溢 → 回退成 `now` → 恒返回空 `runs`，
    /// 前端每 20s 对账一次也永远收敛不了。
    #[tokio::test]
    async fn status_returns_latest_frame_per_run_with_covered_bounds() {
        use crate::events::{Envelope, EventBus, EventType};
        let bus = std::sync::Arc::new(EventBus::new(64));
        for (run, phase) in [
            ("run-a", "run_started"),
            ("run-a", "run_completed"),
            ("run-b", "run_started"),
        ] {
            bus.publish(Envelope::new(
                EventType::Progress,
                json!({ "kind": "orchestration", "phase": phase, "runId": run }),
                "sess-1",
            ));
        }
        bus.publish(Envelope::new(
            EventType::Progress,
            json!({ "kind": "orchestration", "phase": "run_failed", "runId": "run-x" }),
            "sess-2",
        ));
        let ctx = GatewayContext {
            config: crate::persistence::config::Config::default(),
            token: "test".into(),
            started_at: std::time::Instant::now(),
            bus,
        };

        let v = status(&ctx, &json!({ "sessionKey": "sess-1" })).await.unwrap();
        let runs = v["runs"].as_array().expect("runs 必须是数组");
        assert_eq!(runs.len(), 2, "只回本会话、按 runId 去重后的结果：{v}");
        let a = runs.iter().find(|r| r["runId"] == "run-a").unwrap();
        assert_eq!(a["phase"], "run_completed", "同一 run 只保留最后一条：{v}");
        let b = runs.iter().find(|r| r["runId"] == "run-b").unwrap();
        assert_eq!(b["phase"], "run_started");
        assert!(v["coveredSince"].is_string());
        assert!(v["historyOldest"].is_string(), "非空缓冲必须给出最旧帧时间：{v}");

        // 空会话键 = 不按会话过滤（内部调用方语义），两个会话都应出现
        let all = status(&ctx, &json!({})).await.unwrap();
        assert_eq!(all["runs"].as_array().unwrap().len(), 3);
    }

    fn forms() -> [(OrchestrateForm, &'static str); 5] {
        [
            (OrchestrateForm::Single, "single"),
            (OrchestrateForm::SubAgent, "subagent"),
            (OrchestrateForm::Team, "team"),
            (OrchestrateForm::Expert, "expert"),
            (OrchestrateForm::Swarm, "swarm"),
        ]
    }

    #[test]
    fn form_key_roundtrip() {
        for (form, key) in forms() {
            assert_eq!(OrchestrateForm::from_key(key), Some(form));
            assert_eq!(form.as_key(), key);
        }
        // 兼容别名与大小写
        assert_eq!(OrchestrateForm::from_key("Sub-Agent"), Some(OrchestrateForm::SubAgent));
        assert_eq!(OrchestrateForm::from_key("SWARM"), Some(OrchestrateForm::Swarm));
        assert_eq!(OrchestrateForm::from_key(""), Some(OrchestrateForm::Single));
        assert_eq!(OrchestrateForm::from_key("swarmx"), None);
    }

    #[test]
    fn route_meta_maps_each_form_to_correct_backend_and_params() {
        let agents = vec!["researcher".to_string(), "coder".to_string()];
        let caps = vec!["web".to_string()];
        // single → chat.send，参数透传 message
        let v = route_meta(OrchestrateForm::Single, "你好", &agents, &caps, false);
        assert_eq!(v["backend"], "chat.send");
        assert_eq!(v["args"]["message"], "你好");
        assert_eq!(v["capabilities"][0], "web");
        // subagent → SubAgentManager.spawn_with_spec，**全部**选中角色都进 roles
        // （此前只取 agents[0]，多选等于没选；这条断言就是那个缺陷的回归守卫）
        let v = route_meta(OrchestrateForm::SubAgent, "做个方案", &agents, &caps, false);
        assert_eq!(v["backend"], "SubAgentManager.spawn_with_spec");
        assert_eq!(v["args"]["roles"], json!(["researcher", "coder"]));
        assert_eq!(v["args"]["intent"], "General");
        assert_eq!(v["args"]["complexity"], "Medium");
        // 未选角色时回落单个 orchestrator（旧行为不变）
        let v = route_meta(OrchestrateForm::SubAgent, "做个方案", &[], &caps, false);
        assert_eq!(v["args"]["roles"], json!(["orchestrator"]));
        // team → team.run，goal=message
        let v = route_meta(OrchestrateForm::Team, "重构模块", &agents, &caps, false);
        assert_eq!(v["backend"], "team.run");
        assert_eq!(v["args"]["goal"], "重构模块");
        // expert → team.run + expertPick
        let v = route_meta(OrchestrateForm::Expert, "评审查方案", &agents, &caps, false);
        assert_eq!(v["backend"], "team.run");
        assert_eq!(v["args"]["expertPick"], json!(["researcher", "coder"]));
        // swarm → team.run.background + swarm_on
        let v = route_meta(OrchestrateForm::Swarm, "全网调研", &agents, &caps, true);
        assert_eq!(v["backend"], "team.run.background");
        assert_eq!(v["args"]["swarm_on"], true);
    }

    #[test]
    fn gate_form_single_is_readonly_all_gears() {
        for mode in UserMode::all() {
            assert!(gate_form(OrchestrateForm::Single, *mode, false).is_ok());
        }
    }

    #[test]
    fn gate_form_subagent_requires_executive_gear() {
        // Ask 档：只读，子智能体（写档）拒绝
        assert!(gate_form(OrchestrateForm::SubAgent, UserMode::MinimalAsk, false).is_err());
        assert!(gate_form(OrchestrateForm::SubAgent, UserMode::ExtremeAsk, false).is_err());
        // Plan 档：默认只读规划，写操作需确认 → 未确认拒绝
        assert!(gate_form(OrchestrateForm::SubAgent, UserMode::StandardPlan, false).is_err());
        // DoIt 档：放行
        assert!(gate_form(OrchestrateForm::SubAgent, UserMode::StandardDoIt, false).is_ok());
        assert!(gate_form(OrchestrateForm::SubAgent, UserMode::ExtremeDoIt, false).is_ok());
    }

    #[test]
    fn gate_form_team_and_swarm_lock_outside_explicit_authorization() {
        // 高自主形态：Ask/Plan 一律拒绝（即使 confirmed）
        assert!(gate_form(OrchestrateForm::Team, UserMode::MinimalAsk, true).is_err());
        assert!(gate_form(OrchestrateForm::Expert, UserMode::StandardPlan, true).is_err());
        // 极简·执行：默认锁高自主，显式授权后才放行
        assert!(gate_form(OrchestrateForm::Swarm, UserMode::MinimalDoIt, false).is_err());
        assert!(gate_form(OrchestrateForm::Swarm, UserMode::MinimalDoIt, true).is_ok());
        // 标准 / 极致·执行：未授权需确认（RPC 层无宿主 → 拒绝），授权后放行
        assert!(gate_form(OrchestrateForm::Team, UserMode::StandardDoIt, false).is_err());
        assert!(gate_form(OrchestrateForm::Team, UserMode::StandardDoIt, true).is_ok());
        assert!(gate_form(OrchestrateForm::Expert, UserMode::ExtremeDoIt, true).is_ok());
        // 错误信息带形态与档位上下文
        let err = gate_form(OrchestrateForm::Swarm, UserMode::MinimalDoIt, false).unwrap_err();
        assert_eq!(err.code, "ORCHESTRATE_DENIED");
        assert!(err.message.contains("swarm"));
        assert!(err.message.contains("minimal_doit"));
    }
}