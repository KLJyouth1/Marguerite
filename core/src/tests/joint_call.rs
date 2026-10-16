//! 联合调用（模型路由 → 智能体 → 工具）闭环集成测试。
//!
//! 目标：证明「模型路由 → 智能体 → 工具」可离线串成闭环——
//! - 角色/意图路由解析出的模型名符合规则（`resolve_role_model` + `ModelRouter::select_model`）；
//! - `SubAgentManager::spawn_with_spec` 真实 spawn 子代理并完成任务，结果可取；
//! - 经 mock LLM 返回 `tool_calls`，假工具被真实执行且执行记录可见。
//!
//! 全离线：用本地 mock LLM 监听器（循环应答、回明确 200 状态码），不经系统代理、
//! 不用「连接拒绝」做确定性失败源（见 `crate::tests::mock_http` 基建说明）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::agent::multi_strategy::resolve_role_model;
use crate::agent::subagent::{SubAgentConfig, SubAgentManager, SubAgentStatus};
use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::{ToolExecutor, ToolRegistry};
use crate::model::router::{Complexity, IntentType, ModelRouter};
use crate::persistence::config::Config;

/// 极简假工具：记录执行次数，返回确定性结果。
struct RecordingTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl ToolExecutor for RecordingTool {
    fn name(&self) -> &str {
        "probe"
    }

    fn description(&self) -> &str {
        "极简假工具：记录执行并返回确定性结果"
    }

    async fn execute(&self, args: &HashMap<String, String>) -> anyhow::Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let input = args.get("input").cloned().unwrap_or_default();
        Ok(ToolResult::Success {
            data: format!("probe executed: {}", input),
            metadata: HashMap::new(),
        })
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "input": {"type": "string"}
            }
        })
    }
}

/// 首请求返回 `tool_calls`（触发工具），后续请求返回纯文本终答的 mock LLM 服务器。
///
/// 循环 accept、完整读请求后应答 200——避免「单发 mock 让多轮回路挂死」的已知坑；
/// 用明确状态码而非连接拒绝做确定性失败源（系统代理会把 loopback 代答 502）。
async fn mock_llm_tool_loop(
    tool_calls_body: &str,
    final_body: &str,
    served: Arc<AtomicUsize>,
) -> String {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let tool_calls_body = tool_calls_body.to_string();
    let final_body = final_body.to_string();
    tokio::spawn(async move {
        loop {
            if let Ok((mut socket, _)) = listener.accept().await {
                let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                let n = served.fetch_add(1, Ordering::SeqCst);
                let body = if n == 0 {
                    &tool_calls_body
                } else {
                    &final_body
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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

/// 指向 mock 服务器的离线配置（provider=custom 才会真正采用 base_url）。
fn offline_cfg(url: &str) -> Config {
    Config {
        model: crate::persistence::config::ModelConfig {
            provider: "custom".to_string(),
            model_name: "deepseek-v4-flash".to_string(),
            api_key: "offline-test".to_string(),
            base_url: url.to_string(),
            copilot_model: None,
            timeout_secs: 0,
        },
        ..Default::default()
    }
}

// 首轮 tool_calls 响应：让子代理请求调用 `probe` 工具。
const TOOL_CALLS_BODY: &str = r#"{
  "choices": [{
    "message": {
      "content": null,
      "tool_calls": [{
        "id": "call_probe_0",
        "type": "function",
        "function": {"name": "probe", "arguments": "{\"input\":\"ping\"}"}
      }]
    },
    "finish_reason": "tool_calls"
  }],
  "usage": {"prompt_tokens": 8, "completion_tokens": 4, "total_tokens": 12}
}"#;

// 终答：无 tool_calls，子代理据此收敛并返回该文本。
const FINAL_BODY: &str = r#"{
  "choices": [{"message": {"content": "final-answer-from-tool-loop"},"finish_reason":"stop"}],
  "usage": {"prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8}
}"#;

/// 同步可测部分：角色 → 模型解析与意图/复杂度路由的模型名必须符合规则。
#[test]
fn routing_resolves_models_consistently() {
    // 角色级：内置表把常见角色映射到明确模型偏好
    assert_eq!(resolve_role_model("coder", "fb"), "deepseek-v4-flash");
    assert_eq!(resolve_role_model("reviewer", "fb"), "deepseek-v4-pro");
    assert_eq!(resolve_role_model("unknown-role", "fallback"), "fallback");

    // 意图/复杂度级：默认路由规则（纯解析，不触网）
    let router = ModelRouter::new(&offline_cfg("http://127.0.0.1:1"));
    assert_eq!(
        router
            .select_model(&IntentType::Code, Complexity::Medium)
            .config
            .model,
        "deepseek-v4-flash",
        "中等复杂度代码应路由到 flash"
    );
    assert_eq!(
        router
            .select_model(&IntentType::Code, Complexity::Complex)
            .config
            .model,
        "deepseek-v4-pro",
        "复杂代码应路由到 pro"
    );
}

/// 闭环：mock LLM + 假工具 + SubAgentManager，spawn 子代理（intent/complexity 指定），
/// 断言工具被执行、结果可取、mock 应答轮次正确。
#[tokio::test]
async fn joint_call_routes_spawns_and_executes_tool() {
    // 清除全局缓存，防止先行测试填充 SEMANTIC_LAYER 后 HIT 跳过 mock LLM 调用
    if let Some(layer) = crate::cache::semantic_layer() {
        layer.clear();
    }
    crate::gateway::chat::gateway_prefix_cache().clear();
    let served = Arc::new(AtomicUsize::new(0));
    let tool_calls = Arc::new(AtomicUsize::new(0));
    let url = mock_llm_tool_loop(TOOL_CALLS_BODY, FINAL_BODY, served.clone()).await;

    let mut registry = ToolRegistry::new();
    registry.register_local(Box::new(RecordingTool {
        calls: tool_calls.clone(),
    }));
    let registry = Arc::new(registry);

    let manager = SubAgentManager::new(
        SubAgentConfig::default(),
        registry,
        Arc::new(ModelRouter::new(&offline_cfg(&url))),
    );

    let id = manager
        .spawn_with_spec(
            "调用 probe 工具并返回最终结果",
            "全局上下文",
            "你是 coder 角色专家",
            IntentType::Code,
            Complexity::Medium,
        )
        .await
        .expect("子代理应能 spawn");

    // 等待全部子代理完成（有超时护栏，避免异常挂起）
    let statuses = tokio::time::timeout(Duration::from_secs(10), manager.wait_all())
        .await
        .expect("子代理未在期限内完成")
        .expect("wait_all 应成功");
    assert_eq!(statuses.len(), 1, "应只有一个子代理");

    // 断言 2：子代理完成后 result 可取（真实闭环产出）
    let result = manager.get_result(&id).await.expect("子代理结果应可取");
    assert!(
        result.contains("final-answer-from-tool-loop"),
        "子代理结果应回传工具回路终答，实际: {result}"
    );

    // 断言 3：工具执行记录可见（假工具被真实执行一次）
    assert_eq!(
        tool_calls.load(Ordering::SeqCst),
        1,
        "假工具应被执行且只执行一次"
    );

    // mock 至少应答两轮（首轮 tool_calls + 次轮终答）
    assert!(
        served.load(Ordering::SeqCst) >= 2,
        "mock 应至少应答两轮（tool_calls 与终答）"
    );

    // 状态应收敛为 Completed
    match manager.get_status(&id).await {
        Some(SubAgentStatus::Completed { .. }) => {}
        other => panic!("子代理应收敛为 Completed，实际: {:?}", other),
    }
}
