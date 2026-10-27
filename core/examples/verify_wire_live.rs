// 真实端点端到端验证（临时交付物，验证后可删）。
//
// 目的：把 AC2/AC3 从"构造载荷级"提到"线上级"——
// 用**生产同一份** LLMClient 代码，向真实 Responses 端点发请求，
// 断言正文与用量都真的拿到了。
//
// 密钥只从环境变量读，绝不写进文件。

use marguerite_core::model::llm::{LLMClient, LLMConfig, LLMProvider};
use marguerite_core::model::thinking::ThinkingEffort;
use marguerite_core::model::wire::WireApi;
use marguerite_core::types::Message;

fn config(wire: WireApi) -> LLMConfig {
    LLMConfig {
        provider: LLMProvider::Custom {
            base_url: "https://api.apigoto.com/v1".into(),
        },
        model: "gpt-6-astra".into(),
        api_key: std::env::var("APIGOTO_KEY").expect("APIGOTO_KEY 未设置"),
        base_url: "https://api.apigoto.com/v1".into(),
        max_tokens: 128,
        temperature: 0.3,
        timeout_secs: 120,
        thinking: ThinkingEffort::Off,
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

#[tokio::main]
async fn main() {
    let mut failures = 0;

    // ── 1. Responses 非流式 ──
    // 线上实证：该端点非流式会走 chat 兼容层，返回 choices[]。
    // 这正是本轮修复要覆盖的场景：必须仍能拿到正文与用量。
    println!("=== [1/3] Responses 非流式（端点降级为 chat 形态）===");
    let client = LLMClient::new(config(WireApi::Responses));
    match client.chat_complete(&[user("只回复两个字：收到")]).await {
        Ok(r) => {
            println!("  content = {:?}", r.content);
            println!(
                "  usage   = prompt={} completion={} total={}",
                r.usage.prompt_tokens, r.usage.completion_tokens, r.usage.total_tokens
            );
            if r.content.trim().is_empty() {
                println!("  ✗ FAIL：正文为空（这正是修复前的静默失败形态）");
                failures += 1;
            } else if r.usage.total_tokens == 0 {
                println!("  ✗ FAIL：usage 全零（用量台账会记 0）");
                failures += 1;
            } else {
                println!("  ✓ PASS");
            }
        }
        Err(e) => {
            println!("  ✗ FAIL：{e}");
            failures += 1;
        }
    }

    // ── 2. Responses 流式 ──
    // 用一条**必须逐字复述**的提示，这样"解析截断"与"模型自己少说了"可以区分开：
    // 若原始 SSE 有 N 段 delta 而客户端只拼出末尾，就是解析问题。
    println!("\n=== [2/3] Responses 流式（真 Responses 事件）===");
    let client = LLMClient::new(config(WireApi::Responses));
    match client
        .chat_complete_stream(&[user("请完整输出这一行，不要省略：一、二、三、四、五")])
        .await
    {
        Ok(mut rx) => {
            let mut text = String::new();
            let mut usage = None;
            let mut chunks = 0usize;
            while let Some(item) = rx.recv().await {
                match item {
                    Ok(c) => {
                        chunks += 1;
                        if let Some(d) = c.content {
                            text.push_str(&d);
                        }
                        if let Some(u) = c.usage {
                            usage = Some(u);
                        }
                    }
                    Err(e) => {
                        println!("  ✗ 流内错误：{e}");
                        failures += 1;
                        break;
                    }
                }
            }
            println!("  chunks  = {chunks}");
            println!("  content = {:?}", text);
            match &usage {
                Some(u) => println!(
                    "  usage   = prompt={} completion={} total={}",
                    u.prompt_tokens, u.completion_tokens, u.total_tokens
                ),
                None => println!("  usage   = （未收到）"),
            }
            if text.trim().is_empty() {
                println!("  ✗ FAIL：流式正文为空");
                failures += 1;
            } else if usage.is_none() {
                println!("  ✗ FAIL：流式未记 usage（成本不可见）");
                failures += 1;
            } else {
                println!("  ✓ PASS");
            }
        }
        Err(e) => {
            println!("  ✗ FAIL：{e}");
            failures += 1;
        }
    }

    // ── 3. 对照组：显式 chat 协议打同一端点 ──
    // 证明 WireApi::Chat 路径在真实端点上仍然正常（零回归的线上级证据）。
    println!("\n=== [3/3] 对照组：WireApi::Chat 打 /chat/completions ===");
    let mut c = config(WireApi::Chat);
    c.base_url = "https://api.apigoto.com/v1".into();
    let client = LLMClient::new(c);
    match client.chat_complete(&[user("只回复两个字：收到")]).await {
        Ok(r) => {
            println!("  content = {:?}", r.content);
            println!("  usage   = total={}", r.usage.total_tokens);
            if r.content.trim().is_empty() {
                println!("  ✗ FAIL：chat 基线路径正文为空");
                failures += 1;
            } else {
                println!("  ✓ PASS");
            }
        }
        Err(e) => {
            println!("  ✗ FAIL：{e}");
            failures += 1;
        }
    }

    println!("\n=== 结论：{failures} 个失败 ===");
    std::process::exit(if failures == 0 { 0 } else { 1 });
}
