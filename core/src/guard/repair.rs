//! 自动修复循环
//!
//! 集成护栏网关：语法错误 → 规则修复 / LLM 修复 → 重新验证。
//! 修复上限 2 次，避免无限循环。借鉴 NexusStar repair_loop 思路，
//! 规则修复零成本，LLM 修复按需启用。

use std::time::Duration;

use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::{GuardGateway, GuardLayer, Severity};
use crate::guard::syntax::Language;
use crate::model::router::ModelRouter;
use crate::types::Message;

/// 修复配置
#[derive(Debug, Clone)]
pub struct RepairConfig {
    /// 最大修复尝试次数
    pub max_attempts: usize,
    /// 是否启用 LLM 修复（需配置 API Key）
    pub use_llm: bool,
    /// LLM 修复超时（秒）
    pub llm_timeout_secs: u64,
}

impl Default for RepairConfig {
    fn default() -> Self {
        Self {
            max_attempts: 2,
            use_llm: true,
            llm_timeout_secs: 30,
        }
    }
}

/// 修复结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairResult {
    /// 修复后的代码
    pub code: String,
    /// 实际尝试次数
    pub attempts: usize,
    /// 是否已修复（无 Error 级问题）
    pub fixed: bool,
    /// 最终护栏报告
    pub report: super::GuardReport,
}

/// 自动修复循环
///
/// 流程：护栏检查 → 存在语法错误 → 规则/LLM 修复 → 重新检查。
/// 语义/事实/契约问题为 Warning，不阻断代码生成，但保留在报告中。
pub struct RepairLoop {
    gateway: GuardGateway,
    config: RepairConfig,
}

impl RepairLoop {
    pub fn new(gateway: GuardGateway, config: RepairConfig) -> Self {
        Self { gateway, config }
    }

    /// 执行修复循环（同步规则修复）
    ///
    /// 语法错误自动修复，最多 `max_attempts` 次。
    pub fn repair(&self, code: &str, lang: Language) -> RepairResult {
        let mut current = code.to_string();
        let mut attempts = 0;

        loop {
            let report = self.gateway.check(&current, lang);
            if !report.has_error() || attempts >= self.config.max_attempts {
                return RepairResult {
                    code: current,
                    attempts,
                    fixed: !report.has_error(),
                    report,
                };
            }
            let Some(fixed) = Self::apply_rules(&current, &report) else {
                // 规则无法修复 → 交给 LLM（异步方法）
                return RepairResult {
                    code: current,
                    attempts,
                    fixed: false,
                    report,
                };
            };
            if fixed == current {
                return RepairResult {
                    code: current,
                    attempts,
                    fixed: false,
                    report,
                };
            }
            current = fixed;
            attempts += 1;
        }
    }

    /// 执行修复循环（LLM 增强版，异步）
    ///
    /// 规则修复失败时调用 LLM 生成修复。LLM 不可用则退回规则结果。
    pub async fn repair_with_llm(
        &self,
        code: &str,
        lang: Language,
        router: Option<&ModelRouter>,
    ) -> Result<RepairResult> {
        let mut current = code.to_string();
        let mut attempts = 0;

        loop {
            let report = self.gateway.check(&current, lang);
            if !report.has_error() || attempts >= self.config.max_attempts {
                return Ok(RepairResult {
                    code: current,
                    attempts,
                    fixed: !report.has_error(),
                    report,
                });
            }

            // 1. 先尝试零成本规则修复
            if let Some(fixed) = Self::apply_rules(&current, &report) {
                if fixed != current {
                    current = fixed;
                    attempts += 1;
                    continue;
                }
            }

            // 2. 规则修复无效 → LLM 修复
            if self.config.use_llm {
                if let Some(router) = router {
                    if let Ok(fixed) = Self::llm_repair(router, &current, lang, &report).await {
                        if fixed != current {
                            current = fixed;
                            attempts += 1;
                            continue;
                        }
                    }
                }
            }
            return Ok(RepairResult {
                code: current,
                attempts,
                fixed: false,
                report,
            });
        }
    }

    /// 规则修复：基于护栏报告应用确定性文本变换
    ///
    /// 支持：
    /// - 缺失 token（MISSING 节点）→ 在报告位置补全
    fn apply_rules(code: &str, report: &super::GuardReport) -> Option<String> {
        let missing = Regex::new(r"缺少 token: (\S+)").unwrap();
        let mut result = code.to_string();
        let mut applied = false;

        // 从后往前修复，避免位置偏移
        let mut fixes: Vec<(usize, String)> = Vec::new();
        for issue in &report.issues {
            if issue.layer != GuardLayer::Syntax || issue.severity != Severity::Error {
                continue;
            }
            if let Some(caps) = missing.captures(&issue.message) {
                let token = caps[1].to_string();
                if let Some(range) = issue.range {
                    // 插入点：问题行首（该行缺 token）
                    let line_start = line_start_byte(code, range.start_line);
                    let line_end = line_end_byte(code, range.start_line);
                    // 检查该行是否已包含该 token（避免重复插入）
                    let line = &code[line_start..line_end];
                    if line.contains(&token) {
                        continue;
                    }
                    fixes.push((line_end, token));
                }
            }
        }
        // 按字节位置从后往前插入
        fixes.sort_by_key(|(pos, _)| std::cmp::Reverse(*pos));
        for (pos, token) in fixes {
            result.insert_str(pos, &token);
            applied = true;
        }
        if applied {
            Some(result)
        } else {
            None
        }
    }

    /// LLM 修复：将代码与错误反馈给模型，请求修复
    async fn llm_repair(
        router: &ModelRouter,
        code: &str,
        lang: Language,
        report: &super::GuardReport,
    ) -> Result<String> {
        let issues: Vec<String> = report
            .issues
            .iter()
            .filter(|i| i.layer == GuardLayer::Syntax && i.severity == Severity::Error)
            .map(|i| format!("- {}", i.message))
            .collect();
        let prompt = format!(
            "你是代码修复专家。以下 {} 代码存在语法错误，请修复后只输出修复后的完整代码，不要解释：\n\n```\n{code}\n```\n\n错误列表：\n{}\n",
            lang.display(),
            issues.join("\n")
        );
        let messages = vec![
            Message::System {
                content: "你只输出修复后的代码，不要添加任何解释或代码块标记。".to_string(),
                timestamp: chrono::Utc::now(),
            },
            Message::User {
                content: prompt,
                timestamp: chrono::Utc::now(),
            },
        ];
        let resp = tokio::time::timeout(Duration::from_secs(30), router.chat(&messages))
            .await
            .map_err(|_| anyhow::anyhow!("LLM repair timeout"))??;
        Ok(resp.content.trim().to_string())
    }
}

/// 计算某行的起始字节位置
fn line_start_byte(code: &str, line: usize) -> usize {
    let mut pos = 0;
    for _ in 0..line {
        if let Some(offset) = code[pos..].find('\n') {
            pos += offset + 1;
        } else {
            break;
        }
    }
    pos
}

/// 计算某行的结束字节位置（含换行符）
fn line_end_byte(code: &str, line: usize) -> usize {
    let start = line_start_byte(code, line);
    match code[start..].find('\n') {
        Some(offset) => start + offset,
        None => code.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::{GuardIssue, GuardReport, SourceRange};

    #[test]
    fn test_repair_loop_fixes_missing_brace() {
        let gateway = GuardGateway::new(Default::default());
        let loop_engine = RepairLoop::new(gateway, RepairConfig::default());
        // 缺少右括号
        let result = loop_engine.repair("function foo() {\n  return 1;\n", Language::JavaScript);
        assert!(result.fixed, "expected fixed, got: {}", result.code);
        assert!(result.attempts >= 1);
        assert!(!result.report.has_error());
    }

    #[test]
    fn test_repair_loop_clean_code() {
        let gateway = GuardGateway::new(Default::default());
        let loop_engine = RepairLoop::new(gateway, RepairConfig::default());
        let result = loop_engine.repair("const x = 1;\nconsole.log(x);\n", Language::JavaScript);
        assert!(result.fixed);
        assert_eq!(result.attempts, 0);
    }

    #[test]
    fn test_repair_loop_max_attempts() {
        let gateway = GuardGateway::new(Default::default());
        let config = RepairConfig {
            max_attempts: 1,
            ..Default::default()
        };
        let loop_engine = RepairLoop::new(gateway, config);
        // 无法修复的深层错误（乱码）
        let result = loop_engine.repair("function 123 !!! @@", Language::JavaScript);
        assert!(!result.fixed);
        assert!(result.attempts <= 1);
    }

    #[test]
    fn test_line_helpers() {
        let code = "line0\nline1\nline2";
        assert_eq!(line_start_byte(code, 0), 0);
        assert_eq!(line_start_byte(code, 1), 6);
        assert_eq!(line_end_byte(code, 0), 5);
    }

    #[test]
    fn test_line_helpers_edge_cases() {
        let code = "line0\nline1";
        // 末行无换行符 → 结束位置为字符串末尾
        assert_eq!(line_end_byte(code, 1), code.len());
        // 超出范围 → 起始位置停留在最后一行起点
        assert_eq!(line_start_byte(code, 5), 6);
        assert_eq!(line_end_byte(code, 5), code.len());
    }

    fn missing_issue(line: usize, token: &str) -> GuardReport {
        GuardReport {
            passed: false,
            issues: vec![GuardIssue::new(
                GuardLayer::Syntax,
                Severity::Error,
                format!("缺少 token: {token}"),
                Some(SourceRange {
                    start_line: line,
                    start_column: 0,
                    end_line: line,
                    end_column: 0,
                }),
                Some(format!("补全 {token}")),
            )],
        }
    }

    #[test]
    fn test_apply_rules_inserts_missing_token() {
        let code = "const x = 1;\nconsole.log(x)";
        let report = missing_issue(1, ";");
        let fixed = RepairLoop::apply_rules(code, &report).unwrap();
        assert!(fixed.contains("console.log(x);"));
    }

    #[test]
    fn test_apply_rules_skips_when_token_present() {
        let code = "const x = 1;\nconsole.log(x);";
        let report = missing_issue(1, ";");
        assert!(RepairLoop::apply_rules(code, &report).is_none());
    }

    #[test]
    fn test_apply_rules_ignores_warning_severity() {
        let report = GuardReport {
            passed: false,
            issues: vec![GuardIssue::new(
                GuardLayer::Syntax,
                Severity::Warning,
                "缺少 token: ;".to_string(),
                Some(SourceRange {
                    start_line: 0,
                    start_column: 0,
                    end_line: 0,
                    end_column: 0,
                }),
                None,
            )],
        };
        assert!(RepairLoop::apply_rules("code", &report).is_none());
    }

    #[tokio::test]
    async fn test_repair_with_llm_no_router() {
        let gateway = GuardGateway::new(Default::default());
        let loop_engine = RepairLoop::new(gateway, RepairConfig::default());
        let result = loop_engine
            .repair_with_llm(
                "function foo() {\n  return 1;\n",
                Language::JavaScript,
                None,
            )
            .await
            .unwrap();
        assert!(result.fixed);
        assert!(result.attempts >= 1);
    }

    #[tokio::test]
    async fn test_repair_with_llm_clean_code() {
        let gateway = GuardGateway::new(Default::default());
        let loop_engine = RepairLoop::new(gateway, RepairConfig::default());
        let result = loop_engine
            .repair_with_llm(
                "const x = 1;\nconsole.log(x);\n",
                Language::JavaScript,
                None,
            )
            .await
            .unwrap();
        assert!(result.fixed);
        assert_eq!(result.attempts, 0);
    }
}
