//! 决策协议（Decision Protocol）：追加式、防篡改的决策台账。
//!
//! 每条决策记录携带前一条记录的 sha256（链式），任何对历史记录的涂改都会
//! 在 `verify_chain` 时被检出。这是 标准 AI 助手 不具备的可审计治理能力。

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 决策结论
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionOutcome {
    Approved,
    Rejected,
    ApprovedWithConditions,
}

impl DecisionOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            DecisionOutcome::Approved => "approved",
            DecisionOutcome::Rejected => "rejected",
            DecisionOutcome::ApprovedWithConditions => "approved_with_conditions",
        }
    }
}

/// 单条决策记录（链式）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub seq: u64,
    pub ts: String,
    pub plan_id: String,
    pub outcome: String,
    pub rationale: String,
    pub actor: String,
    /// 前一条记录的 hash（创世记录为空串）
    pub prev_hash: String,
    /// 本条记录的 hash（由其余字段计算）
    pub hash: String,
}

/// 决策台账（内存态，可序列化持久化）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DecisionLog {
    records: Vec<DecisionRecord>,
}

impl DecisionLog {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    fn hash_record(
        seq: u64,
        ts: &str,
        plan_id: &str,
        outcome: &str,
        rationale: &str,
        actor: &str,
        prev_hash: &str,
    ) -> String {
        let mut h = Sha256::new();
        let payload = format!(
            "{}|{}|{}|{}|{}|{}|{}",
            seq, ts, plan_id, outcome, rationale, actor, prev_hash
        );
        h.update(payload.as_bytes());
        let out = h.finalize();
        out.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// 追加一条决策，返回生成的记录
    pub fn append(
        &mut self,
        plan_id: &str,
        outcome: DecisionOutcome,
        rationale: &str,
        actor: &str,
    ) -> Result<DecisionRecord> {
        let seq = (self.records.len() as u64) + 1;
        let ts = Utc::now().to_rfc3339();
        let prev_hash = self
            .records
            .last()
            .map(|r| r.hash.clone())
            .unwrap_or_default();
        let outcome_s = outcome.as_str().to_string();
        let hash = Self::hash_record(seq, &ts, plan_id, &outcome_s, rationale, actor, &prev_hash);
        let rec = DecisionRecord {
            seq,
            ts,
            plan_id: plan_id.to_string(),
            outcome: outcome_s,
            rationale: rationale.to_string(),
            actor: actor.to_string(),
            prev_hash,
            hash: hash.clone(),
        };
        self.records.push(rec.clone());
        Ok(rec)
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn records(&self) -> &[DecisionRecord] {
        &self.records
    }

    /// 校验整条决策链的完整性（防篡改）
    pub fn verify_chain(&self) -> Result<()> {
        let mut prev = String::new();
        for r in &self.records {
            if r.prev_hash != prev {
                return Err(anyhow!("chain broken at seq {}: prev_hash mismatch", r.seq));
            }
            let expected = Self::hash_record(
                r.seq,
                &r.ts,
                &r.plan_id,
                &r.outcome,
                &r.rationale,
                &r.actor,
                &r.prev_hash,
            );
            if r.hash != expected {
                return Err(anyhow!(
                    "hash mismatch at seq {}: tampering suspected",
                    r.seq
                ));
            }
            prev = r.hash.clone();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_verify_chain() {
        let mut log = DecisionLog::new();
        log.append(
            "p1",
            DecisionOutcome::Approved,
            "clean plan",
            "orchestrator",
        )
        .unwrap();
        log.append(
            "p2",
            DecisionOutcome::Rejected,
            "destructive unapproved",
            "orchestrator",
        )
        .unwrap();
        log.append(
            "p3",
            DecisionOutcome::ApprovedWithConditions,
            "needs review",
            "user",
        )
        .unwrap();
        assert_eq!(log.len(), 3);
        assert!(log.verify_chain().is_ok());
    }

    #[test]
    fn tamper_detected_on_outcome_change() {
        let mut log = DecisionLog::new();
        log.append("p1", DecisionOutcome::Approved, "clean", "orchestrator")
            .unwrap();
        log.append("p2", DecisionOutcome::Approved, "clean2", "orchestrator")
            .unwrap();
        // 模拟持久化后被涂改：直接改动历史记录字段
        log.records[0].outcome = "rejected".to_string();
        assert!(log.verify_chain().is_err());
    }

    #[test]
    fn tamper_detected_on_prev_hash_break() {
        let mut log = DecisionLog::new();
        log.append("p1", DecisionOutcome::Approved, "clean", "orchestrator")
            .unwrap();
        log.append("p2", DecisionOutcome::Approved, "clean2", "orchestrator")
            .unwrap();
        log.records[1].prev_hash = "forged".to_string();
        assert!(log.verify_chain().is_err());
    }
}
