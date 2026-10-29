//! P-10 真流式 agent 回路 + 输出去冗余（蒸馏 opencode `session/processor.ts`）
//!
//! ## 为什么需要它
//!
//! `runtime::run_tool_loop` 用 `chat_with_tools`（非流式）逐轮**攒齐再吐**：
//! 用户在整轮结束前看不到任何字，工具调用/结果也无从观测。这正是
//! `external/AUDIT_REPORT.md` 标记的 P1 缺口「agent loop 未真正 yield chunk」。
//!
//! ## 蒸馏来源（真读了本机源码）
//!
//! `external/opencode/packages/opencode/src/session/processor.ts`：
//! - **事件分类**（`start-step` / `text-delta` / `tool-call` / `tool-result` /
//!   `tool-error` / `finish-step` / `error`）—— agent 回路吐出的是**带类型的事件流**，
//!   不是一坨最终文本。本模块的 [`AgentEvent`] 是它的极简 Rust 对译。
//! - **doom-loop 检测**（`DOOM_LOOP_THRESHOLD = 3`，processor.ts:24/306-330）——
//!   若最近 N 次工具调用「工具名 + 入参」完全一致，说明模型卡在死循环，
//!   opencode 在此处触发 `permission.ask`；本模块极简为**如实上报并中止**。
//!
//! 未搬运 opencode 的 snapshot/patch、part 持久化、permission 交互、summary
//! 分叉等（依赖其 Effect 运行时与 SQLite part 表，超出极简范围）。
//!
//! ## 设计要点
//!
//! - **DI 接缝**：回路不自己建 HTTP 连接，而是接受一个「给定消息 → 产出
//!   [`StreamReceiver`]」的闭包。因此**无需网络即可单测**真流式行为。
//! - **诚实**：`TextDelta` 是**收到即转发**，不做任何缓冲攒批；事件顺序即真实时序。

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use chrono::Utc;
use tokio::sync::mpsc;

use crate::agent::runtime::parse_tool_args;
use crate::agent::tool_cycle::{ToolCall as CycleToolCall, ToolCycle, ToolResult};
use crate::agent::tool_schema::{discovery_query, ToolDisclosure, DISCOVER_TOOLS_NAME};
use crate::mcp::registry::ToolRegistry;
use crate::model::llm::{LLMChunk, ToolCall as LlmToolCall, ToolDef};
use crate::model::StreamReceiver;
use crate::types::Message;

/// opencode `DOOM_LOOP_THRESHOLD`（processor.ts:24）
///
/// **纠正阈值**：末尾连续这么多次「同名同参」调用 → 判定模型在重复自己，
/// **不中止整轮**，而是在工具结果里注入纠正提示（见 [`DOOM_LOOP_ABORT`]）。
pub const DOOM_LOOP_THRESHOLD: usize = 3;

/// doom-loop **硬中止阈值**：注入纠正提示后仍以完全相同参数重复到此数 → 判定卡死，
/// 如实上报并中止回路。
///
/// 历史缺陷（v0.2.10 修复）：此前只有一档阈值，命中即 `return Err` 炸掉整轮并触发
/// 「降级为纯文本推理」——在单工具锚定面下（模型手上只有 `str_replace_editor`），
/// 「同一编辑工具连续调用 3 次」是**必然发生**的正常轨迹，于是正常任务被误杀。
pub const DOOM_LOOP_ABORT: usize = 5;

/// 退化重复默认阈值：连续同一行重复达到此数即判定模型卡死并停止外发
pub const DEFAULT_REPEAT_THRESHOLD: usize = 3;

/// agent 回路事件（蒸馏 opencode processor.ts 的 stream part 分类）
///
/// 事件按**真实发生顺序**送出：`StepStart` → (`TextDelta`|`Thinking`)* →
/// `StepFinish` → (`ToolCall` → `ToolResult`)* → 下一轮 … → `Done`。
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    /// 本轮开始（对应 `start-step`），`round` 自 1 计
    StepStart { round: u32 },
    /// 增量正文（对应 `text-delta`）——**收到即转发**，无缓冲
    TextDelta { text: String },
    /// 增量思考链（与正文分离，避免污染输出）
    Thinking { text: String },
    /// 本轮结束（对应 `finish-step`）
    StepFinish { round: u32, reason: Option<String> },
    /// 模型请求调用工具（对应 `tool-call`）；`id` 为模型给出的调用 id（无则 `None`）
    ToolCall {
        id: Option<String>,
        name: String,
        arguments: String,
        /// 引擎真实审批裁决（`approved` / `pending` / `denied`）。
        /// 仅对「直接驱动本机」的高自主工具（computer_use …）有值，其余为 `None`
        /// （不伪造字段）。
        approval: Option<String>,
    },
    /// 工具执行结果（对应 `tool-result` / `tool-error`）；`ok=false` 表示错误或超时。
    /// `id` 与对应 [`AgentEvent::ToolCall`] 的 id 一致 —— 供前端**精确配对**，
    /// 不再靠「工具名 + 时钟」猜身份（那会让同毫秒的同名调用互相吞并 / 结果另起一条）。
    ToolResult {
        id: Option<String>,
        name: String,
        output: String,
        ok: bool,
        /// 与 `ToolCall` 同源的审批裁决（approved / pending / denied）。
        approval: Option<String>,
    },
    /// 检测到 doom-loop：同一工具 + 同一入参连续重复（对应 processor.ts:306-330）
    DoomLoop { tool: String, repeats: usize },
    /// 正文因退化重复被抑制（输出去冗余，如实上报而非静默截断）
    OutputSuppressed { repeated_line: String },
    /// 回路收敛，给出最终文本
    Done { text: String },
    /// 出错（对应 `error`）
    ///
    /// R65/B1：`code` 为**稳定错误码**（来自 `model::error_taxonomy::code_of`，如
    /// `AUTH_FAILED`）；前端按码本地化，而不是回显 `message`（后者可能是上游英文正文
    /// 或内部术语）。无法归类时为 `None`，前端回退到服务端文案。
    Error {
        message: String,
        code: Option<String>,
    },
    /// P2-9：单 turn 预算（步数/时长/token）耗尽，硬熔断。携带结构化
    /// `budget:{kind,limit,used,remaining}`，供前端显式提示而非静默截断。
    BudgetExhausted {
        kind: String,
        limit: u64,
        used: u64,
        remaining: u64,
        partial_text: String,
    },
}

/// 流式聚合器：边收边分离思考链、边检测退化重复（蓝图 §2.3 输出去冗余）
///
/// **诚实边界**：只抑制「**连续整行完全相同**」达到阈值的退化输出 —— 这是真实存在的
/// 模型卡死失效模式，判定确定性、可单测。**不做**任何语义去重（那会误删合法重复内容）。
#[derive(Debug, Clone)]
pub struct StreamAggregator {
    content: String,
    thinking: String,
    /// 尚未遇到换行的行尾残片
    pending_line: String,
    /// 最近一条完整行
    last_line: String,
    /// 最近一条完整行连续重复次数（含首次出现，故初值 0）
    repeat_run: usize,
    repeat_threshold: usize,
    suppressed: bool,
    /// 触发抑制的那一行（供上报）
    suppressed_line: Option<String>,
}

impl StreamAggregator {
    pub fn new(repeat_threshold: usize) -> Self {
        Self {
            content: String::new(),
            thinking: String::new(),
            pending_line: String::new(),
            last_line: String::new(),
            repeat_run: 0,
            repeat_threshold: repeat_threshold.max(2),
            suppressed: false,
            suppressed_line: None,
        }
    }

    /// 吞入一块，返回**应实际外发**的正文增量；`None` 表示无正文或已被抑制。
    ///
    /// 思考链无论如何都只进 `thinking`，绝不混入正文（输出去冗余的第一层）。
    /// 正文增量**逐字原样**返回 —— 不攒批、不改写，保证 CLI 渲染即真实时序。
    pub fn push(&mut self, chunk: &LLMChunk) -> Option<String> {
        if let Some(t) = &chunk.thinking {
            self.thinking.push_str(t);
        }
        let content = chunk.content.as_deref()?;
        if content.is_empty() {
            return None;
        }
        self.scan_lines(content);
        // 无论是否抑制都完整累积，便于事后审计「到底重复了什么」
        self.content.push_str(content);
        if self.suppressed {
            None
        } else {
            Some(content.to_string())
        }
    }

    /// 逐字符推进行缓冲，在每条**完整行**上做退化重复判定。
    ///
    /// 空行不参与判定 —— 否则段落间的连续空行会被误判为退化输出。
    fn scan_lines(&mut self, content: &str) {
        for ch in content.chars() {
            if ch != '\n' {
                self.pending_line.push(ch);
                continue;
            }
            let line = std::mem::take(&mut self.pending_line)
                .trim_end()
                .to_string();
            if line.is_empty() {
                continue;
            }
            if line == self.last_line {
                self.repeat_run += 1;
            } else {
                self.last_line = line;
                self.repeat_run = 1;
            }
            if self.repeat_run >= self.repeat_threshold && !self.suppressed {
                self.suppressed = true;
                self.suppressed_line = Some(self.last_line.clone());
            }
        }
    }

    /// 已累积的正文（含被抑制前的部分）
    pub fn content(&self) -> &str {
        &self.content
    }

    /// 已累积的思考链（与正文严格分离）
    pub fn thinking(&self) -> &str {
        &self.thinking
    }

    /// 是否已因退化重复停止外发
    pub fn is_suppressed(&self) -> bool {
        self.suppressed
    }

    /// 触发抑制的那一行
    pub fn suppressed_line(&self) -> Option<&str> {
        self.suppressed_line.as_deref()
    }
}

impl Default for StreamAggregator {
    fn default() -> Self {
        Self::new(DEFAULT_REPEAT_THRESHOLD)
    }
}

/// doom-loop 检测（蒸馏 opencode processor.ts:306-330）
///
/// `calls` 为按时序排列的 `(工具名, 入参原文)`。若**末尾连续 `threshold` 次**完全一致，
/// 返回 `Some((工具名, threshold))`，否则 `None`。不足 `threshold` 次一律 `None`
/// （opencode 同样要求 `recentParts.length === DOOM_LOOP_THRESHOLD`）。
///
/// 判定只看**逐字节一致**的 `(名, 参)` —— 入参有任何变化即视为模型仍在推进，
/// 不误伤合法的多次同名调用（如逐个文件读取）。
pub fn detect_doom_loop(calls: &[(String, String)], threshold: usize) -> Option<(String, usize)> {
    if threshold == 0 || calls.len() < threshold {
        return None;
    }
    let tail = &calls[calls.len() - threshold..];
    let first = &tail[0];
    if tail.iter().all(|c| c == first) {
        Some((first.0.clone(), threshold))
    } else {
        None
    }
}

/// 真流式工具回路：逐块转发正文，逐次上报工具调用与结果。
///
/// 与 `runtime::run_tool_loop` 语义一致（plan → act → observe，最终返回收敛文本），
/// 但**不再攒齐再吐**：每收到一个正文增量立刻经 `events` 送出。
///
/// - `history`：须已含本轮 User 消息；回路会把 `Message::Tool` 与最终 `Message::Assistant` 追加进去。
/// - `round_stream`：DI 接缝 —— 给定本轮完整消息序列，产出该轮的 [`StreamReceiver`]。
///   生产接线传 `|msgs| client.chat_stream_with_tools(&msgs, tools)`；测试传合成流。
/// - `events`：事件汇。接收端提前关闭不会中断回路（尽力投递，忽略 send 错误）。
pub async fn run_tool_loop_streaming<F, Fut>(
    history: &mut Vec<Message>,
    registry: Arc<ToolRegistry>,
    events: mpsc::Sender<AgentEvent>,
    max_rounds: u32,
    mut round_stream: F,
) -> Result<String>
where
    F: FnMut(Vec<Message>) -> Fut,
    Fut: Future<Output = Result<StreamReceiver>>,
{
    run_tool_loop_streaming_inner(
        history,
        None,
        registry,
        None,
        events,
        max_rounds,
        move |messages, _tools| round_stream(messages),
    )
    .await
}

/// 带按需 schema 发现的流式工具回路。旧入口仍保持固定工具面的兼容契约。
pub async fn run_tool_loop_streaming_with_disclosure<F, Fut>(
    history: &mut Vec<Message>,
    registry: Arc<ToolRegistry>,
    disclosure: ToolDisclosure,
    events: mpsc::Sender<AgentEvent>,
    max_rounds: u32,
    round_stream: F,
) -> Result<String>
where
    F: FnMut(Vec<Message>, Vec<ToolDef>) -> Fut,
    Fut: Future<Output = Result<StreamReceiver>>,
{
    run_tool_loop_streaming_with_disclosure_and_session(
        history,
        registry,
        disclosure,
        events,
        max_rounds,
        None,
        round_stream,
    )
    .await
}

/// 带按需 schema 发现 + 会话标识的流式工具回路（E12）。
///
/// `session` 用于在工具输出**真正写进 `Message::Tool` 时**计入上下文令牌台账。
/// 事件桥（`gateway::chat`）随后收到同一条 `ToolResult` 时会按同一 `call_id`
/// 认领失败并跳过，故两条路径不会重复计数。
pub async fn run_tool_loop_streaming_with_disclosure_and_session<F, Fut>(
    history: &mut Vec<Message>,
    registry: Arc<ToolRegistry>,
    disclosure: ToolDisclosure,
    events: mpsc::Sender<AgentEvent>,
    max_rounds: u32,
    session: Option<String>,
    round_stream: F,
) -> Result<String>
where
    F: FnMut(Vec<Message>, Vec<ToolDef>) -> Fut,
    Fut: Future<Output = Result<StreamReceiver>>,
{
    run_tool_loop_streaming_inner(
        history,
        session,
        registry,
        Some(disclosure),
        events,
        max_rounds,
        round_stream,
    )
    .await
}

async fn run_tool_loop_streaming_inner<F, Fut>(
    history: &mut Vec<Message>,
    session: Option<String>,
    registry: Arc<ToolRegistry>,
    mut disclosure: Option<ToolDisclosure>,
    events: mpsc::Sender<AgentEvent>,
    max_rounds: u32,
    mut round_stream: F,
) -> Result<String>
where
    F: FnMut(Vec<Message>, Vec<ToolDef>) -> Fut,
    Fut: Future<Output = Result<StreamReceiver>>,
{
    let cycle = ToolCycle::new(registry).with_session(session.clone());
    // 按时序累积的 `(工具名, 入参)`，仅用于 doom-loop 判定
    let mut call_log: Vec<(String, String)> = Vec::new();
    // P2-9：单 turn 预算硬熔断。
    let budget_limits = crate::agent::turn_budget::limits_from_env();
    let turn_started = std::time::Instant::now();
    let mut tokens_used: u64 = 0;
    // W4.4：反脚本守卫的「本 turn 已纠偏能力」必须跨轮保留（第二次放行），
    // 不能每轮清空——否则模型每次重试都会再被拒一次，直到预算耗尽。
    let mut corrected_capabilities = std::collections::HashSet::new();

    for round in 1..=max_rounds {
        // 每轮开头先做预算硬熔断（round 自 1 计，已完成步数 = round-1）。
        let steps_done = round - 1;
        if let Some(breach) = crate::agent::turn_budget::check_turn_budget(
            &budget_limits,
            steps_done,
            &turn_started,
            tokens_used,
        ) {
            let partial_text = crate::agent::runtime::last_assistant_text(history);
            let _ = events
                .send(AgentEvent::BudgetExhausted {
                    kind: breach.kind_str().to_string(),
                    limit: breach.limit,
                    used: breach.used,
                    remaining: breach.remaining,
                    partial_text: partial_text.clone(),
                })
                .await;
            return Err(anyhow!("budget_exhausted: {}", breach.to_json(&partial_text, None)));
        }
        let _ = events.send(AgentEvent::StepStart { round }).await;

        let active_tools = disclosure
            .as_ref()
            .map(ToolDisclosure::active_tools)
            .unwrap_or_default();
        let mut rx = match round_stream(history.clone(), active_tools).await {
            Ok(rx) => rx,
            Err(e) => {
                let message = e.to_string();
                let code = crate::model::error_taxonomy::code_of(&e).map(str::to_string);
                let _ = events.send(AgentEvent::Error { message, code }).await;
                return Err(e);
            }
        };

        let mut agg = StreamAggregator::default();
        let mut tool_calls: Vec<LlmToolCall> = Vec::new();
        let mut finish_reason: Option<String> = None;
        let mut reported_suppression = false;

        // ---- 真流式核心：收到一块立刻转发一块，绝不攒齐再吐 ----
        while let Some(item) = rx.recv().await {
            let chunk = match item {
                Ok(c) => c,
                Err(e) => {
                    let message = e.to_string();
                    let code = crate::model::error_taxonomy::code_of(&e).map(str::to_string);
                    let _ = events.send(AgentEvent::Error { message, code }).await;
                    return Err(e);
                }
            };

            // P2-9：累计本 turn token（流末给出的 usage）。
            if let Some(u) = &chunk.usage {
                tokens_used = tokens_used.saturating_add(u.total_tokens as u64);
            }
            if let Some(t) = &chunk.thinking {
                if !t.is_empty() {
                    let _ = events.send(AgentEvent::Thinking { text: t.clone() }).await;
                }
            }

            match agg.push(&chunk) {
                Some(delta) => {
                    let _ = events.send(AgentEvent::TextDelta { text: delta }).await;
                }
                None => {
                    // 抑制只上报一次，且如实说明是哪一行触发的
                    if agg.is_suppressed() && !reported_suppression {
                        reported_suppression = true;
                        let _ = events
                            .send(AgentEvent::OutputSuppressed {
                                repeated_line: agg
                                    .suppressed_line()
                                    .unwrap_or_default()
                                    .to_string(),
                            })
                            .await;
                    }
                }
            }

            if let Some(calls) = chunk.tool_calls {
                tool_calls.extend(calls);
            }
            if let Some(r) = chunk.finish_reason {
                finish_reason = Some(r);
            }
        }

        let _ = events
            .send(AgentEvent::StepFinish {
                round,
                reason: finish_reason,
            })
            .await;

        // 无工具调用 → 回路收敛
        // P2-B 兼容层：本地权重可能把动作写在 content 里（tool_calls 恒为空），
        // 先尝试桥接成标准工具调用；桥得上就继续走工具执行，而不是当成最终答复收敛。
        if tool_calls.is_empty() {
            if let Some(bridged) = crate::model::llm::bridge_content_action(agg.content()) {
                tool_calls.push(bridged);
            }
        }
        if tool_calls.is_empty() {
            let text = agg.content().to_string();
            let thinking = if agg.thinking().is_empty() {
                None
            } else {
                Some(agg.thinking().to_string())
            };
            history.push(Message::Assistant {
                content: text.clone(),
                thinking,
                timestamp: Utc::now(),
            });
            let _ = events.send(AgentEvent::Done { text: text.clone() }).await;
            return Ok(text);
        }

        // doom-loop 检测（两档，先记账再判定）：
        // - 达「硬中止阈值」：模型确实卡死 → 如实上报并中止（不静默烧满轮数）；
        // - 达「纠正阈值」：**不中止整轮** —— 执行照旧，但在工具结果里注入纠正提示，
        //   让模型改参数/换工具。同参重复本身不等于任务失败，炸整轮才是。
        for tc in &tool_calls {
            call_log.push((tc.name.clone(), tc.arguments.clone()));
        }
        let repeated: Option<(String, usize)> = match detect_doom_loop(&call_log, DOOM_LOOP_ABORT) {
            Some((tool, repeats)) => {
                let _ = events
                    .send(AgentEvent::DoomLoop {
                        tool: tool.clone(),
                        repeats,
                    })
                    .await;
                return Err(anyhow!(
                    "检测到 doom-loop：工具 `{tool}` 连续 {repeats} 次同名同参调用，已中止回路"
                ));
            }
            None => detect_doom_loop(&call_log, DOOM_LOOP_THRESHOLD),
        };
        if let Some((tool, repeats)) = &repeated {
            let _ = events
                .send(AgentEvent::Thinking {
                    text: format!(
                        "> 检测到重复调用：工具 `{tool}` 已连续 {repeats} 次以相同参数调用；\
已注入纠正提示，不中止整轮。"
                    ),
                })
                .await;
        }

        // 真实审批裁决（与 `runtime::run_tool_loop` 同一判据）：只对「直接驱动本机」
        // 的高自主工具产出 approval，其余工具为 None。
        let approval_session = session.as_deref().unwrap_or("");
        let approvals: Vec<Option<crate::guard::permission_matrix::ToolApproval>> = tool_calls
            .iter()
            .map(|tc| {
                // 参数解析失败 → None：参数级信任条目因此不命中（fail-closed）
                let parsed = serde_json::from_str::<serde_json::Value>(&tc.arguments).ok();
                crate::guard::permission_matrix::decide_tool(
                    &tc.name,
                    approval_session,
                    parsed.as_ref(),
                )
            })
            .collect();
        for (i, tc) in tool_calls.iter().enumerate() {
            let approval = approvals
                .get(i)
                .and_then(|a| a.as_ref())
                .map(|a| a.status.to_string());
            let _ = events
                .send(AgentEvent::ToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                    approval,
                })
                .await;
        }

        // P2：发现工具只扩充下一轮可见 schema，不进入执行注册表；真实工具仍走原 ToolCycle。
        // 按模型返回顺序串行收集结果，保证每个 ToolCall 都有同序 ToolResult。
        let tool_batch_started = std::time::Instant::now();
        let mut results = Vec::with_capacity(tool_calls.len());
        for (i, tc) in tool_calls.iter().enumerate() {
            // 矩阵拒绝 / 需确认但无宿主 → 不执行（fail-closed），结果里写明真实原因。
            if let Some(a) = approvals.get(i).and_then(|a| a.as_ref()) {
                if a.status == "denied" {
                    results.push(ToolResult::Error {
                        message: format!(
                            "审批拒绝（denied）：{}",
                            a.reason.as_deref().unwrap_or("权限矩阵拒绝")
                        ),
                        code: "APPROVAL_DENIED".to_string(),
                    });
                    continue;
                }
                if a.status == "pending" && !crate::guard::gateway::agent_approval_host_present() {
                    results.push(ToolResult::Error {
                        message: format!(
                            "审批待定（pending）：{}；当前没有可用的宿主审批通道，\
                             已按 fail-closed 拒绝执行。",
                            a.reason.as_deref().unwrap_or("需要人工确认")
                        ),
                        code: "APPROVAL_PENDING".to_string(),
                    });
                    continue;
                }
            }
            let result = if disclosure
                .as_ref()
                .is_some_and(|d| d.is_discovery_tool(&tc.name))
            {
                if let Some(disclosure) = disclosure.as_mut() {
                    let args = serde_json::from_str::<serde_json::Value>(&tc.arguments)
                        .unwrap_or(serde_json::Value::Null);
                    let output = discovery_query(&args)
                        .map(|query| disclosure.discover(query))
                        .unwrap_or_else(|| "发现工具需要提供字符串参数 query。".to_string());
                    Ok(vec![ToolResult::Success {
                        data: output,
                        metadata: HashMap::new(),
                    }])
                } else {
                    cycle
                        .execute_sequential(vec![CycleToolCall::new(
                            &tc.name,
                            parse_tool_args(&tc.arguments),
                        )])
                        .await
                }
            } else if disclosure.as_ref().is_some_and(|d| !d.permits(&tc.name)) {
                Ok(vec![ToolResult::Error {
                    message: format!(
                        "工具 `{}` 尚未加载参数 schema。请先调用 `{}` 并描述所需能力，再在下一轮调用该工具。",
                        tc.name, DISCOVER_TOOLS_NAME
                    ),
                    code: "TOOL_SCHEMA_NOT_LOADED".to_string(),
                }])
            } else if let Some(blocked) = crate::agent::capability_guard::check(
                &tc.name,
                &parse_tool_args(&tc.arguments),
                &disclosure
                    .as_ref()
                    .map(|d| {
                        d.active_tools()
                            .into_iter()
                            .map(|tool| tool.name)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
                &mut corrected_capabilities,
            ) {
                Ok(vec![blocked])
            } else {
                cycle
                    .execute_sequential(vec![CycleToolCall::new(
                        &tc.name,
                        parse_tool_args(&tc.arguments),
                    )])
                    .await
            };
            match result {
                Ok(mut one) => results.append(&mut one),
                Err(e) => {
                    let message = e.to_string();
                    let code = crate::model::error_taxonomy::code_of(&e).map(str::to_string);
                    let _ = events.send(AgentEvent::Error { message, code }).await;
                    return Err(e);
                }
            }
        }
        let tool_batch_elapsed_ms = tool_batch_started.elapsed().as_millis() as u64;

        // 回传 reasoning_content（DeepSeek thinking 模式要求多轮回传）
        // 必须在 Tool 消息之前插入，messages_to_api 会将其合并到 assistant.tool_calls 消息
        let thinking = if agg.thinking().is_empty() {
            None
        } else {
            Some(agg.thinking().to_string())
        };
        if thinking.is_some() {
            history.push(Message::Assistant {
                content: String::new(),
                thinking,
                timestamp: Utc::now(),
            });
        }

        for (idx, (tc, res)) in tool_calls.iter().zip(results).enumerate() {
            let (mut output, ok) = match &res {
                ToolResult::Success { data, .. } => (data.clone(), true),
                ToolResult::Error { message, code } => {
                    (format!("[工具错误 {code}] {message}"), false)
                }
                ToolResult::Timeout { tool_name, .. } => (format!("[工具超时] {tool_name}"), false),
            };
            // 同参重复的纠正提示：注进工具结果本身，模型下一轮必然读到；
            // 只提醒、不改判定（`ok` 仍如实反映真实执行结果）。
            if let Some((tool, repeats)) = &repeated {
                if *tool == tc.name {
                    output = format!(
                        "[重复调用提醒] 你已连续 {repeats} 次以**完全相同的参数**调用 `{}`；\
相同的参数不会得到不同结果。请改变参数、换用其它工具，或直接给出结论。\n\n{output}",
                        tc.name
                    );
                }
            }
            // E12：工具输出**真正进入模型上下文**的点，就是下面这条 `Message::Tool`
            // 入历史处（`output` 即模型下一轮读到的完整文本，含重复调用提醒）。
            // 在此按完整文本记一次账：`call_id` 与事件桥 payload 的 `id` 同源，
            // 网关桥随后收到同一条 `ToolResult` 时会按该键认领失败并跳过 —— 只记一次。
            // 注意必须在 `events.send` **之前**记账：桥上后到的那条一定能看到已登记的键。
            // 入口截流（M1-T3）：**模型历史与上下文台账共用这一份**。
            // 台账口径是「真正进入模型上下文的东西」（`overhead_tokens`），按原文计
            // 会让用量表与压缩阈值一起骗人；事件流与复盘台账仍记原文（审计要全文）。
            let context_output =
                crate::context::middle_truncate::truncate_for_context(&tc.name, &output);
            if let Some(sess) = session.as_deref() {
                crate::context::session::record_tool_output_at_context(
                    sess,
                    tc.id.as_deref(),
                    &tc.name,
                    &context_output,
                );
                // 复盘证据：与网关桥/计划 tap 共用同一幂等键（call_id），
                // 谁先看到谁落盘，不会重复写行。
                crate::persistence::session::append_tool_activity(
                    sess,
                    tc.id.as_deref(),
                    &tc.name,
                    &tc.arguments,
                    &output,
                    (tool_calls.len() == 1).then_some(tool_batch_elapsed_ms),
                    Some(ok),
                );
            }
            let _ = events
                .send(AgentEvent::ToolResult {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    output: output.clone(),
                    ok,
                    approval: approvals
                        .get(idx)
                        .and_then(|a| a.as_ref())
                        .map(|a| a.status.to_string()),
                })
                .await;
            // 工具结果回灌历史，供下一轮模型观测（observe）。
            // 间接注入防线（P0）：浏览器/网页/OCR 这类外部可写内容在**进上下文**时
            // 包上不可信标记；事件与台账仍用原文（见上方 event_bus 调用）。
            history.push(Message::Tool {
                name: tc.name.clone(),
                input: tc.arguments.clone(),
                // D11：进上下文前中间截断（与 runtime / multi_strategy 同一入口）。
                output: crate::security::spotlight::wrap_if_untrusted(
                    &tc.name,
                    &context_output,
                ),
                timestamp: Utc::now(),
                // 单轮单工具 → 该批耗时即该工具耗时；多工具批无法拆分 → None（诚实）。
                elapsed_ms: (tool_calls.len() == 1).then_some(tool_batch_elapsed_ms),
            });
        }
    }

    Err(anyhow!(
        "工具回路超过最大轮数 {max_rounds}，未能收敛到最终回复"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::registry::ToolExecutor;
    use crate::model::llm::ToolCall as LlmToolCall;
    use async_trait::async_trait;

    fn text_chunk(s: &str) -> LLMChunk {
        LLMChunk {
            content: Some(s.to_string()),
            thinking: None,
            finish_reason: None,
            tool_calls: None,
            usage: None,
        }
    }

    fn thinking_chunk(s: &str) -> LLMChunk {
        LLMChunk {
            content: None,
            thinking: Some(s.to_string()),
            finish_reason: None,
            tool_calls: None,
            usage: None,
        }
    }

    fn finish_chunk(reason: &str) -> LLMChunk {
        LLMChunk {
            content: None,
            thinking: None,
            finish_reason: Some(reason.to_string()),
            tool_calls: None,
            usage: None,
        }
    }

    fn tool_chunk(name: &str, arguments: &str) -> LLMChunk {
        LLMChunk {
            content: None,
            thinking: None,
            finish_reason: Some("tool_calls".to_string()),
            tool_calls: Some(vec![LlmToolCall {
                id: None,
                name: name.to_string(),
                arguments: arguments.to_string(),
            }]),
            usage: None,
        }
    }

    /// 把若干块合成一个 StreamReceiver（无网络）
    fn synthetic_stream(chunks: Vec<LLMChunk>) -> StreamReceiver {
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            for c in chunks {
                if tx.send(Ok(c)).await.is_err() {
                    return;
                }
            }
        });
        rx
    }

    fn error_stream(msg: &str) -> StreamReceiver {
        let (tx, rx) = mpsc::channel(4);
        let msg = msg.to_string();
        tokio::spawn(async move {
            let _ = tx.send(Err(anyhow!("{msg}"))).await;
        });
        rx
    }

    struct EchoTool;

    #[async_trait]
    impl ToolExecutor for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echo for stream loop tests"
        }
        async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
            Ok(ToolResult::Success {
                data: format!("echo:{}", args.get("m").cloned().unwrap_or_default()),
                metadata: HashMap::new(),
            })
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object","properties":{"m":{"type":"string"}}})
        }
    }

    fn echo_registry() -> Arc<ToolRegistry> {
        let mut r = ToolRegistry::new();
        r.register_local(Box::new(EchoTool));
        Arc::new(r)
    }

    fn user(s: &str) -> Message {
        Message::User {
            content: s.to_string(),
            timestamp: Utc::now(),
        }
    }

    async fn drain(mut rx: mpsc::Receiver<AgentEvent>) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        while let Some(e) = rx.recv().await {
            out.push(e);
        }
        out
    }

    // ---------- StreamAggregator ----------

    #[test]
    fn aggregator_separates_thinking_from_content() {
        let mut agg = StreamAggregator::default();
        agg.push(&thinking_chunk("let me think"));
        agg.push(&text_chunk("answer"));
        assert_eq!(agg.content(), "answer");
        assert_eq!(agg.thinking(), "let me think");
    }

    #[test]
    fn aggregator_emits_content_deltas_verbatim() {
        let mut agg = StreamAggregator::default();
        assert_eq!(agg.push(&text_chunk("he")), Some("he".to_string()));
        assert_eq!(agg.push(&text_chunk("llo")), Some("llo".to_string()));
        assert_eq!(agg.content(), "hello");
    }

    #[test]
    fn aggregator_returns_none_for_thinking_only_chunk() {
        let mut agg = StreamAggregator::default();
        assert_eq!(agg.push(&thinking_chunk("hmm")), None);
    }

    #[test]
    fn aggregator_suppresses_degenerate_repetition() {
        let mut agg = StreamAggregator::new(3);
        // 同一行连续 3 次 → 判定退化重复
        agg.push(&text_chunk("loop\n"));
        agg.push(&text_chunk("loop\n"));
        assert!(!agg.is_suppressed(), "两次重复还不该抑制");
        agg.push(&text_chunk("loop\n"));
        assert!(agg.is_suppressed(), "第三次同行应触发抑制");
        assert_eq!(agg.suppressed_line(), Some("loop"));
        // 抑制后不再外发
        assert_eq!(agg.push(&text_chunk("loop\n")), None);
    }

    #[test]
    fn aggregator_does_not_suppress_distinct_lines() {
        let mut agg = StreamAggregator::new(3);
        for s in ["a\n", "b\n", "c\n", "d\n", "e\n"] {
            assert!(agg.push(&text_chunk(s)).is_some());
        }
        assert!(!agg.is_suppressed());
    }

    // ---------- doom-loop ----------

    #[test]
    fn detect_doom_loop_flags_identical_repeats() {
        let calls = vec![
            ("ls".to_string(), "{}".to_string()),
            ("echo".to_string(), "{\"m\":\"x\"}".to_string()),
            ("echo".to_string(), "{\"m\":\"x\"}".to_string()),
            ("echo".to_string(), "{\"m\":\"x\"}".to_string()),
        ];
        assert_eq!(detect_doom_loop(&calls, 3), Some(("echo".to_string(), 3)));
    }

    #[test]
    fn detect_doom_loop_ignores_varied_args() {
        let calls = vec![
            ("echo".to_string(), "{\"m\":\"a\"}".to_string()),
            ("echo".to_string(), "{\"m\":\"b\"}".to_string()),
            ("echo".to_string(), "{\"m\":\"c\"}".to_string()),
        ];
        assert_eq!(detect_doom_loop(&calls, 3), None);
    }

    #[test]
    fn detect_doom_loop_needs_full_threshold() {
        let calls = vec![
            ("echo".to_string(), "{}".to_string()),
            ("echo".to_string(), "{}".to_string()),
        ];
        assert_eq!(detect_doom_loop(&calls, 3), None);
    }

    // ---------- 真流式回路 ----------

    #[tokio::test]
    async fn stream_loop_yields_text_deltas_before_done() {
        let mut history = vec![user("hi")];
        let (tx, rx) = mpsc::channel(64);
        let mut round = 0usize;
        let text = run_tool_loop_streaming(&mut history, echo_registry(), tx, 4, move |_msgs| {
            round += 1;
            assert_eq!(round, 1, "无工具调用应只跑一轮");
            async move {
                Ok(synthetic_stream(vec![
                    text_chunk("Hel"),
                    text_chunk("lo!"),
                    finish_chunk("stop"),
                ]))
            }
        })
        .await
        .expect("loop should converge");

        assert_eq!(text, "Hello!");
        let events = drain(rx).await;
        // 真流式证据：两个 TextDelta 在 Done 之前，且是分开的两块
        let deltas: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, vec!["Hel", "lo!"], "增量须逐块转发，不得攒批");
        let done_idx = events
            .iter()
            .position(|e| matches!(e, AgentEvent::Done { .. }))
            .expect("Done event");
        let last_delta_idx = events
            .iter()
            .rposition(|e| matches!(e, AgentEvent::TextDelta { .. }))
            .expect("TextDelta event");
        assert!(last_delta_idx < done_idx, "增量必须先于 Done 到达");
        // 历史回写最终回复
        assert!(matches!(history.last(), Some(Message::Assistant { .. })));
    }

    #[tokio::test]
    async fn stream_loop_emits_step_boundaries() {
        let mut history = vec![user("hi")];
        let (tx, rx) = mpsc::channel(64);
        run_tool_loop_streaming(&mut history, echo_registry(), tx, 4, |_m| async {
            Ok(synthetic_stream(vec![
                text_chunk("ok"),
                finish_chunk("stop"),
            ]))
        })
        .await
        .unwrap();
        let events = drain(rx).await;
        assert!(events.contains(&AgentEvent::StepStart { round: 1 }));
        assert!(events.contains(&AgentEvent::StepFinish {
            round: 1,
            reason: Some("stop".to_string())
        }));
    }

    #[tokio::test]
    async fn stream_loop_executes_tools_and_continues() {
        let mut history = vec![user("echo x")];
        let (tx, rx) = mpsc::channel(64);
        let mut round = 0u32;
        let text = run_tool_loop_streaming(&mut history, echo_registry(), tx, 4, move |_msgs| {
            round += 1;
            let r = round;
            async move {
                if r == 1 {
                    Ok(synthetic_stream(vec![tool_chunk("echo", "{\"m\":\"x\"}")]))
                } else {
                    Ok(synthetic_stream(vec![
                        text_chunk("done"),
                        finish_chunk("stop"),
                    ]))
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(text, "done");
        let events = drain(rx).await;
        assert!(events.contains(&AgentEvent::ToolCall {
            id: None,
            name: "echo".to_string(),
            arguments: "{\"m\":\"x\"}".to_string(),
            // echo 不在审批面内 → 不伪造 approval 字段。
            approval: None,
        }));
        assert!(events.contains(&AgentEvent::ToolResult {
            id: None,
            name: "echo".to_string(),
            output: "echo:x".to_string(),
            ok: true,
            approval: None,
        }));
        // 工具结果回灌历史
        assert!(history
            .iter()
            .any(|m| matches!(m, Message::Tool { name, output, .. } if name == "echo" && output == "echo:x")));
        // 两轮 StepStart
        assert!(events.contains(&AgentEvent::StepStart { round: 2 }));
    }

    #[tokio::test]
    async fn stream_loop_loads_discovered_schema_on_next_request() {
        let mut history = vec![user("解释一下这段代码")];
        let mut tools = vec![
            ToolDef {
                name: "read_file".into(),
                description: "读取文件".into(),
                parameters: serde_json::json!({"type":"object"}),
            },
            ToolDef {
                name: "office_automation".into(),
                description: "创建 Excel 工作簿".into(),
                parameters: serde_json::json!({"type":"object","properties":{"action":{"type":"string"}}}),
            },
        ];
        tools.extend((0..20).map(|i| ToolDef {
            name: format!("special_{i}"),
            description: "专用能力".into(),
            parameters: serde_json::json!({"type":"object"}),
        }));
        let disclosure = ToolDisclosure::new(&tools, "解释项目用途", crate::mode::ModeKind::Agent);
        let (tx, rx) = mpsc::channel(64);
        let visible = Arc::new(std::sync::Mutex::new(Vec::<Vec<String>>::new()));
        let seen = visible.clone();
        let text = run_tool_loop_streaming_with_disclosure(
            &mut history,
            echo_registry(),
            disclosure,
            tx,
            4,
            move |_messages, active| {
                let names = active
                    .iter()
                    .map(|tool| tool.name.clone())
                    .collect::<Vec<_>>();
                seen.lock().unwrap().push(names.clone());
                let round = seen.lock().unwrap().len();
                async move {
                    if round == 1 {
                        assert!(!names.iter().any(|name| name == "office_automation"));
                        Ok(synthetic_stream(vec![tool_chunk(
                            DISCOVER_TOOLS_NAME,
                            r#"{"query":"操作 Excel 工作簿"}"#,
                        )]))
                    } else {
                        assert!(names.iter().any(|name| name == "office_automation"));
                        Ok(synthetic_stream(vec![
                            text_chunk("完成"),
                            finish_chunk("stop"),
                        ]))
                    }
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(text, "完成");
        assert_eq!(visible.lock().unwrap().len(), 2);
        assert!(history.iter().any(|message| matches!(
            message,
            Message::Tool { name, output, .. }
                if name == DISCOVER_TOOLS_NAME && output.contains("office_automation")
        )));
        let _ = drain(rx).await;
    }

    #[tokio::test]
    async fn stream_loop_breaks_on_doom_loop() {
        let mut history = vec![user("go")];
        let (tx, rx) = mpsc::channel(64);
        // 永远返回同一工具同一入参 → 应在 DOOM_LOOP_ABORT（纠正无效后）处中止，而非烧满 max_rounds
        let result = run_tool_loop_streaming(&mut history, echo_registry(), tx, 20, |_m| async {
            Ok(synthetic_stream(vec![tool_chunk("echo", "{\"m\":\"x\"}")]))
        })
        .await;
        assert!(result.is_err(), "纠正后仍死循环应中止回路");
        let events = drain(rx).await;
        assert!(
            events.iter().any(|e| matches!(
                e,
                AgentEvent::DoomLoop { tool, repeats } if tool == "echo" && *repeats == DOOM_LOOP_ABORT
            )),
            "须如实上报 DoomLoop 事件，实际事件：{events:?}"
        );
        // 中止得早：不应跑满 20 轮
        let starts = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::StepStart { .. }))
            .count();
        assert!(
            starts <= DOOM_LOOP_ABORT + 1,
            "应尽早中止，实跑 {starts} 轮"
        );
    }

    /// **关键回归**：同参重复达「纠正阈值」时**绝不中止整轮** ——
    /// 必须照常执行、把纠正提示注进工具结果，让模型有机会自行改道后正常收敛。
    ///
    /// 历史缺陷：命中即 `return Err` 会炸掉整轮并降级为纯文本推理；而在单工具锚定面下
    /// （模型手上只有 `str_replace_editor`）「同一编辑工具连续调用 3 次」必然发生。
    #[tokio::test]
    async fn stream_loop_corrects_same_arg_repeats_without_aborting() {
        let mut history = vec![user("go")];
        let (tx, rx) = mpsc::channel(64);
        let mut round = 0u32;
        // 前 3 轮完全相同的调用（触发纠正阈值），第 4 轮改道 → 第 5 轮收敛给出正文
        let text = run_tool_loop_streaming(&mut history, echo_registry(), tx, 8, move |_m| {
            round += 1;
            let r = round;
            async move {
                if r <= 3 {
                    Ok(synthetic_stream(vec![tool_chunk(
                        "echo",
                        "{\"m\":\"same\"}",
                    )]))
                } else if r == 4 {
                    Ok(synthetic_stream(vec![tool_chunk(
                        "echo",
                        "{\"m\":\"changed\"}",
                    )]))
                } else {
                    Ok(synthetic_stream(vec![
                        text_chunk("收敛"),
                        finish_chunk("stop"),
                    ]))
                }
            }
        })
        .await
        .expect("同参重复达纠正阈值后不得中止整轮，应能正常收敛");

        assert_eq!(text, "收敛");
        let events = drain(rx).await;
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AgentEvent::DoomLoop { .. })),
            "未达硬中止阈值不得上报 DoomLoop 中止事件"
        );
        assert!(
            events.iter().any(
                |e| matches!(e, AgentEvent::Thinking { text } if text.contains("已注入纠正提示") )
            ),
            "须如实上报「已注入纠正提示」，实际事件：{events:?}"
        );
        // 纠正提示确实注进了工具结果（模型下一轮必然读到）
        assert!(
            history.iter().any(|m| matches!(
                m,
                Message::Tool { output, .. } if output.contains("[重复调用提醒]")
            )),
            "纠正提示必须写进工具结果"
        );
        // 第 3 次同参调用的结果里，提醒文案须点明重复次数
        assert!(
            history.iter().any(|m| matches!(
                m,
                Message::Tool { output, .. }
                    if output.contains(&format!("连续 {} 次", DOOM_LOOP_THRESHOLD))
            )),
            "提醒须点明重复次数 {DOOM_LOOP_THRESHOLD}"
        );
    }

    #[tokio::test]
    async fn stream_loop_propagates_stream_error() {
        let mut history = vec![user("hi")];
        let (tx, rx) = mpsc::channel(64);
        let result = run_tool_loop_streaming(&mut history, echo_registry(), tx, 4, |_m| async {
            Ok(error_stream("upstream exploded"))
        })
        .await;
        assert!(result.is_err());
        let events = drain(rx).await;
        assert!(
            events.iter().any(|e| matches!(
                e,
                AgentEvent::Error { message, .. } if message.contains("upstream exploded")
            )),
            "须如实上报上游错误，实际事件：{events:?}"
        );
        // R65/B1：未归类错误 → `code` 为 None（前端回退到服务端文案，不误报稳定码）。
        assert!(
            events
                .iter()
                .all(|e| !matches!(e, AgentEvent::Error { code: Some(_), .. })),
            "纯字符串错误不应伪造稳定码，实际事件：{events:?}"
        );
    }

    /// R65/B1：内层已归类的错误（provider AUTH_FAILED）→ 事件带稳定码，
    /// 前端据此本地化，而不是展示 `code: message` 英文正文。
    #[tokio::test]
    async fn stream_loop_attaches_stable_code_when_classified() {
        let mut history = vec![user("hi")];
        let (tx, rx) = mpsc::channel(64);
        let boom = || {
            anyhow::Error::new(crate::model::error_taxonomy::ProviderError::from_status(
                401,
                "raw upstream body",
            ))
        };
        let result = run_tool_loop_streaming(&mut history, echo_registry(), tx, 4, move |_m| {
            let boom = boom();
            async move { Err(boom) }
        })
        .await;
        assert!(result.is_err());
        let events = drain(rx).await;
        assert!(
            events.iter().any(|e| matches!(
                e,
                AgentEvent::Error { code: Some(c), .. } if c == "AUTH_FAILED"
            )),
            "已归类错误须带稳定码 AUTH_FAILED，实际事件：{events:?}"
        );
        // 用户可见文案不得回显上游原始正文。
        assert!(
            events.iter().all(|e| !matches!(
                e,
                AgentEvent::Error { message, .. } if message.contains("raw upstream body")
            )),
            "不得回显上游原始正文：{events:?}"
        );
    }

    #[tokio::test]
    async fn stream_loop_respects_max_rounds() {
        let mut history = vec![user("go")];
        let (tx, _rx) = mpsc::channel(256);
        let mut n = 0u32;
        // 每轮换入参，避开 doom-loop，专测轮数上限
        let result = run_tool_loop_streaming(&mut history, echo_registry(), tx, 3, move |_m| {
            n += 1;
            let args = format!("{{\"m\":\"{n}\"}}");
            async move { Ok(synthetic_stream(vec![tool_chunk("echo", &args)])) }
        })
        .await;
        assert!(result.is_err(), "超过 max_rounds 应报错");
        assert!(
            result.unwrap_err().to_string().contains("3"),
            "错误须点明轮数上限"
        );
    }

    // ---------- E12：工具输出 → 上下文令牌台账 ----------

    /// 40k 字符工具输出的「生产方」（完整文本进入 `Message::Tool.output`）。
    struct BigOutputTool;

    #[async_trait]
    impl ToolExecutor for BigOutputTool {
        fn name(&self) -> &str {
            "big_out"
        }
        fn description(&self) -> &str {
            "E12 回归：返回 4 万字符输出"
        }
        async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
            Ok(ToolResult::Success {
                data: "x".repeat(40_000),
                metadata: HashMap::new(),
            })
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }
    }

    /// E12 回归（M1 Task 3 改写）：工具输出的三个去向**各有各的正确口径**。
    ///
    /// 旧断言只有一条「历史里就是全文 40 000 字符」，它把三个本应不同的口径压成
    /// 同一个；入口截流（D11）接上后那条断言既不再成立、也不该削弱成「随便截一点」：
    /// - **事件流**：给前端与审计看，必须全文（截了用户就看不到日志）；
    /// - **模型历史**：受入口截流约束，只带头尾 + **真能取回**的指纹；
    /// - **台账 `overhead_tokens`**：按「真正进入模型上下文的文本」计量 ——
    ///   截流后必须按截断后计，否则用量表与压缩阈值会一起偏小。
    ///
    /// 另：修复前本回路自身不记账，只靠网关事件桥（异步、且部分事件只有摘要）→
    /// `overhead_tokens` 恒为 0、压缩阈值永远晚到。这条也一并钉在这里。
    #[tokio::test]
    async fn stream_loop_splits_tool_output_across_events_history_and_ledger() {
        let mut reg = ToolRegistry::new();
        reg.register_local(Box::new(BigOutputTool));
        let registry = Arc::new(reg);
        let session = format!("mgt-e12-stream-{}", uuid::Uuid::new_v4());
        let mut history = vec![user("读大输出")];
        let (tx, rx) = mpsc::channel(64);
        let mut round = 0u32;
        let result = run_tool_loop_streaming_inner(
            &mut history,
            Some(session.clone()),
            registry,
            None,
            tx,
            4,
            move |_m, _tools| {
                round += 1;
                async move {
                    if round == 1 {
                        Ok(synthetic_stream(vec![tool_chunk("big_out", "{}")]))
                    } else {
                        Ok(synthetic_stream(vec![
                            text_chunk("完成"),
                            finish_chunk("stop"),
                        ]))
                    }
                }
            },
        )
        .await;
        assert!(result.is_ok(), "回路应正常收敛：{result:?}");
        // ① 历史：短于原文，且**如实标注**被截 + 给出真能取回的指纹
        let tool_out = history
            .iter()
            .find_map(|m| match m {
                Message::Tool { output, .. } => Some(output.clone()),
                _ => None,
            })
            .expect("回路应追加 Message::Tool");
        assert!(tool_out.len() < 40_000, "入口截流应生效：{}", tool_out.len());
        assert!(
            tool_out.contains("中间省略"),
            "不得静默丢弃：{}",
            &tool_out[..tool_out.len().min(200)]
        );
        let fp = tool_out
            .split("tool_output_retrieve(\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("截断标注必须给出取回指纹")
            .to_string();

        // ② 取回：逐字节等于原文 —— 假承诺 → 真兑现的唯一证明
        let full = crate::context::spool::retrieve(&fp).expect("指纹必须取得回");
        assert_eq!(full.len(), 40_000, "取回必须逐字节等于原文");
        assert_eq!(full, "x".repeat(40_000));

        // ③ 事件流：仍是全文（前端与审计不因截流而瞎）
        let events = drain(rx).await;
        assert!(
            events.iter().any(|e| matches!(
                e,
                AgentEvent::ToolResult { output, .. } if output.len() == 40_000
            )),
            "ToolResult 事件应携带完整输出"
        );

        // ④ 台账：按**进入模型的截断后文本**计量，而不是按原文
        let expected = crate::context::session::estimate_tokens(&tool_out);
        assert!(expected > 5_000, "截断后仍应是上万 token 量级：{expected}");
        let status = crate::context::session::context_status(&session);
        assert_eq!(
            status["overhead_tokens"].as_u64(),
            Some(expected),
            "台账要等于模型实际看到的：{status}"
        );
        assert!(
            crate::context::session::estimate_tokens(&full) > expected,
            "自证：原文口径确实更大，否则上面那条断言是空断言"
        );

        let _ = std::fs::remove_file(
            crate::persistence::config::Config::data_dir()
                .join("context")
                .join(format!("{session}.json")),
        );
    }
}
