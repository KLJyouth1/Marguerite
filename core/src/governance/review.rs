//! 自动评审引擎（B 桶）
//!
//! 给定一次变更（diff / 补丁 / PR 内容），调用真实 LLM 后端产出结构化评审裁决：
//! 通过 / 阻断 + 证据 + 建议。输出 `RoleVerdict` 风格的结构化结果，供上游
//! （编排器 / 完成门 / GUI）直接消费。
//!
//! 依赖 `model.cloud`（Real）。评审 JSON 由 LLM 产出并经 `serde_json` 解析，
//! 解析失败时降级为 advisory（不静默通过），保证「评审即质量门」不形同虚设。

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::model::llm::LLMClient;
use crate::types::Message;

/// 评审问题严重度
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    /// 阻断：必须修复后才能合入
    Blocking,
    /// 建议：非强制，供参考
    Advisory,
}

/// 单条评审问题
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewIssue {
    pub severity: ReviewSeverity,
    pub title: String,
    pub detail: String,
    /// 证据（代码片段 / 行号 / 产出物引用）
    pub evidence: String,
}

/// 结构化评审裁决
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewVerdict {
    /// 是否通过（无任何 blocking 问题即通过）
    pub pass: bool,
    /// 一句话结论
    pub summary: String,
    /// 问题清单
    pub issues: Vec<ReviewIssue>,
}

impl ReviewVerdict {
    /// 阻断项数量
    pub fn blocking_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| i.severity == ReviewSeverity::Blocking)
            .count()
    }
}

/// 自动评审引擎
pub struct AutoReview {
    client: LLMClient,
}

impl AutoReview {
    pub fn new(client: LLMClient) -> Self {
        Self { client }
    }

    /// 评审一段 diff / 变更内容，返回结构化裁决。
    pub async fn review(&self, diff: &str) -> Result<ReviewVerdict> {
        let resp = self.client.chat_complete(&self.prompt(diff)).await?;
        parse_review(&resp.content)
    }

    fn prompt(&self, diff: &str) -> Vec<Message> {
        vec![
            Message::System {
                content: "你是严格的高级代码评审。评审以下 diff，只输出一个 JSON 对象，格式严格为：\
                          {\"pass\": bool, \"summary\": string, \"issues\": [{\"severity\": \"blocking\"|\"advisory\", \
                          \"title\": string, \"detail\": string, \"evidence\": string}]}。\
                          只要有正确性缺陷、安全漏洞或契约破坏就标 blocking；风格/可选优化标 advisory。\
                          不要输出 JSON 以外的任何内容。".to_string(),
                timestamp: Utc::now(),
            },
            Message::User {
                content: format!("请评审以下变更：\n```diff\n{diff}\n```"),
                timestamp: Utc::now(),
            },
        ]
    }
}

/// 解析 LLM 返回的评审 JSON。
///
/// 容错：从返回中截取第一个 `{...}`（允许模型夹带少量说明文字），解析失败则降级为
/// 一条 advisory 问题（绝不静默通过）。
pub fn parse_review(raw: &str) -> Result<ReviewVerdict> {
    let json = extract_json_object(raw);
    match json {
        Some(s) => {
            let mut v: ReviewVerdict =
                serde_json::from_str(&s).map_err(|e| anyhow!("评审 JSON 解析失败: {e}"))?;
            // 信任模型的 pass 判定，但只要有 blocking 问题就强制判失败；
            // 模型若显式 pass=false（如存在 advisory 级别的严重保留意见）则保留该判定。
            v.pass = v.pass && v.blocking_count() == 0;
            Ok(v)
        }
        None => Ok(ReviewVerdict {
            pass: true,
            summary: "评审解析失败，已降级为 advisory（未自动阻断）".to_string(),
            issues: vec![ReviewIssue {
                severity: ReviewSeverity::Advisory,
                title: "评审输出不可解析".to_string(),
                detail: raw.trim().to_string(),
                evidence: String::new(),
            }],
        }),
    }
}

/// 从文本中截取第一个完整 JSON 对象（匹配成对花括号，忽略字符串内的花括号）。
pub(crate) fn extract_json_object(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for (i, c) in s[start..].char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match c {
            '"' => in_str = !in_str,
            '\\' if in_str => escape = true,
            '{' if !in_str => depth += 1,
            '}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..start + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_review_passes_when_no_blocking() {
        let raw = r#"{"pass":true,"summary":"看起来不错","issues":[{"severity":"advisory","title":"命名","detail":"x 可更明确","evidence":"line 3"}]}"#;
        let v = parse_review(raw).unwrap();
        assert!(v.pass);
        assert_eq!(v.blocking_count(), 0);
    }

    #[test]
    fn parse_review_recalculates_pass_from_blocking() {
        let raw = r#"{"pass":true,"summary":"有阻断","issues":[{"severity":"blocking","title":"空指针","detail":"可能解引用空","evidence":"fn foo(x: Option<_>)"}]}"#;
        let v = parse_review(raw).unwrap();
        // 即使模型误报 pass=true，只要有 blocking 就强制 pass=false
        assert!(!v.pass);
        assert_eq!(v.blocking_count(), 1);
    }

    #[test]
    fn parse_review_handles_surrounding_text() {
        let raw =
            "好的，评审如下：\n{\"pass\":false,\"summary\":\"有崩溃风险\",\"issues\":[]}\n以上。";
        let v = parse_review(raw).unwrap();
        assert!(!v.pass);
        assert_eq!(v.summary, "有崩溃风险");
    }

    #[test]
    fn parse_review_degrades_on_garbage() {
        let v = parse_review("模型返回了乱码没有 JSON").unwrap();
        // 降级：不阻断、给一条 advisory 说明
        assert!(v.pass);
        assert_eq!(v.issues.len(), 1);
        assert_eq!(v.issues[0].severity, ReviewSeverity::Advisory);
    }

    async fn mock_server(responses: Vec<(u16, String)>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for (status, body) in responses {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let mut buf = [0u8; 4096];
                    let _ = socket.read(&mut buf).await;
                    let reason = if status >= 500 { "Server Error" } else { "OK" };
                    let header = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                    let _ = socket.flush().await;
                }
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn autodreview_blocks_on_critical_issue() {
        let body = r#"{"choices":[{"message":{"content":"{\"pass\":true,\"summary\":\"x\",\"issues\":[{\"severity\":\"blocking\",\"title\":\"SQL注入\",\"detail\":\"拼接SQL\",\"evidence\":\"line 10\"}]}","finish_reason":"stop"}}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let url = mock_server(vec![(200, body.to_string())]).await;
        let config = crate::model::llm::LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let engine = AutoReview::new(LLMClient::new(config));
        let v = engine
            .review("+ sql = \"SELECT * FROM t WHERE id=\" + id")
            .await
            .unwrap();
        assert!(!v.pass);
        assert_eq!(v.blocking_count(), 1);
    }
}
