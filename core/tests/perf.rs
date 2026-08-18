//! 性能基准测试：调用链路 < 50ms（不含 LLM 推理）
//!
//! 覆盖核心热路径：意图分类 → 护栏检查 → 上下文分页 → 记忆检索。
//! 先预热（懒初始化如 tree-sitter parser、SQLite 建表），再计时求均值。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use marguerite_core::context::paging::{PagingEngine, PagingOptions};
use marguerite_core::guard::syntax::Language;
use marguerite_core::guard::{GuardConfig, GuardGateway};
use marguerite_core::intent::classifier::Classifier;
use marguerite_core::memory::long_term::LongTermMemory;
use marguerite_core::memory::sensory::SensoryMemory;
use marguerite_core::types::Message;

/// 调用链路上限：50ms（不含 LLM 推理）
const CHAIN_BUDGET_MS: u128 = 50;
/// 计时轮数
const ITERATIONS: u32 = 200;

/// 完整调用链：意图 → 护栏 → 分页 → 记忆检索
fn run_chain(
    classifier: &Classifier,
    gateway: &GuardGateway,
    paging: &PagingEngine,
    ltm: &LongTermMemory,
    sensory: &mut SensoryMemory,
) {
    // 意图分类（模拟用户高频指令）
    let _ = classifier.classify("请解释一下这个函数的作用并修复其中的 bug");
    // 护栏检查（模拟模型生成代码回灌）
    let code = "function add(a, b) {\n  return a + b;\n}\n";
    let _ = gateway.check(code, Language::JavaScript);
    // 上下文分页（模拟超长会话）
    let big = "x".repeat(6_000);
    let messages = vec![Message::User {
        content: big,
        timestamp: chrono::Utc::now(),
    }];
    let pages = paging.paginate(&messages);
    let _ = pages;
    // 记忆检索（模拟分层记忆查询）
    sensory
        .add_message(Message::User {
            content: "项目约定使用 tokio 异步运行时".to_string(),
            timestamp: chrono::Utc::now(),
        })
        .unwrap();
    let _ = ltm.search("tokio", 3);
}

#[test]
fn perf_call_chain_under_50ms() {
    // 初始化（含懒初始化，不计入计时）
    let classifier = Classifier::new();
    let gateway = GuardGateway::new(GuardConfig::default());
    let paging = PagingEngine::new(PagingOptions::default());

    let path = temp_db();
    let _ = std::fs::remove_file(&path);
    let mut ltm = LongTermMemory::new(&path);
    ltm.init().unwrap();
    let mut sensory = SensoryMemory::new("perf-session");

    // 预热：懒初始化（parser、SQLite 连接、FTS 等）
    for _ in 0..10 {
        run_chain(&classifier, &gateway, &paging, &ltm, &mut sensory);
    }

    // 计时
    let mut total = Duration::ZERO;
    let mut worst = Duration::ZERO;
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        run_chain(&classifier, &gateway, &paging, &ltm, &mut sensory);
        let elapsed = start.elapsed();
        total += elapsed;
        worst = worst.max(elapsed);
    }

    let _ = std::fs::remove_file(&path);
    let avg_ms = total.as_millis() / ITERATIONS as u128;
    println!(
        "调用链路: 平均 {:.3}ms, 最差 {:.3}ms (预算 {CHAIN_BUDGET_MS}ms)",
        avg_ms as f64,
        worst.as_micros() as f64 / 1000.0
    );
    assert!(
        avg_ms < CHAIN_BUDGET_MS,
        "调用链路平均 {avg_ms}ms 超出预算 {CHAIN_BUDGET_MS}ms"
    );
}

/// 临时数据库路径
fn temp_db() -> PathBuf {
    std::env::temp_dir().join(format!("marguerite_perf_ltm_{}.db", std::process::id()))
}
