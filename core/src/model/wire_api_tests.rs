//! `wire_api` 契约测试（PRD: `docs/prd/2026-10-21-wire-api-responses.md`）。
//!
//! 作为 `llm` 的子模块存在（不是 `tests/` 下的集成测试）：这里要断言的是
//! `build_body` / `parse_response` / `parse_responses_stream_chunk` 等**私有**实现，
//! 集成测试够不着它们。为了测试而在公开 API 上加 `_for_test` 包装属于污染，
//! 故用 `#[cfg(test)] #[path]` 挂在模块内。
//!
//! 覆盖：AC1 零回归 / AC2 非流式解析 / AC3 流式事件映射 / AC4 冲突拒绝 /
//! AC5 URL 归一化 / AC6 工具成对。

use super::*;
use crate::model::wire::{responses_url, validate, WireApi};

fn cfg(wire: WireApi, base_url: &str) -> LLMConfig {
    LLMConfig {
        provider: LLMProvider::Custom {
            base_url: base_url.to_string(),
        },
        model: "gpt-6-astra".into(),
        api_key: "test-key".into(),
        base_url: base_url.into(),
        max_tokens: 1234,
        temperature: 0.25,
        timeout_secs: 30,
        thinking: crate::model::thinking::ThinkingEffort::Off,
        context_window: 128_000,
        wire_api: wire,
    }
}

fn user(text: &str) -> Message {
    Message::User {
        content: text.into(),
        timestamp: chrono::Utc::now(),
    }
}

fn system(text: &str) -> Message {
    Message::System {
        content: text.into(),
        timestamp: chrono::Utc::now(),
    }
}

fn tool_def(name: &str) -> ToolDef {
    ToolDef {
        name: name.into(),
        description: "search".into(),
        parameters: serde_json::json!({"type": "object"}),
    }
}

// ── AC1：默认 Chat 零回归 ────────────────────────────────────────────────

#[test]
fn ac1_default_wire_api_is_chat() {
    // 默认值必须是 Chat：改错这一行会让全部存量用户 404。
    assert_eq!(LLMConfig::default().wire_api, WireApi::Chat);
    // 缺字段的反序列化同样落到 Chat（旧 models.json / config.yaml 零迁移）
    let parsed: LLMConfig =
        serde_json::from_str(r#"{"model":"m","base_url":"https://x"}"#).unwrap();
    assert_eq!(parsed.wire_api, WireApi::Chat);
}

#[test]
fn ac1_chat_url_equals_legacy_build_chat_url() {
    // 硬判据：Chat 分派结果必须与"没有 wire_api 时代"的函数逐字相同。
    for base in [
        "https://api.deepseek.com",
        "https://api.deepseek.com/v1",
        "http://127.0.0.1:11434/v1",
        "https://openrouter.ai/api/v1",
    ] {
        let client = LLMClient::new(cfg(WireApi::Chat, base));
        assert_eq!(
            client.chat_url(),
            build_chat_url(base, false),
            "Chat 路径被改动了: base={base}"
        );
    }
    // anthropic 的 chat 形态仍是 /messages（不受 responses 影响）
    let mut c = cfg(WireApi::Chat, "https://api.anthropic.com");
    c.provider = LLMProvider::Anthropic;
    assert!(LLMClient::new(c).chat_url().ends_with("/messages"));
}

#[test]
fn ac1_chat_body_keeps_legacy_shape() {
    let client = LLMClient::new(cfg(WireApi::Chat, "https://api.deepseek.com"));
    let body = client.build_body(&[system("sys"), user("hi")], false, Some(&[tool_def("grep")]));
    // Chat 形态：system 留在 messages[]、上限叫 max_tokens、工具嵌套 function
    assert_eq!(body["max_tokens"], 1234);
    assert!(body.get("max_output_tokens").is_none());
    assert!(body.get("instructions").is_none());
    assert!(body.get("input").is_none());
    assert_eq!(body["messages"][0]["role"], "system");
    assert!(
        body["tools"][0].get("function").is_some(),
        "Chat 工具必须嵌套 function"
    );
}

// ── AC5：URL 归一化 ─────────────────────────────────────────────────────

#[test]
fn ac5_no_double_v1_for_any_base_url_shape() {
    for base in [
        "https://api.openai.com",
        "https://api.openai.com/",
        "https://api.openai.com/v1",
        "https://api.openai.com/v1/",
        "  https://api.openai.com/v1  ",
    ] {
        let url = responses_url(base);
        assert!(!url.contains("/v1/v1"), "base={base} → {url}");
        assert!(url.ends_with("/v1/responses"), "base={base} → {url}");
    }
    // 客户端分派也要走这条归一化
    let client = LLMClient::new(cfg(WireApi::Responses, "https://api.openai.com/v1/"));
    assert_eq!(client.chat_url(), "https://api.openai.com/v1/responses");
}

// ── AC4：冲突组合必须响亮拒绝 ───────────────────────────────────────────

#[test]
fn ac4_responses_with_anthropic_is_rejected_with_both_field_names() {
    let err = validate(WireApi::Responses, true).expect_err("应拒绝 responses + anthropic");
    assert!(err.contains("wire_api"), "{err}");
    assert!(err.contains("anthropic"), "{err}");
    assert!(err.contains("chat"), "须给出修复示例: {err}");
    assert!(validate(WireApi::Chat, true).is_ok());
    assert!(validate(WireApi::Responses, false).is_ok());
}

// ── AC2：非流式解析 ─────────────────────────────────────────────────────

#[test]
fn ac2_parses_output_text_and_function_call() {
    let payload = serde_json::json!({
        "id": "resp_1",
        "model": "gpt-6-astra",
        "output": [
            {
                "type": "message",
                "role": "assistant",
                "content": [
                    { "type": "output_text", "text": "我先看看文件。" },
                    { "type": "output_text", "text": "然后再改。" }
                ]
            },
            {
                "type": "function_call",
                "call_id": "call_abc",
                "name": "read_file",
                "arguments": "{\"path\":\"a.rs\"}"
            }
        ],
        "usage": {
            "input_tokens": 100,
            "output_tokens": 20,
            "total_tokens": 120,
            "input_tokens_details": { "cached_tokens": 40 }
        }
    });

    let client = LLMClient::new(cfg(WireApi::Responses, "https://api.openai.com"));
    let parsed = client
        .parse_response(payload.to_string())
        .expect("应解析成功");

    // 多段 output_text 按顺序拼接，不能只取第一段
    assert_eq!(parsed.content, "我先看看文件。然后再改。");
    assert_eq!(parsed.tool_calls.len(), 1);
    assert_eq!(parsed.tool_calls[0].name, "read_file");
    assert_eq!(parsed.tool_calls[0].arguments, "{\"path\":\"a.rs\"}");
    assert_eq!(parsed.tool_calls[0].id.as_deref(), Some("call_abc"));
    // usage 字段名映射：input→prompt / output→completion / cached→cache_read
    assert_eq!(parsed.usage.prompt_tokens, 100);
    assert_eq!(parsed.usage.completion_tokens, 20);
    assert_eq!(parsed.usage.total_tokens, 120);
    assert_eq!(parsed.usage.cache_read_input_tokens, 40);
}

#[test]
fn ac2_reasoning_summary_becomes_thinking() {
    let payload = serde_json::json!({
        "output": [
            { "type": "reasoning", "summary": [{ "type": "summary_text", "text": "先分解问题" }] },
            { "type": "message", "content": [{ "type": "output_text", "text": "答案" }] }
        ],
        "usage": { "input_tokens": 5, "output_tokens": 3, "total_tokens": 8 }
    });
    let client = LLMClient::new(cfg(WireApi::Responses, "https://api.openai.com"));
    let parsed = client.parse_response(payload.to_string()).unwrap();
    assert_eq!(parsed.content, "答案");
    assert_eq!(parsed.thinking.as_deref(), Some("先分解问题"));
}

#[test]
fn ac2_refusal_is_surfaced_not_dropped() {
    // 拒答必须是可见内容：丢掉它会让上层以为"模型什么都没说"。
    let payload = serde_json::json!({
        "output": [{
            "type": "message",
            "content": [{ "type": "refusal", "refusal": "我不能协助这个请求。" }]
        }],
        "usage": { "input_tokens": 5, "output_tokens": 8, "total_tokens": 13 }
    });
    let client = LLMClient::new(cfg(WireApi::Responses, "https://api.openai.com"));
    let parsed = client.parse_response(payload.to_string()).unwrap();
    assert!(parsed.content.contains("我不能协助"), "{}", parsed.content);
}

#[test]
fn ac2_error_payload_becomes_err_with_original_message() {
    let payload = serde_json::json!({ "error": { "message": "Unsupported model: gpt-9" } });
    let client = LLMClient::new(cfg(WireApi::Responses, "https://api.openai.com"));
    let err = client
        .parse_response(payload.to_string())
        .expect_err("应报错");
    // 必须保留上游原文：否则又是"下游文案掩盖真因"
    assert!(err.to_string().contains("Unsupported model"), "{err}");
}

// ── AC3：流式事件映射 ───────────────────────────────────────────────────

#[test]
fn ac3_stream_events_map_to_internal_chunks() {
    let text = parse_responses_stream_chunk(
        r#"{"type":"response.output_text.delta","delta":"你好"}"#,
        "m",
    )
    .unwrap();
    assert_eq!(text.content.as_deref(), Some("你好"));
    assert!(text.usage.is_none(), "文本增量不该带 usage");

    let reasoning = parse_responses_stream_chunk(
        r#"{"type":"response.reasoning_summary_text.delta","delta":"先想一下"}"#,
        "m",
    )
    .unwrap();
    assert_eq!(reasoning.thinking.as_deref(), Some("先想一下"));

    // 函数调用起始：带 call_id 与 name，index 取 output_index
    let started = parse_responses_stream_chunk(
        r#"{"type":"response.output_item.added","output_index":0,
            "item":{"type":"function_call","call_id":"call_x","name":"grep"}}"#,
        "m",
    )
    .unwrap();
    assert_eq!(started.tool_calls.len(), 1);
    assert_eq!(started.tool_calls[0].id.as_deref(), Some("call_x"));
    assert_eq!(
        started.tool_calls[0]
            .function
            .as_ref()
            .unwrap()
            .name
            .as_deref(),
        Some("grep")
    );

    // 参数增量：只有 arguments
    let args = parse_responses_stream_chunk(
        r#"{"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"q\":"}"#,
        "m",
    )
    .unwrap();
    assert_eq!(
        args.tool_calls[0]
            .function
            .as_ref()
            .unwrap()
            .arguments
            .as_deref(),
        Some("{\"q\":")
    );

    // 完成事件：finish_reason + usage（usage 只在这里出现）
    let done = parse_responses_stream_chunk(
        r#"{"type":"response.completed","response":{"usage":
            {"input_tokens":50,"output_tokens":7,"total_tokens":57}}}"#,
        "m",
    )
    .unwrap();
    assert_eq!(done.finish_reason.as_deref(), Some("stop"));
    let usage = done.usage.expect("completed 必须带 usage");
    assert_eq!(usage.prompt_tokens, 50);
    assert_eq!(usage.completion_tokens, 7);
}

#[test]
fn ac3_unknown_events_are_ignored_not_fatal() {
    // 协议会演进：新增事件类型不该让整条流失败。
    let chunk = parse_responses_stream_chunk(
        r#"{"type":"response.some_future_event","x":1}"#,
        "m",
    )
    .expect("未知事件应被忽略");
    assert!(chunk.content.is_none());
    assert!(chunk.usage.is_none());
}

#[test]
fn ac3_stream_error_events_become_err() {
    let err = parse_responses_stream_chunk(r#"{"type":"error","message":"rate limited"}"#, "m")
        .expect_err("error 事件必须变成 Err");
    assert!(err.to_string().contains("rate limited"), "{err}");

    let failed = parse_responses_stream_chunk(
        r#"{"type":"response.failed","response":{"error":{"message":"bad request"}}}"#,
        "m",
    )
    .expect_err("response.failed 必须变成 Err");
    assert!(failed.to_string().contains("bad request"), "{failed}");
}

// ── AC6：请求体形状 ─────────────────────────────────────────────────────

#[test]
fn ac6_system_promotes_to_instructions() {
    let client = LLMClient::new(cfg(WireApi::Responses, "https://api.openai.com"));
    let body = client.build_body(&[system("你是助手"), system("简洁回答"), user("hi")], false, None);
    assert_eq!(body["instructions"], "你是助手\n\n简洁回答");
    let input = body["input"].as_array().expect("input 必须是数组");
    assert!(
        !input
            .iter()
            .any(|i| i.get("role").and_then(|r| r.as_str()) == Some("system")),
        "system 不该出现在 input[] 里: {input:?}"
    );
    // 上限字段名必须是 max_output_tokens
    assert_eq!(body["max_output_tokens"], 1234);
    assert!(body.get("max_tokens").is_none());
}

#[test]
fn ac6_tool_round_pairs_function_call_with_output() {
    let client = LLMClient::new(cfg(WireApi::Responses, "https://api.openai.com"));
    let messages = vec![
        user("读一下 a.rs"),
        Message::Tool {
            name: "read_file".into(),
            input: "{\"path\":\"a.rs\"}".into(),
            output: "fn main() {}".into(),
            timestamp: chrono::Utc::now(),
            elapsed_ms: None,
        },
    ];
    let body = client.build_body(&messages, false, None);
    let input = body["input"].as_array().unwrap();
    let calls: Vec<_> = input.iter().filter(|i| i["type"] == "function_call").collect();
    let outputs: Vec<_> = input
        .iter()
        .filter(|i| i["type"] == "function_call_output")
        .collect();
    assert_eq!(calls.len(), 1, "应有 1 个 function_call");
    assert_eq!(outputs.len(), 1, "应有 1 个 function_call_output");
    // 成对的关键：call_id 一致，否则服务端拒绝
    assert_eq!(calls[0]["call_id"], outputs[0]["call_id"]);
    assert_eq!(calls[0]["name"], "read_file");
    assert_eq!(outputs[0]["output"], "fn main() {}");
}

#[test]
fn ac6_tools_use_flat_shape() {
    let client = LLMClient::new(cfg(WireApi::Responses, "https://api.openai.com"));
    let body = client.build_body(&[user("hi")], false, Some(&[tool_def("grep")]));
    let tools = body["tools"].as_array().expect("tools 数组");
    assert_eq!(tools[0]["type"], "function");
    // Responses 扁平：name 在顶层；Chat 才是 {"function":{"name":...}}
    assert_eq!(tools[0]["name"], "grep");
    assert!(tools[0].get("function").is_none(), "不该出现嵌套 function");
}

#[test]
fn ac6_stream_flag_is_carried_into_body() {
    // 流式与非流式必须走同一套结构，只差 stream 字段；
    // 否则"非流式能用、流式静默坏掉"这类半修会藏起来。
    let client = LLMClient::new(cfg(WireApi::Responses, "https://api.openai.com"));
    let body = client.build_stream_request_body(&[user("hi")]);
    assert_eq!(body["stream"], true);
    assert!(body.get("input").is_some());
    assert!(body.get("stream_options").is_none(), "Responses 不吃 stream_options");
}

// ── 线上实证（2026-10-21，api.apigoto.com）暴露的兼容层行为 ────────────────
//
// 真实端点实测：同一个 `/v1/responses` 路径，
//   · **流式** → 真 Responses 协议（`object:"response"`、`response.output_text.delta`）；
//   · **非流式** → 返回 **Chat Completions 格式**（`object:"chat.completion"`、`choices[]`）。
//
// 也就是说"支持 Responses"的服务商可能只实现了一半，非流式走了 chat 兼容层。
// 旧实现只认 `output[]`，遇到这种响应会得到**空 content + 全零 usage**——
// 用户看到"模型什么都没说"，用量台账记 0，且**不报错**。这是最坏的一种失败：
// 看起来成功，实际什么都没拿到。
//
// 因此解析必须**按响应体实际形态**分派，而不是按我们请求时声明的协议。
// 这不是投机：请求协议是我们发的，响应形态是上游决定的，两者可能不一致。

#[test]
fn live_compat_layer_chat_shaped_body_is_still_parsed() {
    // 逐字取自线上真实响应（仅裁剪 id / created）。
    let chat_shaped = r#"{
        "id":"resp_0cf0a604ccf581e6","object":"chat.completion","created":1792592095,
        "model":"gpt-6-astra",
        "choices":[{"index":0,"message":{"role":"assistant","content":"收到"},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":12,"completion_tokens":5,"total_tokens":17,
                 "prompt_tokens_details":{"cached_tokens":0}}
    }"#;

    let client = LLMClient::new(cfg(WireApi::Responses, "https://api.apigoto.com"));
    let parsed = client
        .parse_response(chat_shaped.to_string())
        .expect("chat 形态的响应也必须能解析");

    // 旧实现在这里是空串 → 用户看到"模型没回答"却不报错
    assert_eq!(parsed.content, "收到", "不得把 chat 形态响应解析成空答复");
    assert_eq!(parsed.usage.prompt_tokens, 12, "usage 不得丢成全零");
    assert_eq!(parsed.usage.completion_tokens, 5);
}

#[test]
fn live_compat_layer_chat_shaped_stream_is_still_parsed() {
    // 同一端点流式是真 Responses；但若上游某天把流式也降级成 chat 形态，
    // 必须同样能解析，而不是"事件不认识 → 静默无输出"。
    let chat_chunk = r#"{"choices":[{"delta":{"content":"你好"},"finish_reason":null}]}"#;
    let parsed = parse_responses_stream_chunk(chat_chunk, "m")
        .expect("chat 形态的流 chunk 也必须能解析");
    assert_eq!(
        parsed.content.as_deref(),
        Some("你好"),
        "chat 形态流 chunk 不得被当成未知事件丢掉"
    );
}

#[test]
fn live_output_index_has_holes_and_holes_never_become_tool_calls() {
    // 线上实证：`output_index:0` 是 reasoning item，正文在 `output_index:1`。
    // 若工具调用落在 output_index N，累积数组会在 0..N 留下空洞。
    // 这里锁死：空洞不得被下游当成"一个没有名字的工具调用"。
    use super::{accumulate_tool_call, RawFunctionDelta, RawToolCallDelta};
    let mut accum = Vec::new();
    // 模拟工具落在 output_index=3（前面 0/1/2 都是 reasoning/message）
    accumulate_tool_call(
        &mut accum,
        RawToolCallDelta {
            index: Some(3),
            id: Some("call_z".into()),
            function: Some(RawFunctionDelta {
                name: Some("read_file".into()),
                arguments: Some("{}".into()),
            }),
        },
    );
    // 空洞存在（这是 output_index 语义决定的，无法避免）
    assert_eq!(accum.len(), 4);
    // 但转成 ToolCall 时必须只产出 1 个真实调用
    let calls: Vec<_> = accum
        .into_iter()
        .filter_map(|p| {
            p.name.map(|name| {
                (
                    name,
                    p.id.unwrap_or_default(),
                    p.arguments,
                )
            })
        })
        .collect();
    assert_eq!(calls.len(), 1, "空洞不得变成工具调用");
    assert_eq!(calls[0].0, "read_file");
    assert_eq!(calls[0].1, "call_z");
}
