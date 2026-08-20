# Marguerite Agent 自优化报告（2026-08-21）

> 目标：直接优化「Agent 自己」的源码核心路径（agent 运行时 / 上下文引擎 / 记忆系统 / 网关对话链路），
> 全程结合联网资料定位可验证的优化点；资料不足处以代码分析 + 门禁验证兜底。
> 门禁结论：cargo check --workspace 零警告；marguerite-core lib 测试 1972 passed + 14 ignored，0 failed（较基线 +5 项回归测试）。

## 一、方法论

- 基线先行：改动前全仓 cargo check --workspace 留档（发现 2 个未使用 import 警告，属零警告门禁违规）。
- 尊重既有结论：docs/core-optimization-top10.md 已明确「无脑 Mutex→tokio Mutex 会炸」等红线，本次不做假优化；
  其 ⑩（网络 await 超时）经复查已在代码中落地（pyengine::client::call 的 call_timeout()、网关 WS 空闲看门狗 180s），不再重复实现。
- 联网资料（用于定位与佐证）：
  - 上下文工程 / 会话记忆管理：OpenAI Cookbook Context Engineering - Short-Term Memory Management（https://developers.openai.com/cookbook/examples/agents_sdk/session_memory）
  - Agent 记忆成本优化六法（检索缓存/分层记忆）：https://mem0.ai/blog/6-techniques-to-cut-ai-agent-memory-cost-beyond-basic-retrieval
  - 本地 prompt caching 降低工具驱动工作流 token：https://www.kerno.io/blog/how-local-prompt-caching-rediuces-tokens-in-tool-driven-llm-workflows
  - SQLite 连接复用/并发：SQLite 官方 PRAGMA data_version 语义（https://www.sqlite.org/pragma.html#pragma_data_version）
  - Rust async 超时与阻塞卸载：https://www.rustfaq.org/en/how-to-handle-timeouts-in-async-rust/（spawn_blocking 卸载阻塞 IO）

## 二、已落地优化（7 项）

### 1. 零警告门禁恢复（agent/trellis.rs）
- 删除未使用的 HashMap / AtomicUsize / Ordering import（基线 check 的 2 个警告清零）。

### 2. 摘要缓存缺陷修复 + O(n)→O(log n) 淘汰（context/cache.rs）
- 缺陷 A（TTL 泄漏）：get 遇过期条目只返回 None 不删除，条目永久占坑，且 put 的 contains_key 挡住同键重写。现在过期即移除、允许重写。
- 缺陷 B（O(n) 淘汰）：evict_one 全表 min_by_key 扫描。改为 BinaryHeap（访问时间 + 代数 gen）维护 LRU，淘汰 O(log n) 摊还；
  堆中陈旧节点经 gen 校验惰性丢弃（防误淘汰刚访问过的条目）。
- 新增 2 个回归测试（过期移除+重写、陈旧堆节点不误淘汰）。

### 3. Agent 运行时消息装配统一（agent/runtime.rs）
- process_input / chat_stream / run_tool_loop / run_tool_loop_stream 四份重复装配逻辑收敛为
  build_model_messages(history, tool_guide)，顺序固定：稳定前缀 → 工具指引 → 经验提示 → 路由策略片段 → 历史。
- 顺带修复两处不一致：非流式工具回路漏装配稳定前缀；流式回路工具指引排在稳定前缀之前（违背 W-6「最前端」契约）。
  统一后四条路径的 System 半区逐字节一致，最大化 KV-cache 前缀复用（P-7 stable 半区）。

### 4. 每轮对话的 SQLite/JSON 重复打开消除（gateway/chat.rs）
- build_evolution()：每轮 open 2 个库 + 建表 DDL → OnceLock 进程单例（连接复用安全：SQLite 语句级读事务可见其他连接提交）。
- 经验台账（memory/experience.db）：每轮 ExperienceLedger::new+init → 按路径缓存的进程单例。
- 召回记忆 KB（claude-mem 四类 JSON）：每轮读盘+解析 → TTL=5s 进程缓存（保证新导入记忆最多延迟 5s 可见，不牺牲新鲜度）。
- sessions.db：resolve_session_project 回退查询与 persist_session 写库共用 OnceLock<Mutex<SessionManager>> 单例；
  并修正潜在死锁（persist_session 原实现持 manager 再调 resolve——两把非重入 Mutex 抢同一单例会自锁；现改为先 resolve 后取写库）。

### 5. 记忆检索向量索引（memory/long_term.rs、memory/experience.rs）
- 原实现每次 search 全表扫描 + 逐行 split/parse 256 维向量（记忆越多越慢，「越学越慢」）。
- 新增进程内惰性索引：首次检索一次性加载解析；本实例 store/record 增量追加（复用刚算出的向量）、upsert 原地替换；
  跨实例/跨进程写经 PRAGMA data_version 失效检测自动重建（其他连接提交必触发；本连接自身写不误触发）。
- 长期记忆 +2 回归测试（增量可见、跨实例可见），经验台账 +1（upsert 后索引反映新正文）。

### 6. 记忆召回脱离 tokio worker（gateway/chat.rs memory_stable_prefix）
- 四路召回（跨项目 / 分层记忆 / claude-mem / 经验台账）原在请求处理线程上同步执行（SQLite + 磁盘 IO 阻塞 worker，
  与本仓已知坑「同步 TcpStream 阻塞 worker」同类）。
- 改为 async fn + tokio::task::spawn_blocking 整体卸载到阻塞池，内部 thread::scope 四路并行；
  结果按固定顺序拼装，稳定前缀内容确定性不变。

### 7. clippy 硬错误修复（kb/hybrid.rs、vision/diff.rs 测试）
- approx_constant（0.7071 → std::f64::consts::FRAC_1_SQRT_2）与 erasing_op（0*8 消零）两处 deny 级错误，恢复 clippy --all-targets 可编译。

## 三、验证

| 门禁 | 结果 |
| --- | --- |
| cargo check -p marguerite-core | 0 warning / 0 error |
| cargo check --workspace | 0 warning / 0 error（core 视角） |
| cargo test -p marguerite-core --lib（全量） | 1972 passed + 14 ignored，0 failed |
| cargo test -p marguerite-core --lib memory:: | 85 passed |
| cargo clippy -p marguerite-core --all-targets | 0 error（85 个 style 类 warning 为工作树内既有未提交 WIP 引入，本次未触碰，见四） |

## 四、遗留（供后续）

1. clippy --all-targets 仍有 85 个 style 类 warning（needless_return / derivable_impl / into_iter 等，66 处重复），
   分布于工作树内既有未提交改动的测试代码，与本次优化文件无关；建议单独清理 PR。
2. AGENTS.md §2 引用的 docs/cli-tui-optimization-plan.md 实际不存在（文档漂移），已回改删除该引用。
3. 更深层优化候选（需压测数据支撑）：reorder_stable_prefix 每请求克隆全量消息（API 形状所限）；
   Orchestrator::analyze 离线分析随任务数线性；fleet 多模型路由的并发预取。