//! DSH × Marguerite 二创 Harness 桥接（T-1.4 + T-1.5）
//!
//! 本模块把 DSH 10 款插件声明的 lifecycle hooks / Router profile 与 Marguerite 原生
//! `HookRunner`、`EventBus`、`agent::multi_strategy::FleetRouter` 连起来。
//!
//! ## T-1.4 三类 Hooks 对应关系
//!
//! | DSH 声明 | 对应 HookPhase | 桥接实现 |
//! |---|---|---|
//! | hooks.PreToolUse → modlens_prepare | PreToolCall | `install_modlens_hook`：当 HookContext.args["attachments"] 包含图片路径时，调用 `vision::modlens::ModLens::metadata_fallback` 产出结构化摘要，写入 `HookContext.notes`，等效「上下文注入 OCR 结果」 |
//! | hooks.PostToolUse → usage_accumulate | PostToolCall | `install_usage_hook`：统计 tokens_in / tokens_out / duration_ms / tool_name，`Arc<Mutex<UsageAggregator>>` 累加，并通过 `publish_usage_accumulated()` 发布 `EventType::Status` + `kind="usage.accumulated"`（A 阶段内存聚合；B 阶段升级 SQLite） |
//! | hooks.Notification → pet_switch_state | 事件订阅 | `DshPetBridge::subscribe(bus)` 订阅事件流中的 `agent.*` / `plan.*` 状态路由，自动切换 PetState（T-1.4 内存状态机；B 阶段替换 workbench/pet.rs 原生实现） |
//!
//! ## T-1.5 Router 3 模式
//!
//! 新增 `RouterProfile` 枚举：Plan / Execute / Weak。
//! - 从 session 持久化（`RouterProfileStore`）读写；
//! - `dsh.router.set_mode` / `dsh.router.query` 作为 PluginManifest 能力暴露；
//! - `capability_models` 被 FleetRouter::from_manifest 消费（已有接口，无需新增）。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::agent::multi_strategy::FleetRouter;
use crate::cli::dslash::{RouterCmd, RouterMode};
use crate::events::{bus::DEFAULT_HISTORY_CAPACITY, Envelope, EventBus, EventType};
use crate::harness::hooks::{HookDecision, HookPhase, HookRunner};
use crate::plugins::manifest::PluginManifest;

// -------------------- T-1.4.1 Modlens PreToolUse Hook --------------------

/// 安装 Modlens 识图钩子（PreToolCall）。
///
/// 约定：当 HookContext.args 里存在键 `image_path` 或 `attachments` 时，
/// 走"识图→注入 notes"路径；否则直接放行。
/// 注入的 notes 文本可被上层 agent loop 在拼装 prompt 时拼入 context。
pub fn install_modlens_hook(runner: &HookRunner) {
    runner.register_fn(
        HookPhase::PreToolCall,
        "dsh-modlens-prepare",
        |ctx| {
            // 检查是否有待识图附件
            let path_candidate = ctx
                .args
                .get("image_path")
                .cloned()
                .or_else(|| ctx.args.get("attachments").cloned())
                .filter(|s| !s.is_empty());
            let Some(path) = path_candidate else {
                return Ok(HookDecision::Continue);
            };
            // A 阶段：降级到 metadata_fallback（纯本地，不联网）。
            // B 阶段可替换为 ModLens::analyze（需要真实模型后端）。
            let bytes = std::fs::read(&path).unwrap_or_default();
            use crate::vision::modlens::ModLens;
            let doc = ModLens::metadata_fallback(&bytes);
            let summary = format!(
                "[modlens-ocr] image={} summary={} blocks={}",
                path,
                doc.summary,
                doc.blocks.len()
            );
            // 注入 notes：等效「上下文注入识别结果」
            let mut next = ctx.clone();
            next.notes.push(summary.clone());
            // 把识别结果作为 arg 回写，供工具侧消费
            next.args.insert("ocr_summary".into(), summary);
            next.args
                .insert("ocr_layout_json".into(), doc.to_json());
            Ok(HookDecision::Modify(next))
        },
    );
}

// -------------------- T-1.4.2 Usage PostToolUse 聚合器 --------------------

/// Usage 单次记录（A 阶段内存，B 阶段将落 SQLite）
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageRecord {
    pub tool_name: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub duration_ms: u64,
    pub ok: bool,
    pub model: String,
    pub ts_ms: i64,
}

/// 内存聚合器
#[derive(Debug, Default)]
pub struct UsageAggregator {
    pub records: Vec<UsageRecord>,
    /// day_key(YYYY-MM-DD) → (tokens_in, tokens_out, turns)
    pub day_stats: BTreeMap<String, (u64, u64, u64)>,
}

impl UsageAggregator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, r: UsageRecord) {
        let day = day_key_ms(r.ts_ms);
        let entry = self.day_stats.entry(day).or_insert((0, 0, 0));
        entry.0 += r.tokens_in;
        entry.1 += r.tokens_out;
        entry.2 += 1;
        self.records.push(r);
    }

    pub fn total_tokens(&self) -> u64 {
        self.records
            .iter()
            .map(|r| r.tokens_in + r.tokens_out)
            .sum()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

fn day_key_ms(ts_ms: i64) -> String {
    // 本地时区换算简化（以 UTC 日键，避免额外 chrono 依赖）
    let secs = (ts_ms / 1000) as i64;
    let days = secs.div_euclid(86400);
    format!("day_{days}")
}

/// 发布 usage.accumulated 事件（EventType::Status + kind）。
fn publish_usage_accumulated(
    bus: Option<&EventBus>,
    session: &str,
    rec: &UsageRecord,
    agg_total: u64,
) {
    let Some(bus) = bus else { return };
    let payload = json!({
        "kind": "usage.accumulated",
        "usage": {
            "tool_name": rec.tool_name,
            "tokens_in": rec.tokens_in,
            "tokens_out": rec.tokens_out,
            "duration_ms": rec.duration_ms,
            "ok": rec.ok,
            "model": rec.model,
            "ts_ms": rec.ts_ms,
            "session_total_tokens": agg_total,
        }
    });
    bus.publish(Envelope::new(EventType::Status, payload, session));
}

/// 安装 Usage 记账钩子（PostToolCall）。
///
/// 参数来源约定：
/// - `tokens_in`  ← args["tokens_in"] 或 fallback 0
/// - `tokens_out` ← args["tokens_out"] 或 fallback 0
/// - `duration_ms` ← args["duration_ms"] 或 fallback 0
/// - `model`     ← args["model"] 或 fallback "unknown"
/// - `ok`        ← 钩子只在 PostToolCall 运行，默认 true（OnError 路径单独处理）
///
/// 返回 Arc<Mutex<UsageAggregator>> 供测试/渲染读取。
pub fn install_usage_hook(
    runner: &HookRunner,
    bus: Option<Arc<EventBus>>,
    session: impl Into<String>,
) -> Arc<Mutex<UsageAggregator>> {
    install_usage_hook_with_projection(runner, bus, session, None)
}

/// 安装 Usage 记账钩子，并把每条 usage 样本**折叠进会话 token 用量投影**
/// （DSH 融合 v5：`TokenUsageUnit` 同一步替换、跨步累加；上游 token-meter
/// tokenUsage 单元语义）。`projections = None` 时等价 [`install_usage_hook`]。
pub fn install_usage_hook_with_projection(
    runner: &HookRunner,
    bus: Option<Arc<EventBus>>,
    session: impl Into<String>,
    projections: Option<Arc<Mutex<crate::projection::ProjectionStore>>>,
) -> Arc<Mutex<UsageAggregator>> {
    let agg = Arc::new(Mutex::new(UsageAggregator::new()));
    let session_post = session.into();
    let session_err = session_post.clone();
    let session_proj = session_post.clone();
    let agg_for_hook = Arc::clone(&agg);
    let bus_for_hook = bus.clone();
    let proj_for_hook = projections.clone();

    runner.register_fn(
        HookPhase::PostToolCall,
        "dsh-usage-accumulate",
        move |ctx| {
            let parse = |k: &str| -> u64 {
                ctx.args
                    .get(k)
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
            };
            let rec = UsageRecord {
                tool_name: ctx.tool_name.clone(),
                tokens_in: parse("tokens_in"),
                tokens_out: parse("tokens_out"),
                duration_ms: parse("duration_ms"),
                ok: true,
                model: ctx
                    .args
                    .get("model")
                    .cloned()
                    .unwrap_or_else(|| "unknown".into()),
                ts_ms: chrono::Utc::now().timestamp_millis(),
            };
            let mut guard = agg_for_hook.lock().unwrap_or_else(|e| e.into_inner());
            guard.push(rec.clone());
            let total = guard.total_tokens();
            drop(guard);
            publish_usage_accumulated(
                bus_for_hook.as_deref(),
                session_post.as_str(),
                &rec,
                total,
            );
            // 凭证/投影缝：把样本折叠进会话 tokenUsage 投影（best-effort，失败不阻断记账）
            if let Some(proj) = proj_for_hook.as_ref() {
                let event = json!({
                    "kind": "usage.accumulated",
                    "usage": {
                        "tool_name": rec.tool_name,
                        "tokens_in": rec.tokens_in,
                        "tokens_out": rec.tokens_out,
                        "ts_ms": rec.ts_ms,
                    }
                });
                if let Ok(mut store) = proj.lock() {
                    let _ = store.project(
                        &crate::projection::TokenUsageUnit,
                        &session_proj,
                        &[event],
                        0,
                    );
                }
            }
            Ok(HookDecision::Continue)
        },
    );

    // OnError 也记一笔：错误型工具调用（ok=false）
    let agg_for_err = Arc::clone(&agg);
    let bus_for_err = bus;
    runner.register_fn(
        HookPhase::OnError,
        "dsh-usage-accumulate-error",
        move |ctx| {
            let rec = UsageRecord {
                tool_name: ctx.tool_name.clone(),
                tokens_in: 0,
                tokens_out: 0,
                duration_ms: 0,
                ok: false,
                model: ctx
                    .args
                    .get("model")
                    .cloned()
                    .unwrap_or_else(|| "unknown".into()),
                ts_ms: chrono::Utc::now().timestamp_millis(),
            };
            let mut guard = agg_for_err.lock().unwrap_or_else(|e| e.into_inner());
            guard.push(rec.clone());
            let total = guard.total_tokens();
            drop(guard);
            publish_usage_accumulated(
                bus_for_err.as_deref(),
                session_err.as_str(),
                &rec,
                total,
            );
            Ok(HookDecision::Continue)
        },
    );
    agg
}

// -------------------- T-1.4.3 Pet Notification 状态机 --------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetState {
    Idle,
    Thinking,
    Running,
    Sleeping,
    Eating,
    Petted,
}

impl Default for PetState {
    fn default() -> Self {
        PetState::Idle
    }
}

impl PetState {
    pub fn as_str(&self) -> &'static str {
        match self {
            PetState::Idle => "idle",
            PetState::Thinking => "thinking",
            PetState::Running => "running",
            PetState::Sleeping => "sleeping",
            PetState::Eating => "eating",
            PetState::Petted => "petted",
        }
    }

    /// 事件路由键匹配：哪些 Status payload kind 触发什么状态
    fn from_event_type_and_payload(
        event: EventType,
        kind: Option<&str>,
    ) -> Option<PetState> {
        match (event, kind) {
            // 工具回路开始 / 计划开始 → running
            (EventType::StepStart, _) | (EventType::PlanRun, _) => Some(PetState::Running),
            // 模型思考帧 → thinking
            (EventType::Thinking, _) => Some(PetState::Thinking),
            // 工具结果 / 完成 / 计划修订结束 → idle
            (EventType::Done, _) | (EventType::StepFinish, _) => Some(PetState::Idle),
            // 显式 payload kind（slash.pet 触发等）
            (EventType::Status, Some("pet.pet")) => Some(PetState::Petted),
            (EventType::Status, Some("pet.feed")) => Some(PetState::Eating),
            _ => None,
        }
    }
}

/// DSH Pet 桥（A 阶段共享状态 + 事件订阅）。
pub struct DshPetBridge {
    pub state: Arc<Mutex<PetState>>,
}

impl DshPetBridge {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(PetState::Idle)),
        }
    }

    /// 手动切换状态（例如 `/pet` slash 命令触发）
    pub fn set_state(&self, next: PetState) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = next;
    }

    pub fn current(&self) -> PetState {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 处理一帧 Envelope：按映射表切换状态；不匹配则保持。
    pub fn handle_envelope(&self, env: &Envelope) {
        let kind = env.payload.get("kind").and_then(|v| v.as_str());
        if let Some(next) = PetState::from_event_type_and_payload(env.r#type, kind) {
            self.set_state(next);
        }
    }

    /// 向 EventBus 注册订阅：`**`（所有事件），后台驱动状态机。
    ///
    /// 返回的 mpsc::Receiver 应被上层驱动消费（tokio::spawn loop）。
    /// 对于单测或同步场景，可手动 `handle_envelope` 模拟。
    pub fn subscribe(self: &Arc<Self>, bus: &EventBus) -> tokio::sync::mpsc::Receiver<Envelope> {
        let rx = bus.subscribe("**");
        let weak_self = Arc::clone(self);
        // 此处只返回 receiver；不自动 spawn，以避免在无 runtime 环境下出错。
        // 调用方（CLI program / TUI app）负责 spawn：
        //     tokio::spawn(async move { while let Some(e) = rx.recv().await { weak_self.handle_envelope(&e); } });
        let _ = weak_self; // 预留：spawn 时闭包捕获
        rx
    }
}

impl Default for DshPetBridge {
    fn default() -> Self {
        Self::new()
    }
}

// -------------------- T-1.5 Router 3 模式 Profile --------------------

/// Router 3 模式 profile（T-1.5 主数据结构）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterProfile {
    Plan,
    Execute,
    Weak,
}

impl Default for RouterProfile {
    fn default() -> Self {
        RouterProfile::Weak
    }
}

impl RouterProfile {
    pub fn from_str(s: &str) -> Result<Self> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "plan" | "planner" | "planning" => RouterProfile::Plan,
            "execute" | "exec" | "run" | "executor" => RouterProfile::Execute,
            "weak" | "auto" | "default" | "balanced" => RouterProfile::Weak,
            other => return Err(anyhow!("未知 Router profile: {other}")),
        })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RouterProfile::Plan => "plan",
            RouterProfile::Execute => "execute",
            RouterProfile::Weak => "weak",
        }
    }

    /// 对应的 FleetRouter 级 capability 路由（用于 FleetRouter::from_manifest 消费）
    pub fn default_model_hint(&self) -> &'static str {
        match self {
            RouterProfile::Plan => "opus",
            RouterProfile::Execute => "sonnet",
            RouterProfile::Weak => "inherit",
        }
    }
}

impl From<RouterMode> for RouterProfile {
    fn from(m: RouterMode) -> Self {
        match m {
            RouterMode::Plan => RouterProfile::Plan,
            RouterMode::Execute => RouterProfile::Execute,
            RouterMode::Weak => RouterProfile::Weak,
        }
    }
}

/// Router profile 存储（A 阶段 JSON 文件；与 Marguerite persistence 目录保持同级）
pub struct RouterProfileStore {
    path: std::path::PathBuf,
}

impl RouterProfileStore {
    /// 指向某 session 对应的 router-profile.json。path 不存在时默认 Weak。
    pub fn new(session_path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: session_path.into(),
        }
    }

    pub fn load(&self) -> RouterProfile {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|t| serde_json::from_str::<RouterProfile>(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, profile: RouterProfile) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
        let text = serde_json::to_string_pretty(&profile)?;
        std::fs::write(&self.path, text)?;
        Ok(())
    }
}

/// DSH Router 桥（T-1.5）：store + fleet router 导出 + 命令解析适配
pub struct DshRouterBridge {
    pub store: RouterProfileStore,
}

impl DshRouterBridge {
    pub fn new(store: RouterProfileStore) -> Self {
        Self { store }
    }

    /// 应用一次 `/router` 命令，返回响应文本（可直接交给用户）。
    pub fn apply_cmd(&mut self, cmd: &RouterCmd) -> Result<String> {
        match cmd {
            RouterCmd::Set { mode } => {
                let p: RouterProfile = (*mode).into();
                self.store.save(p)?;
                Ok(format!("router → {}", p.as_str()))
            }
            RouterCmd::Query => {
                let p = self.store.load();
                Ok(format!("router@current = {}", p.as_str()))
            }
            RouterCmd::Auto { task } => {
                // A 阶段启发式自动路由
                let t = task.to_ascii_lowercase();
                let profile = if t.contains("设计")
                    || t.contains("规划")
                    || t.contains("方案")
                    || t.contains("plan")
                    || t.contains("design")
                {
                    RouterProfile::Plan
                } else if t.contains("执行")
                    || t.contains("实现")
                    || t.contains("修复")
                    || t.contains("编码")
                    || t.contains("implement")
                    || t.contains("fix")
                {
                    RouterProfile::Execute
                } else {
                    RouterProfile::Weak
                };
                self.store.save(profile)?;
                Ok(format!("router auto → {}", profile.as_str()))
            }
        }
    }

    /// 当前 profile
    pub fn current(&self) -> RouterProfile {
        self.store.load()
    }

    /// 根据 manifest 构建 FleetRouter（已存在 FleetRouter::from_manifest；这里仅薄封装，合并多个）
    pub fn fleet_from_manifests(manifests: &[PluginManifest]) -> FleetRouter {
        let mut routes: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for m in manifests {
            for (k, v) in &m.capability_models {
                routes.insert(k.clone(), v.clone());
            }
        }
        let mut f = FleetRouter::new();
        for (k, v) in routes {
            f.add(&k, &v);
        }
        f
    }
}

// -------------------- 便捷入口：一次性安装 DSH bridge --------------------

/// 一次性安装：Modlens + Usage Hook 到同一个 HookRunner；
/// 返回 (UsageAggregator, DshPetBridge, DshRouterBridge) 三件套供上层持有。
///
/// 用法：
/// ```ignore
/// let runner = HookRunner::new();
/// let bus = Arc::new(EventBus::new(DEFAULT_HISTORY_CAPACITY));
/// let (usage, pet, router) = install_dsh_bridges(
///     &runner,
///     Some(Arc::clone(&bus)),
///     "ws.s1",
///     RouterProfileStore::new("data/ws.s1/router-profile.json"),
/// );
/// ```
pub fn install_dsh_bridges(
    runner: &HookRunner,
    bus: Option<Arc<EventBus>>,
    session: impl Into<String>,
    store: RouterProfileStore,
) -> (
    Arc<Mutex<UsageAggregator>>,
    Arc<DshPetBridge>,
    DshRouterBridge,
) {
    install_dsh_bridges_with_projection(runner, bus, session, store, None)
}

/// 一次性安装：Modlens + Usage Hook（含会话 token 用量投影折叠）到同一个
/// HookRunner；返回三件套 + 投影库句柄。
pub fn install_dsh_bridges_with_projection(
    runner: &HookRunner,
    bus: Option<Arc<EventBus>>,
    session: impl Into<String>,
    store: RouterProfileStore,
    projections: Option<Arc<Mutex<crate::projection::ProjectionStore>>>,
) -> (
    Arc<Mutex<UsageAggregator>>,
    Arc<DshPetBridge>,
    DshRouterBridge,
) {
    install_modlens_hook(runner);
    let agg = install_usage_hook_with_projection(runner, bus, session, projections);
    let pet = Arc::new(DshPetBridge::new());
    let router = DshRouterBridge::new(store);
    (agg, pet, router)
}

/// 便捷地创建一个带默认容量的 EventBus（给单测和快速启动）
pub fn default_bus() -> EventBus {
    EventBus::new(DEFAULT_HISTORY_CAPACITY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::hooks::{HookContext, HookOutcome};
    use crate::events::bus::DEFAULT_HISTORY_CAPACITY;

    // ---- T-1.4 TR-R1: Modlens PreToolUse 注入 OCR notes ----
    #[test]
    fn hook_modlens_pretooluse_inject_ocr() {
        let runner = HookRunner::new();
        install_modlens_hook(&runner);
        // 用 workspace (存在即可) + 一个虚假 image_path；
        // 因为 metadata_fallback 不联网，对不存在的文件会返回空文档，但 notes 条目应被写入。
        let tmp = temp_dir_marker();
        let ctx = HookContext::new(HookPhase::PreToolCall, &tmp)
            .with_tool("read_file")
            .with_arg("image_path", &tmp.join("fake.png").display().to_string());
        match runner.run_phase(ctx).unwrap() {
            HookOutcome::Proceed(c) => {
                let ocr = c.notes.iter().find(|n| n.starts_with("[modlens-ocr]"));
                assert!(ocr.is_some(), "notes 里应有 OCR 摘要：{:?}", c.notes);
                assert!(
                    c.args.get("ocr_summary").is_some(),
                    "args 应注入 ocr_summary"
                );
                assert!(
                    c.args.get("ocr_layout_json").is_some(),
                    "args 应注入 ocr_layout_json"
                );
            }
            HookOutcome::Aborted { by, reason } => {
                panic!("被意外阻断 by={by} reason={reason}")
            }
        }
    }

    // ---- T-1.4 TR-R2: Usage PostToolCall 聚合 + 事件 ----
    #[test]
    fn hook_usage_posttooluse_accumulate() {
        let runner = HookRunner::new();
        let bus = Arc::new(EventBus::new(DEFAULT_HISTORY_CAPACITY));
        let agg = install_usage_hook(&runner, Some(Arc::clone(&bus)), "t-1-4-2");
        // run_tool_call 会走 PreToolCall + exec + PostToolCall
        let ctx = HookContext::new(HookPhase::PostToolCall, temp_dir_marker())
            .with_tool("bash")
            .with_arg("tokens_in", "100")
            .with_arg("tokens_out", "200")
            .with_arg("duration_ms", "17")
            .with_arg("model", "deepseek-chat");
        let out = runner
            .run_tool_call(ctx, |approved| Ok(format!("run: {}", approved.tool_name)))
            .unwrap();
        assert!(out.contains("run: bash"));

        // 再跑一笔
        let ctx2 = HookContext::new(HookPhase::PostToolCall, temp_dir_marker())
            .with_tool("grep")
            .with_arg("tokens_in", "50")
            .with_arg("tokens_out", "25")
            .with_arg("model", "deepseek-chat");
        let _ = runner
            .run_tool_call(ctx2, |_| Ok("grep-ok".to_string()))
            .unwrap();

        let guard = agg.lock().unwrap();
        assert_eq!(guard.len(), 2, "应有 2 条记录");
        // 100+200+50+25 = 375
        assert_eq!(guard.total_tokens(), 375);

        // 总线应有 2 条 usage.accumulated
        use std::time::Instant;
        let evs = bus.history(Instant::now() - std::time::Duration::from_secs(10));
        let usage_events: Vec<_> = evs
            .iter()
            .filter(|e| e.payload["kind"] == "usage.accumulated")
            .collect();
        assert_eq!(usage_events.len(), 2, "总线应有 2 条 usage 事件");
    }

    // ---- v5: usage 钩子 → 会话 token 用量投影折叠（上游 tokenUsage 语义） ----
    #[test]
    fn hook_usage_folds_into_session_projection() {
        let runner = HookRunner::new();
        let projections = Arc::new(Mutex::new(
            crate::projection::ProjectionStore::open(None).unwrap(),
        ));
        let agg = install_usage_hook_with_projection(&runner, None, "s1", Some(projections.clone()));

        let ctx = HookContext::new(HookPhase::PostToolCall, temp_dir_marker())
            .with_tool("run_code")
            .with_arg("tokens_in", "100")
            .with_arg("tokens_out", "50");
        let _ = runner.run_tool_call(ctx, |_| Ok("ok1".to_string())).unwrap();
        // 新一步（工具不同 → step_key 不同）
        let ctx2 = HookContext::new(HookPhase::PostToolCall, temp_dir_marker())
            .with_tool("web_search")
            .with_arg("tokens_in", "30")
            .with_arg("tokens_out", "10");
        let _ = runner.run_tool_call(ctx2, |_| Ok("ok2".to_string())).unwrap();

        let store = projections.lock().unwrap();
        let view = store
            .view_of(&crate::projection::TokenUsageUnit, "s1")
            .expect("应有 tokenUsage 视图");
        assert_eq!(view["tokens_in"], 130, "跨步累加");
        assert_eq!(view["tokens_out"], 60);
        assert_eq!(view["steps"], 2);
        assert_eq!(agg.lock().unwrap().len(), 2, "聚合器仍逐条记录");
    }

    // ---- T-1.4 TR-R3: Notification → Pet 状态切换 ----
    #[test]
    fn hook_notification_pet_state_transition() {
        let pet = DshPetBridge::new();
        // running
        pet.handle_envelope(&Envelope::new(
            EventType::StepStart,
            json!({"step": 1}),
            "s",
        ));
        assert_eq!(pet.current(), PetState::Running);

        // thinking
        pet.handle_envelope(&Envelope::new(
            EventType::Thinking,
            json!({"text": "."}),
            "s",
        ));
        assert_eq!(pet.current(), PetState::Thinking);

        // done → idle
        pet.handle_envelope(&Envelope::new(EventType::Done, json!({}), "s"));
        assert_eq!(pet.current(), PetState::Idle);

        // slash pet → petted
        pet.handle_envelope(&Envelope::new(
            EventType::Status,
            json!({"kind": "pet.pet"}),
            "s",
        ));
        assert_eq!(pet.current(), PetState::Petted);

        // slash feed → eating
        pet.handle_envelope(&Envelope::new(
            EventType::Status,
            json!({"kind": "pet.feed"}),
            "s",
        ));
        assert_eq!(pet.current(), PetState::Eating);
    }

    // ---- T-1.4 TR-R4: 确认 3 类 hooks 全部通过 registry 注册 ----
    #[test]
    fn all_three_hook_phases_registered() {
        let runner = HookRunner::new();
        let _ = install_usage_hook(&runner, None, "s1");
        install_modlens_hook(&runner);
        // PreToolCall: dsh-modlens-prepare
        assert!(runner
            .hook_names(HookPhase::PreToolCall)
            .iter()
            .any(|n| n == "dsh-modlens-prepare"));
        // PostToolCall: dsh-usage-accumulate
        assert!(runner
            .hook_names(HookPhase::PostToolCall)
            .iter()
            .any(|n| n == "dsh-usage-accumulate"));
        // OnError: dsh-usage-accumulate-error
        assert!(runner
            .hook_names(HookPhase::OnError)
            .iter()
            .any(|n| n == "dsh-usage-accumulate-error"));
    }

    // ---- T-1.5 TR-R1: RouterProfile from_str / roundtrip ----
    #[test]
    fn router_3_profiles_enum() {
        assert_eq!(RouterProfile::from_str("plan").unwrap(), RouterProfile::Plan);
        assert_eq!(
            RouterProfile::from_str("execute").unwrap(),
            RouterProfile::Execute
        );
        assert_eq!(RouterProfile::from_str("weak").unwrap(), RouterProfile::Weak);
        assert_eq!(
            RouterProfile::from_str("auto").unwrap(),
            RouterProfile::Weak
        );
        // 别名
        assert_eq!(
            RouterProfile::from_str("planner").unwrap(),
            RouterProfile::Plan
        );
        assert_eq!(
            RouterProfile::from_str("exec").unwrap(),
            RouterProfile::Execute
        );
        // 非法
        assert!(RouterProfile::from_str("ultra").is_err());
        // 缺省
        assert_eq!(RouterProfile::default(), RouterProfile::Weak);
    }

    // ---- T-1.5 TR-R2: set command persists & reloads ----
    #[test]
    fn router_set_command_persists() {
        let dir = temp_dir_marker();
        let path = dir.join("router-profile.json");
        let store = RouterProfileStore::new(&path);
        let mut bridge = DshRouterBridge::new(store);

        bridge.apply_cmd(&RouterCmd::Set {
            mode: RouterMode::Plan,
        }).unwrap();
        // 重启：new 一个 store 读同一路径
        let store2 = RouterProfileStore::new(&path);
        let bridge2 = DshRouterBridge::new(store2);
        assert_eq!(bridge2.current(), RouterProfile::Plan);
    }

    // ---- T-1.5 TR-R3: PluginRegistry invoke query 语义（store 查询） ----
    #[test]
    fn router_capabilities_query_ok() {
        let dir = temp_dir_marker();
        let store = RouterProfileStore::new(dir.join("r.json"));
        let mut bridge = DshRouterBridge::new(store);
        bridge.apply_cmd(&RouterCmd::Set {
            mode: RouterMode::Execute,
        }).unwrap();
        let r = bridge.apply_cmd(&RouterCmd::Query).unwrap();
        assert!(r.contains("execute"), "query 应答应含模式名: {r}");
    }

    // ---- T-1.5 TR-R4: FleetRouter from_manifest capability_models ----
    #[test]
    fn fleet_router_consumes_capability_models() {
        let manifest_json = r#"{
            "name": "dsh-router-demo",
            "version": "0.1.0",
            "description": "测试 manifest",
            "capabilities": ["a.run", "b.review"],
            "capability_models": {"b.review": "opus", "a.run": "sonnet"}
        }"#;
        let m = PluginManifest::from_json(manifest_json).unwrap();
        let f = FleetRouter::from_manifest(&m);
        assert_eq!(f.resolve("b.review"), Some("opus"));
        assert_eq!(f.resolve("a.run"), Some("sonnet"));
        assert_eq!(f.resolve("nonexistent"), None);
    }

    // 辅助：创建一个临时目录（不写文件），路径唯一即可
    fn temp_dir_marker() -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!(
            "mgr-dsh-t-{}-{}",
            std::process::id(),
            rand_like()
        ));
        std::fs::create_dir_all(&base).ok();
        base
    }
    fn rand_like() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }
}
