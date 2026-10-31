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

/// 落盘文件名（不含目录）。`observability::io_report` 汇总各代大小时用同一个名字，
/// 免得两处各写一遍字符串字面量、改一个漏一个。
pub(crate) const LOG_FILE_NAME: &str = "router_decision_log.jsonl";

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
        dir.join(LOG_FILE_NAME)
    }

    /// 追加一行决策日志到默认 data_dir（best-effort，失败仅 warn）。
    pub fn record(decision: &RouteDecision, session: &str, turn_id: &str, model_name: &str) {
        let path = Self::log_path(&Config::data_dir());
        if let Err(e) = Self::append_one(&path, decision, session, turn_id, model_name, ROTATE_AT_BYTES) {
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
        Self::append_one(
            &Self::log_path(dir),
            decision,
            session,
            turn_id,
            model_name,
            ROTATE_AT_BYTES,
        )
    }

    /// [`Self::record_to_dir`] 的显式轮转阈值版本（供单测用小阈值驱动轮转）。
    ///
    /// 阈值做成参数而不是让测试去改常量：10 MB 的真实门槛在单测里要写 10 MB 的
    /// 决策记录才能触发，那是把测试时间花在制造体积上，而不是验证语义上。
    pub fn record_to_dir_with_rotation(
        decision: &RouteDecision,
        session: &str,
        turn_id: &str,
        model_name: &str,
        dir: &Path,
        rotate_at_bytes: u64,
    ) -> std::io::Result<()> {
        Self::append_one(
            &Self::log_path(dir),
            decision,
            session,
            turn_id,
            model_name,
            rotate_at_bytes,
        )
    }

    fn append_one(
        path: &Path,
        decision: &RouteDecision,
        session: &str,
        turn_id: &str,
        model_name: &str,
        rotate_at_bytes: u64,
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
        // `f` 必须在轮转之前释放：Windows 上重命名一个仍被本进程打开的文件会失败
        // （共享冲突），于是轮转永远不生效 —— 而这正是本条要修的东西。
        {
            let mut f = OpenOptions::new().create(true).append(true).open(path)?;
            let line = serde_json::to_string(&entry)?;
            writeln!(f, "{line}")?;
            // W7：这条是真实的追加写出口之一，计入「本进程写入次数」。
            crate::observability::io_report::tally_write(line.len() as u64 + 1);
        }
        // W4（2026-10-30）：这份日志此前是**纯 append、永不轮转**。
        // 每次路由决策一行，每行带完整 `steps` 管线轨迹 ⇒ 长任务下它只增不减，
        // 而且没有任何界面会读它（只有排障的人手工看）。
        // 复用 `observability::rotation` 的同一套「按大小切代 + 保留 3 代」，
        // 别再写第四份轮转实现；上限也直接沿用侧车日志那档 10 MB（同仓已有口径）。
        if let Err(err) =
            crate::observability::rotation::rotate_if_oversized_with(path, rotate_at_bytes)
        {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "路由决策日志轮转失败，本次继续写入（不影响对话）"
            );
        }
        Ok(())
    }
}

/// 决策日志单文件的轮转阈值。沿用 [`crate::observability::rotation::MAX_LOG_BYTES`]
/// 而不是另起一个数字：同一仓里「一个运行时日志文件该多大」只该有一个口径。
/// `pub(crate)`：`observability::io_report` 要把它随健康报告下发，
/// 面板才知道「多大算超」，而不是显示一个字节数让人自己猜阈值。
pub(crate) const ROTATE_AT_BYTES: u64 = crate::observability::rotation::MAX_LOG_BYTES;
