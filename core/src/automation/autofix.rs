//! 自动修复引擎（B 桶）
//!
//! 给定一个错误上下文（出错文件、错误信息、可选的相关代码与错误栈），调用真实
//! LLM 后端生成修复后的完整文件内容，经注入式 `FixApplier` 应用，并产出结构化
//! 裁决（RoleVerdict 风格：applied / explanation / raw）。
//!
//! 应用动作通过 `FixApplier` trait 注入，使引擎在离线（无 API Key）环境下也能
//! 用 mock applier 做单元测试；真实环境注入「写文件 + 护栏校验」的 applier 即可。
//! 依赖 `model.cloud`（Real）作为 LLM 后端，不再硬阻塞于「model.* 全 Narrative」。

use anyhow::Result;
use chrono::Utc;

use crate::model::llm::LLMClient;
use crate::types::Message;

/// 待修复的错误上下文
#[derive(Debug, Clone)]
pub struct FixContext {
    /// 出错文件（相对或绝对路径，仅用于上下文与展示）
    pub file: String,
    /// 错误信息（编译器报错 / 测试失败 / 运行时异常等）
    pub error: String,
    /// 相关代码（可选，提供上下文以生成更精准修复）
    pub code: Option<String>,
    /// 错误栈（可选）
    pub trace: Option<String>,
}

impl FixContext {
    /// 最简构造：仅文件 + 错误
    pub fn new(file: &str, error: &str) -> Self {
        Self {
            file: file.to_string(),
            error: error.to_string(),
            code: None,
            trace: None,
        }
    }

    /// 附带相关代码
    pub fn with_code(mut self, code: &str) -> Self {
        self.code = Some(code.to_string());
        self
    }

    /// 附带错误栈
    pub fn with_trace(mut self, trace: &str) -> Self {
        self.trace = Some(trace.to_string());
        self
    }
}

/// 修复应用器（注入式）
///
/// 真实环境应实现「写文件 + 护栏校验」逻辑（对照 `core.apply` 能力：改动经
/// `sec.guardrail` 校验后才落盘）；测试环境用内存/记录型 applier 替代。
pub trait FixApplier: Send + Sync {
    /// 应用修复后的完整文件内容。返回是否成功落盘。
    fn apply(&self, file: &str, new_content: &str) -> Result<()>;
}

/// 自动修复结果
#[derive(Debug, Clone)]
pub struct FixOutcome {
    /// 修复是否已被应用器成功落盘
    pub applied: bool,
    /// 目标文件
    pub file: String,
    /// 修复要点说明（来自 LLM）
    pub explanation: String,
    /// 解析出的新文件内容（若 LLM 返回了代码块）
    pub new_content: Option<String>,
    /// LLM 原始返回（便于审计与调试）
    pub raw: String,
}

/// 自动修复引擎
pub struct AutoFix {
    client: LLMClient,
    applier: Box<dyn FixApplier>,
}

impl AutoFix {
    /// 构造：注入 LLM 客户端与修复应用器
    pub fn new(client: LLMClient, applier: Box<dyn FixApplier>) -> Self {
        Self { client, applier }
    }

    /// 执行一次自动修复：调用 LLM 生成修复 → 解析 → 应用。
    pub async fn fix(&self, ctx: &FixContext) -> Result<FixOutcome> {
        let resp = self.client.chat_complete(&self.prompt(ctx)).await?;
        let (new_content, explanation) = parse_fix(&resp.content);
        let applied = if let Some(content) = &new_content {
            self.applier.apply(&ctx.file, content).is_ok()
        } else {
            false
        };
        Ok(FixOutcome {
            applied,
            file: ctx.file.clone(),
            explanation,
            new_content,
            raw: resp.content,
        })
    }

    fn prompt(&self, ctx: &FixContext) -> Vec<Message> {
        let code_block = ctx
            .code
            .clone()
            .map(|c| format!("\n\n相关代码：\n```\n{c}\n```"))
            .unwrap_or_default();
        let trace_block = ctx
            .trace
            .clone()
            .map(|t| format!("\n\n错误栈：\n{t}"))
            .unwrap_or_default();
        let user = format!(
            "请修复文件 {}\n错误信息：{}\n{}{}",
            ctx.file, ctx.error, code_block, trace_block
        );
        vec![
            Message::System {
                content: "你是资深软件工程师。根据错误上下文修复代码，只输出修复后的【完整文件内容】，\
                          放在一个 ``` 代码块中。代码块之后用一两句话简述修复要点。不要输出多余解释、不要追问。"
                    .to_string(),
                timestamp: Utc::now(),
            },
            Message::User {
                content: user,
                timestamp: Utc::now(),
            },
        ]
    }
}

/// 从 LLM 返回中解析出修复内容与说明。
///
/// 规则：取第一个 ``` 围栏代码块作为新文件内容；代码块之外的文本作为说明。
/// 若没有任何代码块，则视为未产出可用修复，`new_content = None`。
pub fn parse_fix(raw: &str) -> (Option<String>, String) {
    let start = match raw.find("```") {
        Some(s) => s,
        None => return (None, raw.trim().to_string()),
    };
    let rest = &raw[start + 3..];
    // 跳过首行语言标签
    let after_opening = match rest.find('\n') {
        Some(nl) => &rest[nl + 1..],
        None => rest,
    };
    match after_opening.find("```") {
        Some(end) => {
            let content = after_opening[..end].trim_end().to_string();
            // 说明 = 代码块之前 + 之后的文本
            let before = raw[..start].trim().to_string();
            let after = &after_opening[end + 3..];
            let after = after.trim().to_string();
            let explanation = match (before.is_empty(), after.is_empty()) {
                (true, true) => String::new(),
                (false, true) => before,
                (true, false) => after,
                (false, false) => format!("{before}\n{after}"),
            };
            (Some(content), explanation)
        }
        None => (None, raw.trim().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 内存记录型 applier，用于离线测试
    struct RecordingApplier {
        applied: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
    }

    impl FixApplier for RecordingApplier {
        fn apply(&self, file: &str, new_content: &str) -> Result<()> {
            self.applied
                .lock()
                .unwrap()
                .push((file.to_string(), new_content.to_string()));
            Ok(())
        }
    }

    /// 简易 mock HTTP 服务器：依次返回给定响应
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

    fn fix_llm_body(new_content: &str, note: &str) -> String {
        format!(
            "{{\"choices\":[{{\"message\":{{\"content\":\"{note}\\n```\\n{new_content}\\n```\\n\",\"finish_reason\":\"stop\"}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}}}"
        )
    }

    #[test]
    fn parse_fix_extracts_code_block() {
        let raw = "已修复。\n```\nfn main() {}\n```\n这是要点说明";
        let (content, explanation) = parse_fix(raw);
        assert_eq!(content.as_deref(), Some("fn main() {}"));
        assert!(explanation.contains("已修复"));
        assert!(explanation.contains("要点说明"));
    }

    #[test]
    fn parse_fix_no_block_returns_none() {
        let raw = "无法修复，请提供更多上下文";
        let (content, explanation) = parse_fix(raw);
        assert!(content.is_none());
        assert_eq!(explanation, raw);
    }

    #[test]
    fn parse_fix_skips_lang_tag() {
        let raw = "```rust\nlet x = 1;\n```";
        let (content, _) = parse_fix(raw);
        assert_eq!(content.as_deref(), Some("let x = 1;"));
    }

    #[tokio::test]
    async fn autofix_applies_generated_fix() {
        let new_src = "fn add(a: i32, b: i32) -> i32 { a + b }";
        let body = fix_llm_body(new_src, "补上返回类型与加号");
        let url = mock_server(vec![(200, body)]).await;
        let config = crate::model::llm::LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let recorder = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let applier = RecordingApplier {
            applied: recorder.clone(),
        };
        let engine = AutoFix::new(client, Box::new(applier));
        let ctx = FixContext::new("src/math.rs", "missing return type");
        let outcome = engine.fix(&ctx).await.unwrap();
        assert!(outcome.applied, "修复应被成功应用");
        assert_eq!(outcome.new_content.as_deref(), Some(new_src));
        let recorded = recorder.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "src/math.rs");
        assert_eq!(recorded[0].1, new_src);
    }

    #[tokio::test]
    async fn autofix_no_code_block_does_not_apply() {
        let body = "{\"choices\":[{\"message\":{\"content\":\"我无法修复，缺少上下文\",\"finish_reason\":\"stop\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}";
        let url = mock_server(vec![(200, body.to_string())]).await;
        let config = crate::model::llm::LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let client = LLMClient::new(config);
        let recorder = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let engine = AutoFix::new(
            client,
            Box::new(RecordingApplier {
                applied: recorder.clone(),
            }),
        );
        let outcome = engine.fix(&FixContext::new("x.rs", "err")).await.unwrap();
        assert!(!outcome.applied);
        assert!(outcome.new_content.is_none());
        assert!(recorder.lock().unwrap().is_empty());
    }
}
