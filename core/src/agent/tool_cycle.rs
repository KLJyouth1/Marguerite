//! 工具循环
//!
//! 管理工具调用的生命周期，支持顺序执行和并行执行，
//! 提供超时控制、并发限制和结果收集能力。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use tokio::sync::Semaphore;

use crate::agent::tool_cache::ToolCache;
use crate::agent::tool_compress::ToolCompressor;
use crate::mcp::registry::ToolRegistry;

/// 工具执行结果
#[derive(Debug, Clone)]
pub enum ToolResult {
    /// 执行成功
    Success {
        /// 返回数据
        data: String,
        /// 元数据
        metadata: HashMap<String, String>,
    },
    /// 执行错误
    Error {
        /// 错误消息
        message: String,
        /// 错误码
        code: String,
    },
    /// 超时
    Timeout {
        /// 工具名称
        tool_name: String,
        /// 超时时间
        duration: Duration,
    },
}

impl ToolResult {
    /// 判断结果是否为成功
    pub fn is_success(&self) -> bool {
        matches!(self, ToolResult::Success { .. })
    }

    /// 判断结果是否为错误
    pub fn is_error(&self) -> bool {
        matches!(self, ToolResult::Error { .. })
    }

    /// 判断结果是否为超时
    pub fn is_timeout(&self) -> bool {
        matches!(self, ToolResult::Timeout { .. })
    }

    /// 获取成功数据（如果结果是成功）
    pub fn data(&self) -> Option<&str> {
        match self {
            ToolResult::Success { data, .. } => Some(data),
            _ => None,
        }
    }
}

/// 工具调用
#[derive(Debug, Clone)]
pub struct ToolCall {
    /// 工具名称
    pub name: String,
    /// 参数
    pub arguments: HashMap<String, String>,
    /// 超时时间
    pub timeout: Duration,
}

impl ToolCall {
    /// 创建新的工具调用
    pub fn new(name: &str, arguments: HashMap<String, String>) -> Self {
        Self {
            name: name.to_string(),
            arguments,
            timeout: Duration::from_secs(30),
        }
    }

    /// 设置超时
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// 工具循环默认工具缓存容量（FIFO 上限：超出仅淘汰最旧条目、不影响命中正确性）。
const DEFAULT_TOOL_CACHE_CAP: usize = 64;

/// 工具循环
///
/// 管理工具调用的执行生命周期，支持顺序和并行执行策略。
pub struct ToolCycle {
    /// 最大工具循环次数
    max_iterations: u32,
    /// 最大并行工具数
    max_concurrent: usize,
    /// 默认超时时间
    default_timeout: Duration,
    /// 工具注册表
    registry: Arc<ToolRegistry>,
    /// 幂等工具结果缓存：默认构造即注入（W-3，接 P-12 `ToolCache`）。注入后每次
    /// 工具调用同 (tool,args) 命中即返回缓存、不再重跑底层工具；非幂等（写/发/删/建）
    /// 与失败结果不入缓存（诚实，不掩盖瞬时失败）。跨轮/跨会话共享需显式 `with_tool_cache`
    /// 注入同一 `Arc<ToolCache>`。
    tool_cache: Option<Arc<ToolCache>>,
    /// 工具输出压缩器（W-4，接 P-12 `ToolCompressor`）：默认构造即注入。大体积工具输出
    /// （长日志/大 JSON/diff）经确定性压缩后进入上下文，缩小上下文、放大前缀缓存命中率；
    /// 原文按内容指纹存入内存 LRU，模型可经 `retrieve_full` 取回（CCR 可逆）。
    tool_compressor: Option<Arc<ToolCompressor>>,
}

impl ToolCycle {
    /// 创建新的工具循环（默认注入幂等工具结果缓存，W-3）。
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self {
            max_iterations: 10,
            max_concurrent: 5,
            default_timeout: Duration::from_secs(30),
            registry,
            tool_cache: Some(Arc::new(ToolCache::with_default_policy(
                DEFAULT_TOOL_CACHE_CAP,
                None,
            ))),
            tool_compressor: Some(Arc::new(ToolCompressor::default())),
        }
    }

    /// 注入共享幂等工具结果缓存（覆盖默认实例，用于跨轮/跨会话复用同一缓存）。
    pub fn with_tool_cache(mut self, cache: Arc<ToolCache>) -> Self {
        self.tool_cache = Some(cache);
        self
    }

    /// 注入共享工具输出压缩器（覆盖默认实例，用于跨轮/跨会话复用同一压缩器与 CCR 原文库）。
    pub fn with_tool_compressor(mut self, compressor: Arc<ToolCompressor>) -> Self {
        self.tool_compressor = Some(compressor);
        self
    }

    /// 执行工具调用（默认并行执行）
    pub async fn execute(&self, tool_calls: Vec<ToolCall>) -> Result<Vec<ToolResult>> {
        self.execute_parallel(tool_calls).await
    }

    /// 顺序执行工具调用
    ///
    /// 按顺序依次执行每个工具调用，前一个完成后才执行下一个。
    pub async fn execute_sequential(&self, tool_calls: Vec<ToolCall>) -> Result<Vec<ToolResult>> {
        if tool_calls.len() > self.max_iterations as usize {
            return Err(anyhow!(
                "Too many tool calls: {} (max: {})",
                tool_calls.len(),
                self.max_iterations
            ));
        }

        let mut results = Vec::with_capacity(tool_calls.len());

        for call in tool_calls {
            let result = dispatch_cached(
                self.tool_cache.clone(),
                self.tool_compressor.clone(),
                self.registry.clone(),
                &call.name,
                &call.arguments,
                call.timeout,
            )
            .await;
            results.push(result);
        }

        Ok(results)
    }

    /// 并行执行工具调用
    ///
    /// 使用 tokio::spawn 并发执行所有工具调用，通过 Semaphore 控制并发数。
    pub async fn execute_parallel(&self, tool_calls: Vec<ToolCall>) -> Result<Vec<ToolResult>> {
        if tool_calls.len() > self.max_iterations as usize {
            return Err(anyhow!(
                "Too many tool calls: {} (max: {})",
                tool_calls.len(),
                self.max_iterations
            ));
        }

        let semaphore = Arc::new(Semaphore::new(self.max_concurrent));
        let mut handles = Vec::with_capacity(tool_calls.len());

        for call in tool_calls {
            let sem = semaphore.clone();
            let name = call.name.clone();
            let args = call.arguments.clone();
            let timeout = call.timeout;
            let registry = self.registry.clone();
            let cache = self.tool_cache.clone();

            let comp = self.tool_compressor.clone();
            let handle = tokio::spawn(async move {
                let _permit = sem.acquire().await;
                dispatch_cached(cache, comp, registry, &name, &args, timeout).await
            });

            handles.push(handle);
        }

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => results.push(ToolResult::Error {
                    message: format!("Task join error: {}", e),
                    code: "JOIN_ERROR".to_string(),
                }),
            }
        }

        Ok(results)
    }

    /// 单工具调用（带超时）
    async fn call_tool_with_timeout(
        &self,
        name: &str,
        args: &HashMap<String, String>,
        timeout: Duration,
    ) -> ToolResult {
        dispatch_cached(
            self.tool_cache.clone(),
            self.tool_compressor.clone(),
            self.registry.clone(),
            name,
            args,
            timeout,
        )
        .await
    }

    /// 单工具调用（使用默认超时）
    ///
    /// 调用单个工具，带超时控制。
    pub async fn call_tool(&self, name: &str, args: &HashMap<String, String>) -> ToolResult {
        self.call_tool_with_timeout(name, args, self.default_timeout)
            .await
    }

    /// 设置最大循环次数
    pub fn set_max_iterations(&mut self, n: u32) {
        self.max_iterations = n;
    }

    /// 设置最大并行数
    pub fn set_max_concurrent(&mut self, n: usize) {
        self.max_concurrent = n;
    }

    /// 获取最大循环次数
    pub fn max_iterations(&self) -> u32 {
        self.max_iterations
    }

    /// 获取最大并行数
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// 获取默认超时时间
    pub fn default_timeout(&self) -> Duration {
        self.default_timeout
    }
}

/// 内部工具执行函数
///
/// 检查工具是否在注册表中，然后执行。
async fn execute_tool_internal(
    registry: &ToolRegistry,
    name: &str,
    args: &HashMap<String, String>,
) -> Result<ToolResult> {
    // 先检查本地工具
    if registry.has_tool(name) {
        // 本地工具经 get_tool 返回 None，走 execute_local_tool；
        // MCP 工具经 get_tool 命中后走其 execute（与 chain::executor 一致）。
        if let Some(tool) = registry.get_tool(name) {
            return tool.execute(args).await;
        }
        return registry.execute_local_tool(name, args).await;
    }

    Err(anyhow!("Tool '{}' not found in registry", name))
}

/// 经幂等工具缓存分派单次工具调用（W-3，接 P-12 `ToolCache`）。
///
/// - 注入缓存时：相同 (tool,args) 命中即返回缓存结果、不再重跑底层工具；未命中执行
///   `execute_tool_internal` 并按「幂等 + 成功」门控回填缓存（非幂等/失败不缓存，诚实）。
/// - 未注入缓存时：退化为直接超时包装执行，语义与旧 `call_tool` 一致。
/// 若注入了压缩器则压缩工具结果，否则原样返回（确定性、无副作用）。
fn maybe_compress(
    compressor: &Option<Arc<ToolCompressor>>,
    name: &str,
    r: ToolResult,
) -> ToolResult {
    match compressor {
        Some(c) => c.compress(name, &r),
        None => r,
    }
}

async fn dispatch_cached(
    cache: Option<Arc<ToolCache>>,
    compressor: Option<Arc<ToolCompressor>>,
    registry: Arc<ToolRegistry>,
    name: &str,
    args: &HashMap<String, String>,
    timeout: Duration,
) -> ToolResult {
    if let Some(cache) = &cache {
        // 缓存键用独立副本，执行闭包用另一副本，避免与 execute_cached 的 & 借用冲突。
        let key_tool = name.to_string();
        let key_args = args.clone();
        let exec_tool = key_tool.clone();
        let exec_args = key_args.clone();
        let reg = registry.clone();
        let t = timeout;
        let comp = compressor.clone();
        return cache
            .execute_cached(&key_tool, &key_args, move || {
                let reg = reg.clone();
                let exec_args = exec_args.clone();
                let exec_tool = exec_tool.clone();
                let comp = comp.clone();
                async move {
                    let raw = match tokio::time::timeout(
                        t,
                        execute_tool_internal(&reg, &exec_tool, &exec_args),
                    )
                    .await
                    {
                        Ok(Ok(result)) => result,
                        Ok(Err(e)) => ToolResult::Error {
                            message: e.to_string(),
                            code: "EXECUTION_ERROR".to_string(),
                        },
                        Err(_) => ToolResult::Timeout {
                            tool_name: exec_tool.clone(),
                            duration: t,
                        },
                    };
                    maybe_compress(&comp, &exec_tool, raw)
                }
            })
            .await;
    }
    // 无缓存：直接超时包装执行 + 压缩
    let raw = match tokio::time::timeout(timeout, execute_tool_internal(&registry, name, args)).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => ToolResult::Error {
            message: e.to_string(),
            code: "EXECUTION_ERROR".to_string(),
        },
        Err(_) => ToolResult::Timeout {
            tool_name: name.to_string(),
            duration: timeout,
        },
    };
    maybe_compress(&compressor, name, raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::registry::{ToolExecutor, ToolRegistry};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct EchoTool;

    #[async_trait]
    impl ToolExecutor for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echo tool for testing"
        }

        async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
            let msg = args.get("message").cloned().unwrap_or_default();
            Ok(ToolResult::Success {
                data: format!("echo: {}", msg),
                metadata: HashMap::new(),
            })
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {"type": "string"}
                }
            })
        }
    }

    struct SlowTool;

    #[async_trait]
    impl ToolExecutor for SlowTool {
        fn name(&self) -> &str {
            "slow"
        }

        fn description(&self) -> &str {
            "Slow tool for testing timeout"
        }

        async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(ToolResult::Success {
                data: "done".to_string(),
                metadata: HashMap::new(),
            })
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
    }

    struct FailingTool;

    #[async_trait]
    impl ToolExecutor for FailingTool {
        fn name(&self) -> &str {
            "fail"
        }

        fn description(&self) -> &str {
            "Tool that always fails"
        }

        async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
            Ok(ToolResult::Error {
                message: "Intentional failure".to_string(),
                code: "TEST_FAIL".to_string(),
            })
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
    }

    fn setup_registry() -> Arc<ToolRegistry> {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(EchoTool));
        registry.register_local(Box::new(FailingTool));
        Arc::new(registry)
    }

    #[tokio::test]
    async fn test_sequential_execution() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let calls = vec![
            ToolCall::new("echo", {
                let mut args = HashMap::new();
                args.insert("message".to_string(), "hello".to_string());
                args
            }),
            ToolCall::new("echo", {
                let mut args = HashMap::new();
                args.insert("message".to_string(), "world".to_string());
                args
            }),
        ];

        let results = cycle.execute_sequential(calls).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].is_success());
        assert!(results[1].is_success());
        assert_eq!(results[0].data(), Some("echo: hello"));
        assert_eq!(results[1].data(), Some("echo: world"));
    }

    #[tokio::test]
    async fn test_parallel_execution() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let calls = vec![
            ToolCall::new("echo", {
                let mut args = HashMap::new();
                args.insert("message".to_string(), "parallel1".to_string());
                args
            }),
            ToolCall::new("echo", {
                let mut args = HashMap::new();
                args.insert("message".to_string(), "parallel2".to_string());
                args
            }),
            ToolCall::new("echo", {
                let mut args = HashMap::new();
                args.insert("message".to_string(), "parallel3".to_string());
                args
            }),
        ];

        let results = cycle.execute_parallel(calls).await.unwrap();
        assert_eq!(results.len(), 3);
        for result in &results {
            assert!(result.is_success());
        }
    }

    #[tokio::test]
    async fn test_timeout_control() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(SlowTool));
        let registry = Arc::new(registry);

        let mut cycle = ToolCycle::new(registry);
        cycle.set_max_concurrent(1);

        let call = ToolCall::new("slow", HashMap::new()).with_timeout(Duration::from_millis(100));

        let results = cycle.execute_sequential(vec![call]).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_timeout());

        match &results[0] {
            ToolResult::Timeout {
                tool_name,
                duration,
            } => {
                assert_eq!(tool_name, "slow");
                assert!(duration.as_millis() <= 500);
            }
            _ => panic!("Expected Timeout"),
        }
    }

    #[tokio::test]
    async fn test_tool_error() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let call = ToolCall::new("fail", HashMap::new());
        let results = cycle.execute_sequential(vec![call]).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_error());

        match &results[0] {
            ToolResult::Error { message, code } => {
                assert_eq!(message, "Intentional failure");
                assert_eq!(code, "TEST_FAIL");
            }
            _ => panic!("Expected Error"),
        }
    }

    #[tokio::test]
    async fn test_tool_not_found() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let call = ToolCall::new("nonexistent", HashMap::new());
        let results = cycle.execute_sequential(vec![call]).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_error());
    }

    #[tokio::test]
    async fn test_execute_default_parallel() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let calls = vec![ToolCall::new("echo", {
            let mut args = HashMap::new();
            args.insert("message".to_string(), "default".to_string());
            args
        })];

        let results = cycle.execute(calls).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_success());
        assert_eq!(results[0].data(), Some("echo: default"));
    }

    #[tokio::test]
    async fn test_call_tool_success() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let mut args = HashMap::new();
        args.insert("message".to_string(), "direct".to_string());
        let result = cycle.call_tool("echo", &args).await;
        assert!(result.is_success());
        assert_eq!(result.data(), Some("echo: direct"));
    }

    #[tokio::test]
    async fn test_call_tool_error() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let result = cycle.call_tool("fail", &HashMap::new()).await;
        assert!(result.is_error());
        match &result {
            ToolResult::Error { message, code } => {
                assert_eq!(message, "Intentional failure");
                assert_eq!(code, "TEST_FAIL");
            }
            _ => panic!("Expected Error"),
        }
    }

    #[tokio::test]
    async fn test_parallel_timeout() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(SlowTool));
        let registry = Arc::new(registry);

        let mut cycle = ToolCycle::new(registry);
        cycle.set_max_concurrent(1);

        let calls = vec![
            ToolCall::new("slow", HashMap::new()).with_timeout(Duration::from_millis(100)),
            ToolCall::new("slow", HashMap::new()).with_timeout(Duration::from_millis(100)),
        ];

        let results = cycle.execute_parallel(calls).await.unwrap();
        assert_eq!(results.len(), 2);
        for result in &results {
            assert!(result.is_timeout());
        }
        match &results[0] {
            ToolResult::Timeout {
                tool_name,
                duration,
            } => {
                assert_eq!(tool_name, "slow");
                assert!(duration.as_millis() <= 500);
            }
            _ => panic!("Expected Timeout"),
        }
    }

    #[tokio::test]
    async fn test_parallel_error_and_success_mixed() {
        let registry = setup_registry();
        let cycle = ToolCycle::new(registry);

        let calls = vec![
            ToolCall::new("echo", HashMap::new()),
            ToolCall::new("fail", HashMap::new()),
            ToolCall::new("missing", HashMap::new()),
        ];

        let results = cycle.execute_parallel(calls).await.unwrap();
        assert_eq!(results.len(), 3);
        assert!(results[0].is_success());
        assert!(results[1].is_error());
        assert!(results[2].is_error());
    }

    #[tokio::test]
    async fn test_max_iterations_exceeded() {
        let registry = setup_registry();
        let mut cycle = ToolCycle::new(registry);
        cycle.set_max_iterations(1);

        let calls = vec![
            ToolCall::new("echo", HashMap::new()),
            ToolCall::new("echo", HashMap::new()),
        ];

        // 顺序执行超出限制
        let result = cycle.execute_sequential(calls.clone()).await;
        assert!(result.is_err());

        // 并行执行超出限制
        let result = cycle.execute_parallel(calls).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_concurrent_limit() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(EchoTool));
        let registry = Arc::new(registry);

        let mut cycle = ToolCycle::new(registry);
        cycle.set_max_concurrent(2);

        let calls = (0..5)
            .map(|i| {
                ToolCall::new("echo", {
                    let mut args = HashMap::new();
                    args.insert("message".to_string(), format!("msg{}", i));
                    args
                })
            })
            .collect();

        let results = cycle.execute_parallel(calls).await.unwrap();
        assert_eq!(results.len(), 5);
        for result in &results {
            assert!(result.is_success());
        }
    }

    #[test]
    fn test_tool_result_is_success() {
        let success = ToolResult::Success {
            data: "ok".to_string(),
            metadata: HashMap::new(),
        };
        assert!(success.is_success());
        assert!(!success.is_error());
        assert!(!success.is_timeout());
        assert_eq!(success.data(), Some("ok"));
    }

    #[test]
    fn test_tool_result_is_error() {
        let error = ToolResult::Error {
            message: "fail".to_string(),
            code: "ERR".to_string(),
        };
        assert!(error.is_error());
        assert!(!error.is_success());
        assert!(!error.is_timeout());
        assert_eq!(error.data(), None);
    }

    #[test]
    fn test_tool_result_is_timeout() {
        let timeout = ToolResult::Timeout {
            tool_name: "test".to_string(),
            duration: Duration::from_secs(1),
        };
        assert!(timeout.is_timeout());
        assert!(!timeout.is_success());
        assert!(!timeout.is_error());
        assert_eq!(timeout.data(), None);
    }

    #[test]
    fn test_tool_call_with_timeout() {
        let call = ToolCall::new("test", HashMap::new()).with_timeout(Duration::from_secs(10));
        assert_eq!(call.timeout.as_secs(), 10);
        assert_eq!(call.name, "test");
    }

    #[test]
    fn test_tool_cycle_defaults() {
        let registry = Arc::new(ToolRegistry::new());
        let cycle = ToolCycle::new(registry);
        assert_eq!(cycle.max_iterations(), 10);
        assert_eq!(cycle.max_concurrent(), 5);
        assert_eq!(cycle.default_timeout().as_secs(), 30);
    }

    #[test]
    fn test_tool_cycle_setters() {
        let registry = Arc::new(ToolRegistry::new());
        let mut cycle = ToolCycle::new(registry);
        cycle.set_max_iterations(20);
        cycle.set_max_concurrent(10);
        assert_eq!(cycle.max_iterations(), 20);
        assert_eq!(cycle.max_concurrent(), 10);
    }

    /// 计数工具：每次真实执行 +1，用于验证缓存是否跳过重复执行（W-3）。
    struct CountingTool(Arc<AtomicUsize>);

    #[async_trait]
    impl ToolExecutor for CountingTool {
        fn name(&self) -> &str {
            "count"
        }
        fn description(&self) -> &str {
            "counts executions"
        }
        async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ToolResult::Success {
                data: "c".into(),
                metadata: HashMap::new(),
            })
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
    }

    #[tokio::test]
    async fn test_tool_cycle_caches_repeat_calls() {
        // W-3：注入 ToolCache 后，同一 (tool,args) 的两次 call_tool 仅执行底层工具一次
        // （第二次命中缓存，不再重跑）。
        let mut registry = ToolRegistry::new();
        let calls = Arc::new(AtomicUsize::new(0));
        registry.register_local(Box::new(CountingTool(calls.clone())));
        let registry = Arc::new(registry);
        let cache = Arc::new(ToolCache::with_default_policy(8, None));
        let cycle = ToolCycle::new(registry).with_tool_cache(cache.clone());
        let args = HashMap::from([("x".to_string(), "1".to_string())]);

        let _r1 = cycle.call_tool("count", &args).await;
        let _r2 = cycle.call_tool("count", &args).await;

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "相同入参第二次应命中工具缓存，底层工具仅执行 1 次"
        );
        assert!(
            cache.stats().hits >= 1,
            "工具缓存应记录命中，hits={}",
            cache.stats().hits
        );
    }

    #[tokio::test]
    async fn test_tool_cycle_default_injects_cache_and_caches() {
        // W-3 延伸：默认构造的 ToolCycle 即带工具缓存（生产默认）；execute_sequential
        // 重复同参应仅执行底层工具一次（第二次命中缓存）。
        let mut registry = ToolRegistry::new();
        let calls = Arc::new(AtomicUsize::new(0));
        registry.register_local(Box::new(CountingTool(calls.clone())));
        let registry = Arc::new(registry);
        let cycle = ToolCycle::new(registry);
        let cache = cycle
            .tool_cache
            .clone()
            .expect("ToolCycle::new 应默认注入 ToolCache（W-3 未完成时缺失）");

        let args = HashMap::from([("x".to_string(), "1".to_string())]);
        let calls_tc = vec![
            ToolCall::new("count", args.clone()),
            ToolCall::new("count", args),
        ];
        let _ = cycle.execute_sequential(calls_tc).await.unwrap();

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "相同入参顺序两次应仅执行底层工具 1 次"
        );
        assert!(
            cache.stats().hits >= 1,
            "默认工具缓存应记录命中，hits={}",
            cache.stats().hits
        );
    }

    /// 大 JSON 工具经 call_tool 返回时，应被确定性压缩并附指纹（W-4 接线验证）。
    struct BigJsonTool;

    #[async_trait]
    impl ToolExecutor for BigJsonTool {
        fn name(&self) -> &str {
            "bigjson"
        }
        fn description(&self) -> &str {
            "returns large json"
        }
        async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
            // 构造含大数组的对象：超默认预算（2000），且数组截断后显著变短，
            // 验证 W-4 压缩器在 call_tool 链路上真正生效并附指纹。
            let items: Vec<String> = (0..300)
                .map(|i| {
                    format!(
                        "{{\"id\":{},\"name\":\"item number {} with a long descriptive label\"}}",
                        i, i
                    )
                })
                .collect();
            Ok(ToolResult::Success {
                data: format!("{{\"items\":[{}]}}", items.join(",")),
                metadata: HashMap::new(),
            })
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
    }

    #[tokio::test]
    async fn test_tool_cycle_compresses_large_tool_output() {
        let mut registry = ToolRegistry::new();
        registry.register_local(Box::new(BigJsonTool));
        let registry = Arc::new(registry);
        let cycle = ToolCycle::new(registry);
        let comp = cycle
            .tool_compressor
            .clone()
            .expect("ToolCycle::new 应默认注入 ToolCompressor（W-4）");
        let result = cycle.call_tool("bigjson", &HashMap::new()).await;
        assert!(result.is_success());
        let data = result.data().expect("应有数据");
        assert!(
            data.contains("compressed sha256:"),
            "大输出应被压缩并附指纹: {}",
            data
        );
        assert!(comp.stats().compressed >= 1, "压缩器应记录一次压缩");
    }
}
