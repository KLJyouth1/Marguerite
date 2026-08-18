//! 自进化运行器（参考 deepseek-auto-evolving-harness / penguin-harness）
//!
//! 在 distill.rs 的 `SelfEvolveGate` / `run_self_evolve` 基础上增加：
//! - 进化历史持久化（JSONL，支持回滚）
//! - 多轮进化循环（诊断 → 变异 → 评分 → 门禁 → 落盘）
//! - 进化结果应用到系统 prompt
//!
//! 这是 `harness evolve` CLI 命令的后端。

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::harness::distill::{diagnose, run_self_evolve};

/// 单次进化记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionRecord {
    /// 进化轮次（从 1 开始）
    pub round: usize,
    /// 时间戳
    pub timestamp: String,
    /// 诊断的薄弱面
    pub diagnosis: String,
    /// 变异前评分
    pub score_before: f64,
    /// 变异后评分
    pub score_after: f64,
    /// 是否被门禁采纳
    pub accepted: bool,
    /// 变异后的 prompt（仅 accepted 时存在）
    pub mutated_prompt: Option<String>,
    /// 拒绝原因（仅 rejected 时存在）
    pub rejection_reason: Option<String>,
}

/// 自进化运行器
pub struct EvolutionRunner {
    /// 进化历史落盘目录
    data_dir: PathBuf,
    /// 门禁阈值
    threshold: f64,
    /// 当前系统 prompt
    current_prompt: String,
    /// 已执行轮次
    rounds: usize,
}

impl EvolutionRunner {
    /// 创建进化运行器
    pub fn new(data_dir: impl Into<PathBuf>, base_prompt: impl Into<String>) -> Self {
        let data_dir = data_dir.into();
        Self {
            data_dir,
            threshold: 0.6,
            current_prompt: base_prompt.into(),
            rounds: 0,
        }
    }

    /// 设置门禁阈值
    pub fn with_threshold(mut self, threshold: f64) -> Self {
        self.threshold = threshold;
        self
    }

    /// 当前 prompt
    pub fn current_prompt(&self) -> &str {
        &self.current_prompt
    }

    /// 已执行轮次
    pub fn rounds(&self) -> usize {
        self.rounds
    }

    /// 执行一轮进化
    ///
    /// 参数：
    /// - `aspects`：诊断出的薄弱面（如 ["幻觉", "工具调用格式错误"]）
    /// - `score_before`：变异前评分
    /// - `score_after`：变异后评分
    pub fn evolve_round(
        &mut self,
        aspects: &[&str],
        score_before: f64,
        score_after: f64,
    ) -> Result<EvolutionRecord> {
        self.rounds += 1;
        let diagnosis = diagnose(aspects);
        let accepted = run_self_evolve(
            &self.current_prompt,
            &diagnosis,
            score_before,
            score_after,
            self.threshold,
        );

        let record = match accepted {
            Some(mutated) => {
                self.current_prompt = mutated.clone();
                EvolutionRecord {
                    round: self.rounds,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    diagnosis,
                    score_before,
                    score_after,
                    accepted: true,
                    mutated_prompt: Some(mutated),
                    rejection_reason: None,
                }
            }
            None => {
                let reason = if score_after <= score_before {
                    "评分未提升".to_string()
                } else if score_after < self.threshold {
                    format!("评分 {score_after:.2} 未达阈值 {}", self.threshold)
                } else {
                    "门禁拒绝".to_string()
                };
                EvolutionRecord {
                    round: self.rounds,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    diagnosis,
                    score_before,
                    score_after,
                    accepted: false,
                    mutated_prompt: None,
                    rejection_reason: Some(reason),
                }
            }
        };

        self.append_record(&record)?;
        Ok(record)
    }

    /// 执行多轮进化（deepseek-auto-evolving-harness 的 6 轮方法论）
    ///
    /// 每轮聚焦不同的优化方向：
    /// 1. 缩短执行路径
    /// 2. 修正工具调用格式
    /// 3. 调整 system prompt
    /// 4. 调整推理参数
    /// 5. 增加执行后反思
    /// 6. 综合优化
    pub fn evolve_multi_round(
        &mut self,
        mut evaluate: impl FnMut(&str) -> f64,
    ) -> Result<Vec<EvolutionRecord>> {
        let aspects_per_round: Vec<Vec<&str>> = vec![
            vec!["执行路径冗长"],
            vec!["工具调用格式错误"],
            vec!["system prompt 不精确"],
            vec!["推理参数不当"],
            vec!["缺少执行后反思"],
            vec!["综合能力薄弱"],
        ];

        let mut records = Vec::new();
        for aspects in &aspects_per_round {
            let score_before = evaluate(&self.current_prompt);
            // 模拟变异后的评分（实际应由评估函数对变异 prompt 评分）
            // 这里用诊断 + 变异 prompt 评估
            let diagnosis = diagnose(aspects);
            let mutated = crate::harness::distill::mutate_prompt(&self.current_prompt, &diagnosis);
            let score_after = evaluate(&mutated);

            let record = self.evolve_round(aspects, score_before, score_after)?;
            records.push(record);

            // 如果连续 3 轮被拒绝，提前停止
            let recent_rejected = records.iter().rev().take(3).filter(|r| !r.accepted).count();
            if recent_rejected >= 3 {
                break;
            }
        }
        Ok(records)
    }

    /// 回滚到指定轮次
    pub fn rollback(&mut self, target_round: usize) -> Result<()> {
        let history = self.load_history()?;
        let target = history
            .iter()
            .find(|r| r.round == target_round && r.accepted)
            .ok_or_else(|| anyhow::anyhow!("轮次 {target_round} 不存在或未被采纳"))?;

        self.current_prompt = target
            .mutated_prompt
            .clone()
            .ok_or_else(|| anyhow::anyhow!("轮次 {target_round} 无变异 prompt"))?;
        self.rounds = target_round;
        Ok(())
    }

    /// 加载进化历史
    pub fn load_history(&self) -> Result<Vec<EvolutionRecord>> {
        let path = self.history_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = std::fs::read_to_string(&path)?;
        let records: Vec<EvolutionRecord> = content
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        Ok(records)
    }

    /// 获取历史文件路径
    fn history_path(&self) -> PathBuf {
        self.data_dir.join("evolutions.jsonl")
    }

    /// 追加记录到历史文件
    fn append_record(&self, record: &EvolutionRecord) -> Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        let line = serde_json::to_string(record)?;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.history_path())?;
        writeln!(file, "{line}")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_prompt() -> &'static str {
        "你是 Marguerite 助手。"
    }

    #[test]
    fn test_evolve_round_accepted() {
        let dir = std::env::temp_dir().join(format!("marguerite_evolve_{}", uuid::Uuid::new_v4()));
        let mut runner = EvolutionRunner::new(&dir, base_prompt());
        let record = runner.evolve_round(&["幻觉"], 0.5, 0.85).unwrap();
        assert!(record.accepted);
        assert!(record.mutated_prompt.is_some());
        assert!(runner.current_prompt().contains("幻觉"));
        assert_eq!(runner.rounds(), 1);
        // 历史文件应存在
        assert!(dir.join("evolutions.jsonl").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_evolve_round_rejected() {
        let dir = std::env::temp_dir().join(format!("marguerite_evolve_{}", uuid::Uuid::new_v4()));
        let mut runner = EvolutionRunner::new(&dir, base_prompt());
        let record = runner.evolve_round(&["幻觉"], 0.8, 0.5).unwrap();
        assert!(!record.accepted);
        assert!(record.rejection_reason.is_some());
        // prompt 不应改变
        assert_eq!(runner.current_prompt(), base_prompt());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_rollback() {
        let dir = std::env::temp_dir().join(format!("marguerite_evolve_{}", uuid::Uuid::new_v4()));
        let mut runner = EvolutionRunner::new(&dir, base_prompt());
        // 第一轮：采纳
        runner.evolve_round(&["幻觉"], 0.5, 0.85).unwrap();
        let prompt_after_r1 = runner.current_prompt().to_string();
        // 第二轮：采纳
        runner.evolve_round(&["冗长"], 0.6, 0.9).unwrap();
        assert_ne!(runner.current_prompt(), prompt_after_r1);
        // 回滚到第一轮
        runner.rollback(1).unwrap();
        assert_eq!(runner.current_prompt(), prompt_after_r1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_multi_round_early_stop() {
        let dir = std::env::temp_dir().join(format!("marguerite_evolve_{}", uuid::Uuid::new_v4()));
        let mut runner = EvolutionRunner::new(&dir, base_prompt());
        // 评估函数始终返回低分 → 全部拒绝 → 3 轮后停止
        let records = runner.evolve_multi_round(|_| 0.3).unwrap();
        assert!(records.len() <= 4); // 3 轮拒绝后停止
        assert!(records.iter().all(|r| !r.accepted));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_history() {
        let dir = std::env::temp_dir().join(format!("marguerite_evolve_{}", uuid::Uuid::new_v4()));
        let mut runner = EvolutionRunner::new(&dir, base_prompt());
        runner.evolve_round(&["幻觉"], 0.5, 0.85).unwrap();
        runner.evolve_round(&["冗长"], 0.6, 0.55).unwrap();
        let history = runner.load_history().unwrap();
        assert_eq!(history.len(), 2);
        assert!(history[0].accepted);
        assert!(!history[1].accepted);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
