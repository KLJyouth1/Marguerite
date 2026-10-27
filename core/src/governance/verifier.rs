//! 验证器引擎（B 桶）
//!
//! 给定一个变更与其验收标准（EARS 风格或自然语言），可选运行一条校验命令
//! （如 `cargo test` / 自定义脚本），再调用真实 LLM 后端判定验收标准是否被满足，
//! 产出结构化裁决（`Verdict`）。
//!
//! 命令执行通过 `VerifyRunner` trait 注入：真实环境用 `OsShellRunner`（OS shell），
//! 离线测试用内存 fake。依赖 `model.cloud`（Real）。

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::governance::review::extract_json_object;
use crate::model::llm::LLMClient;
use crate::proc::SilentCommand;
use crate::types::Message;

/// 一次校验命令的执行结果
#[derive(Debug, Clone)]
pub struct VerifyRun {
    /// 退出码是否为 0（命令视作通过）
    pub success: bool,
    /// 标准输出
    pub stdout: String,
    /// 标准错误
    pub stderr: String,
}

/// 校验命令执行器（注入式）
pub trait VerifyRunner: Send + Sync {
    /// 执行给定命令，返回结果（不抛错；执行失败体现在 `success=false`）。
    fn run(&self, command: &str) -> VerifyRun;
}

/// 验收规格
#[derive(Debug, Clone)]
pub struct VerifySpec {
    /// 变更摘要（改了什么）
    pub change_summary: String,
    /// 验收标准列表
    pub acceptance_criteria: Vec<String>,
    /// 可选校验命令（运行期执行以辅助判定，如 `cargo test`）
    pub verify_command: Option<String>,
}

impl VerifySpec {
    pub fn new(change_summary: &str, acceptance_criteria: &[&str]) -> Self {
        Self {
            change_summary: change_summary.to_string(),
            acceptance_criteria: acceptance_criteria.iter().map(|s| s.to_string()).collect(),
            verify_command: None,
        }
    }

    /// 附带校验命令
    pub fn with_command(mut self, command: &str) -> Self {
        self.verify_command = Some(command.to_string());
        self
    }
}

/// 结构化验证裁决
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    /// 验收是否全部满足
    pub satisfied: bool,
    /// 判定推理
    pub reasoning: String,
    /// 支撑证据
    pub evidence: Vec<String>,
}

impl Verdict {
    /// 未满足的验收项（按序号从 criteria 推断，这里仅暴露 satisfied）
    pub fn is_satisfied(&self) -> bool {
        self.satisfied
    }
}

/// 验证器引擎
pub struct Verifier {
    client: LLMClient,
    runner: Box<dyn VerifyRunner>,
}

impl Verifier {
    pub fn new(client: LLMClient, runner: Box<dyn VerifyRunner>) -> Self {
        Self { client, runner }
    }

    /// 执行验证：可选先跑校验命令，再让 LLM 依据全部上下文判定验收。
    pub async fn verify(&self, spec: &VerifySpec) -> Result<Verdict> {
        let command_run = spec.verify_command.as_ref().map(|cmd| self.runner.run(cmd));
        let resp = self
            .client
            .chat_complete(&self.prompt(spec, &command_run))
            .await?;
        parse_verdict(&resp.content, command_run.as_ref())
    }

    fn prompt(&self, spec: &VerifySpec, run: &Option<VerifyRun>) -> Vec<Message> {
        let criteria = spec
            .acceptance_criteria
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{}. {c}", i + 1))
            .collect::<Vec<_>>()
            .join("\n");
        let run_section = match run {
            Some(r) => format!(
                "\n\n校验命令执行结果（退出成功={}）：\nstdout:\n{}\nstderr:\n{}",
                r.success, r.stdout, r.stderr
            ),
            None => String::new(),
        };
        vec![
            Message::System {
                content:
                    "你是严格的验收验证官。依据变更摘要与校验命令结果，判定每条验收标准是否满足，\
                          只输出一个 JSON 对象：{\"satisfied\": bool, \"reasoning\": string, \
                          \"evidence\": [string]}。只要有一条标准未满足就 satisfied=false。\
                          不要输出 JSON 以外的任何内容。"
                        .to_string(),
                timestamp: Utc::now(),
            },
            Message::User {
                content: format!(
                    "变更摘要：{}\n\n验收标准：\n{}\n{}",
                    spec.change_summary, criteria, run_section
                ),
                timestamp: Utc::now(),
            },
        ]
    }
}

/// 解析 LLM 返回的裁决 JSON。
///
/// 容错：截取第一个 `{...}`；解析失败则降级为 `satisfied=false`（宁可拒真，
/// 不可放伪），避免验证器形同虚设。
pub fn parse_verdict(raw: &str, _run: Option<&VerifyRun>) -> Result<Verdict> {
    match extract_json_object(raw) {
        Some(s) => {
            let v: Verdict =
                serde_json::from_str(&s).map_err(|e| anyhow!("验证裁决 JSON 解析失败: {e}"))?;
            Ok(v)
        }
        None => Ok(Verdict {
            satisfied: false,
            reasoning: "验证裁决输出不可解析，已保守判定为未通过".to_string(),
            evidence: vec![raw.trim().to_string()],
        }),
    }
}

/// 真实 OS shell 校验命令执行器（跨平台）。
pub struct OsShellRunner;
impl VerifyRunner for OsShellRunner {
    fn run(&self, command: &str) -> VerifyRun {
        let (shell, flag) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };
        let output = std::process::Command::new(shell)
            .silent()
            .arg(flag)
            .arg(command)
            .output();
        match output {
            Ok(o) => VerifyRun {
                success: o.status.success(),
                stdout: String::from_utf8_lossy(&o.stdout).to_string(),
                stderr: String::from_utf8_lossy(&o.stderr).to_string(),
            },
            Err(e) => VerifyRun {
                success: false,
                stdout: String::new(),
                stderr: format!("命令执行失败: {e}"),
            },
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 完成判定的**可选** LLM 验收（opt-in，默认关闭）
//
// 背景：`Verifier` 此前**零生产调用者** —— 验收层从未在生产路径生效。
// 但 LLM 验收会为每次完成判定增加一次模型调用（成本/延迟），属产品决策，
// 故这里做成**显式开关**：默认关闭时零行为变化；开启后才在确定性门禁放行之后复核。
// ─────────────────────────────────────────────────────────────────────────────

/// 环境变量开关：`MARGUERITE_COMPLETION_VERIFY=1` 启用完成判定的 LLM 验收。
pub const COMPLETION_VERIFY_ENV: &str = "MARGUERITE_COMPLETION_VERIFY";

/// 解析开关真值（`1` / `true` / `yes` / `on`，大小写不敏感；其余一律为关闭）。
fn parse_completion_verify(v: Option<&str>) -> bool {
    matches!(
        v.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

/// 是否启用完成判定的 LLM 验收（opt-in，默认关闭）。
pub fn completion_verify_enabled() -> bool {
    parse_completion_verify(std::env::var(COMPLETION_VERIFY_ENV).ok().as_deref())
}

/// 由应用配置构造验收用的 LLM 配置（无 API key → `None`）。
pub fn llm_config_from(cfg: &crate::persistence::config::Config) -> Option<LLMClient> {
    let m = &cfg.model;
    if m.api_key.trim().is_empty() {
        return None;
    }
    Some(LLMClient::new(crate::model::llm::LLMConfig {
        provider: crate::model::llm::LLMProvider::Custom {
            base_url: m.base_url.clone(),
        },
        model: m.model_name.clone(),
        api_key: m.api_key.clone(),
        base_url: m.base_url.clone(),
        max_tokens: 1024,
        temperature: 0.0,
        timeout_secs: 30,
        thinking: crate::model::thinking::ThinkingEffort::Off,
        context_window: crate::model::thinking::CONTEXT_WINDOW_DEFAULT,
        wire_api: m.wire_api,
    }))
}

/// 完成判定的可选 LLM 验收：仅当开关开启且配置了 API key 时执行。
///
/// **fail-open**：未启用 / 无 key / 网络失败 / 解析失败一律返回 `None` ——
/// 确定性门禁已先行裁决，本层只是可选增强，绝不因基础设施抖动把正常完成误判为失败。
pub async fn verify_completion_if_enabled(task: &str, change_summary: &str) -> Option<Verdict> {
    if !completion_verify_enabled() {
        return None;
    }
    let cfg = crate::persistence::config::Config::load().ok()?;
    let client = llm_config_from(&cfg)?;
    let spec = VerifySpec::new(change_summary, &[task]);
    let verifier = Verifier::new(client, Box::new(OsShellRunner));
    match verifier.verify(&spec).await {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "完成判定的 LLM 验收调用失败（fail-open：不影响确定性门禁裁决）"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 内存 fake runner：按命令内容决定成功/失败
    struct FakeRunner {
        success: bool,
        stdout: String,
    }
    impl VerifyRunner for FakeRunner {
        fn run(&self, _command: &str) -> VerifyRun {
            VerifyRun {
                success: self.success,
                stdout: self.stdout.clone(),
                stderr: String::new(),
            }
        }
    }

    #[test]
    fn parse_verdict_satisfied() {
        let raw = r#"{"satisfied":true,"reasoning":"全部满足","evidence":["测试通过"]}"#;
        let v = parse_verdict(raw, None).unwrap();
        assert!(v.satisfied);
        assert_eq!(v.evidence.len(), 1);
    }

    #[test]
    fn parse_verdict_degrades_on_garbage() {
        // 解析失败 → 保守判未通过
        let v = parse_verdict("无 JSON 输出", None).unwrap();
        assert!(!v.satisfied);
    }

    #[test]
    fn os_shell_runner_echo_works() {
        // 用跨平台可用的命令：`true` 在 POSIX 必定成功；Windows 下用 `cmd /C "exit 0"`
        let runner = OsShellRunner;
        let run = if cfg!(windows) {
            runner.run("exit /B 0")
        } else {
            runner.run("true")
        };
        assert!(run.success, "退出码 0 应判定成功");
    }

    /// opt-in 开关解析：仅 1/true/yes/on 视为开启，其余一律关闭。
    #[test]
    fn completion_verify_flag_parsing() {
        assert!(!parse_completion_verify(None));
        assert!(!parse_completion_verify(Some("")));
        assert!(!parse_completion_verify(Some("0")));
        assert!(!parse_completion_verify(Some("off")));
        assert!(!parse_completion_verify(Some("no")));
        assert!(parse_completion_verify(Some("1")));
        assert!(parse_completion_verify(Some("true")));
        assert!(parse_completion_verify(Some("TRUE")));
        assert!(parse_completion_verify(Some(" yes ")));
        assert!(parse_completion_verify(Some("On")));
    }

    /// 无 API key → 不构造客户端（opt-in 层静默不生效，fail-open）。
    #[test]
    fn llm_config_from_requires_api_key() {
        let mut cfg = crate::persistence::config::Config::default();
        cfg.model.api_key = String::new();
        assert!(llm_config_from(&cfg).is_none(), "无 key 不应构造客户端");
        cfg.model.api_key = "k".into();
        assert!(llm_config_from(&cfg).is_some(), "有 key 应构造客户端");
    }

    async fn mock_server(responses: Vec<(u16, String)>) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let last = responses.last().cloned();
            for (status, body) in responses {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
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
            if let Some((status, body)) = last {
                loop {
                    if let Ok((mut socket, _)) = listener.accept().await {
                        let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
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
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn verifier_satisfied_when_criteria_met() {
        let body = r#"{"choices":[{"message":{"content":"{\"satisfied\":true,\"reasoning\":\"测试全绿\",\"evidence\":[\"cargo test passed\"]}","finish_reason":"stop"}}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let url = mock_server(vec![(200, body.to_string())]).await;
        let config = crate::model::llm::LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let engine = Verifier::new(
            LLMClient::new(config),
            Box::new(FakeRunner {
                success: true,
                stdout: "test result: ok".to_string(),
            }),
        );
        let spec = VerifySpec::new("实现 add 函数", &["While 调用 add(2,3)，系统必须返回 5"])
            .with_command("cargo test");
        let v = engine.verify(&spec).await.unwrap();
        assert!(v.satisfied);
        assert_eq!(v.evidence[0], "cargo test passed");
    }

    #[tokio::test]
    async fn verifier_unsatisfied_when_criteria_fail() {
        let body = r#"{"choices":[{"message":{"content":"{\"satisfied\":false,\"reasoning\":\"缺少错误处理\",\"evidence\":[\"无 try-catch\"]}","finish_reason":"stop"}}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let url = mock_server(vec![(200, body.to_string())]).await;
        let config = crate::model::llm::LLMConfig {
            base_url: url,
            api_key: "k".to_string(),
            timeout_secs: 5,
            ..Default::default()
        };
        let engine = Verifier::new(
            LLMClient::new(config),
            Box::new(FakeRunner {
                success: false,
                stdout: String::new(),
            }),
        );
        let spec = VerifySpec::new("实现 div", &["除零必须返回错误"]);
        let v = engine.verify(&spec).await.unwrap();
        assert!(!v.satisfied);
    }
}
