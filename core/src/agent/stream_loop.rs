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

use std::future::Future;
use std::sync::Arc;

#[cfg(test)]
use std::collections::HashMap;

use anyhow::{anyhow, Result};
use chrono::Utc;
use tokio::sync::mpsc;

use crate::agent::runtime::parse_tool_args;
use crate::agent::tool_cycle::{ToolCall as CycleToolCall, ToolCycle, ToolResult};
use crate::mcp::registry::ToolRegistry;
use crate::model::llm::{LLMChunk, ToolCall as LlmToolCall};
use crate::model::StreamReceiver;
use crate::types::Message;

/// opencode `DOOM_LOOP_THRESHOLD`（processor.ts:24）
pub const DOOM_LOOP_THRESHOLD: usize = 3;

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
    /// 模型请求调用工具（对应 `tool-call`）
    ToolCall { name: String, arguments: String },
    /// 工具执行结果（对应 `tool-result` / `tool-error`）；`ok=false` 表示错误或超时
    ToolResult {
        name: String,
        output: String,
        ok: bool,
    },
    /// 检测到 doom-loop：同一工具 + 同一入参连续重复（对应 processor.ts:306-330）
    DoomLoop { tool: String, repeats: usize },
    /// 正文因退化重复被抑制（输出去冗余，如实上报而非静默截断）
    OutputSuppressed { repeated_line: String },
    /// 回路收敛，给出最终文本
    Done { text: String },
    /// 出错（对应 `error`）
    Error { message: String },
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
    let cycle = ToolCycle::new(registry);
    // 按时序累积的 `(工具名, 入参)`，仅用于 doom-loop 判定
    let mut call_log: Vec<(String, String)> = Vec::new();

    for round in 1..=max_rounds {
        let _ = events.send(AgentEvent::StepStart { round }).await;

        let mut rx = match round_stream(history.clone()).await {
            Ok(rx) => rx,
            Err(e) => {
                let _ = events
                    .send(AgentEvent::Error {
                        message: e.to_string(),
                    })
                    .await;
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
                    let _ = events
                        .send(AgentEvent::Error {
                            message: e.to_string(),
                        })
                        .await;
                    return Err(e);
                }
            };

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

        // doom-loop 检测：先记账再判定，命中即如实上报并中止（不静默烧满轮数）
        for tc in &tool_calls {
            call_log.push((tc.name.clone(), tc.arguments.clone()));
        }
        if let Some((tool, repeats)) = detect_doom_loop(&call_log, DOOM_LOOP_THRESHOLD) {
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

        for tc in &tool_calls {
            let _ = events
                .send(AgentEvent::ToolCall {
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                })
                .await;
        }

        let calls: Vec<CycleToolCall> = tool_calls
            .iter()
            .map(|tc| CycleToolCall::new(&tc.name, parse_tool_args(&tc.arguments)))
            .collect();
        let results = match cycle.execute_sequential(calls).await {
            Ok(r) => r,
            Err(e) => {
                let _ = events
                    .send(AgentEvent::Error {
                        message: e.to_string(),
                    })
                    .await;
                return Err(e);
            }
        };

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

        for (tc, res) in tool_calls.iter().zip(results.into_iter()) {
            let (output, ok) = match &res {
                ToolResult::Success { data, .. } => (data.clone(), true),
                ToolResult::Error { message, code } => {
                    (format!("[工具错误 {code}] {message}"), false)
                }
                ToolResult::Timeout { tool_name, .. } => (format!("[工具超时] {tool_name}"), false),
            };
            let _ = events
                .send(AgentEvent::ToolResult {
                    name: tc.name.clone(),
                    output: output.clone(),
                    ok,
                })
                .await;
            // 工具结果回灌历史，供下一轮模型观测（observe）
            history.push(Message::Tool {
                name: tc.name.clone(),
                input: tc.arguments.clone(),
                output,
                timestamp: Utc::now(),
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
        }
    }

    fn thinking_chunk(s: &str) -> LLMChunk {
        LLMChunk {
            content: None,
            thinking: Some(s.to_string()),
            finish_reason: None,
            tool_calls: None,
        }
    }

    fn finish_chunk(reason: &str) -> LLMChunk {
        LLMChunk {
            content: None,
            thinking: None,
            finish_reason: Some(reason.to_string()),
            tool_calls: None,
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
            name: "echo".to_string(),
            arguments: "{\"m\":\"x\"}".to_string()
        }));
        assert!(events.contains(&AgentEvent::ToolResult {
            name: "echo".to_string(),
            output: "echo:x".to_string(),
            ok: true
        }));
        // 工具结果回灌历史
        assert!(history
            .iter()
            .any(|m| matches!(m, Message::Tool { name, output, .. } if name == "echo" && output == "echo:x")));
        // 两轮 StepStart
        assert!(events.contains(&AgentEvent::StepStart { round: 2 }));
    }

    #[tokio::test]
    async fn stream_loop_breaks_on_doom_loop() {
        let mut history = vec![user("go")];
        let (tx, rx) = mpsc::channel(64);
        // 永远返回同一工具同一入参 → 应在 DOOM_LOOP_THRESHOLD 处中止，而非烧满 max_rounds
        let result = run_tool_loop_streaming(&mut history, echo_registry(), tx, 20, |_m| async {
            Ok(synthetic_stream(vec![tool_chunk("echo", "{\"m\":\"x\"}")]))
        })
        .await;
        assert!(result.is_err(), "doom-loop 应中止回路");
        let events = drain(rx).await;
        assert!(
            events.iter().any(|e| matches!(
                e,
                AgentEvent::DoomLoop { tool, repeats } if tool == "echo" && *repeats == DOOM_LOOP_THRESHOLD
            )),
            "须如实上报 DoomLoop 事件，实际事件：{events:?}"
        );
        // 中止得早：不应跑满 20 轮
        let starts = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::StepStart { .. }))
            .count();
        assert!(
            starts <= DOOM_LOOP_THRESHOLD + 1,
            "应尽早中止，实跑 {starts} 轮"
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
            events.iter().any(
                |e| matches!(e, AgentEvent::Error { message } if message.contains("upstream exploded"))
            ),
            "须如实上报上游错误，实际事件：{events:?}"
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
}
