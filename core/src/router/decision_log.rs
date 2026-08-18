//! DecisionLog：决策日志 JSONL 落盘（best-effort，失败仅 warn 不阻断对话）。
//!
//! 落盘文件：`{Config::data_dir()}/router_decision_log.jsonl`（与 `sandbox/audit.rs`、
//! `memory/behavior.rs` 的 JSONL 实践一致）。敏感字段仅存 hash：模型名经
//! [`crate::cache::hash_text`] 取 FNV-1a 哈希后格式化为小写十六进制；原始用户输入
//! 永不入日志。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::*;
use crate::persistence::config::Config;

/// 决策日志 schema 版本（前向兼容：消费方忽略未知字段）。
pub const SCHEMA_VERSION: u64 = 1;

/// 单条决策日志（落盘 JSONL；敏感字段仅存 hash）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub schema_version: u64,
    pub ts: String,
    pub session: String,
    pub turn_id: String,
    pub tier: PersonaTier,
    pub thinking_mode: ThinkingMode,
    pub prompt_policy: PromptPolicy,
    pub confidence: f64,
    pub source: DecisionSource,
    /// 模型名哈希（小写十六进制），不直接存明文模型名
    pub model_hash: String,
    pub fallback_reason: Option<String>,
    pub steps: Vec<PipelineStepRecord>,
}

/// 决策日志写入器（best-effort，失败仅 warn 不阻断对话）。
pub struct DecisionLog;

impl DecisionLog {
    fn log_path(dir: &Path) -> PathBuf {
        dir.join("router_decision_log.jsonl")
    }

    /// 追加一行决策日志到默认 data_dir（best-effort，失败仅 warn）。
    pub fn record(decision: &RouteDecision, session: &str, turn_id: &str, model_name: &str) {
        let path = Self::log_path(&Config::data_dir());
        if let Err(e) = Self::append_one(&path, decision, session, turn_id, model_name) {
            tracing::warn!(
                error = %e,
                session,
                "router decision_log write failed (best-effort, ignored)"
            );
        }
    }

    /// 写入到指定目录（测试用，避免污染真实 data_dir）。
    pub fn record_to_dir(
        decision: &RouteDecision,
        session: &str,
        turn_id: &str,
        model_name: &str,
        dir: &Path,
    ) -> std::io::Result<()> {
        Self::append_one(&Self::log_path(dir), decision, session, turn_id, model_name)
    }

    fn append_one(
        path: &Path,
        decision: &RouteDecision,
        session: &str,
        turn_id: &str,
        model_name: &str,
    ) -> std::io::Result<()> {
        let entry = LogEntry {
            schema_version: SCHEMA_VERSION,
            ts: Utc::now().to_rfc3339(),
            session: session.to_string(),
            turn_id: turn_id.to_string(),
            tier: decision.tier,
            thinking_mode: decision.thinking_mode,
            prompt_policy: decision.prompt_policy,
            confidence: decision.confidence,
            source: decision.source,
            model_hash: format!("{:x}", crate::cache::hash_text(model_name)),
            fallback_reason: decision.fallback_reason.clone(),
            steps: decision.steps.clone(),
        };
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(f, "{}", serde_json::to_string(&entry)?)?;
        Ok(())
    }
}
