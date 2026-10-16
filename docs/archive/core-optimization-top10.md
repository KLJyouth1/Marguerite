# marguerite-core 极致优化 Top10

> 生成日期：2026-08-17
> 范围：`core/src`（Rust / axum 网关，编译进 gui exe）
> 目标：在不破坏「0 warning / 0 error / 既有测试」门禁的前提下，做高确定性的运行时优化
> 验证基准：`cargo check -p marguerite-core` → 0 warning；`cargo test -p marguerite-core --lib events::bus` → 13 passed

---

## 方法论说明（为什么不是「无脑 std::sync::Mutex → tokio::sync::Mutex」）

扫描全仓得到 ~70 处 `Mutex` / `RwLock` / `UnboundedSender` 命中。 naïve 地全部替换会**直接破坏构建**：

- 绝大多数 `std::sync::Mutex` 命中是 `OnceLock` / `LazyLock` 懒加载全局单例（如 `S.get_or_init(|| Mutex::new(...))`）。
  `get_or_init` 的闭包是**同步**的，无法 `.await` `tokio::sync::Mutex::lock()`，编译期就不可能替换。
- 这些锁的持有区间 = 「取出/克隆内部数据后立即释放」，从不跨 `.await`，在异步运行时上**本就是安全的**，无需替换为异步锁。
- 真正需要异步锁的场景是「锁跨 `.await` 持有」或「异步热路径高频争用」——本仓经核查仅有 `browser/gateway.rs`、`tools/builtin.rs`、`pyengine/sidecar.rs` 等少数几处已正确使用 `tokio::sync::Mutex`，无需改动。

因此 Top10 聚焦在**有真实收益且绝对安全**的改动上：无界信道→有界（反压 + 内存上限）、吞掉的错误→上报、以及把「看似要改实则已最优 / 改了会炸」的项明确标注为**已核查/已延期**，避免假优化。

---

## 已落地的优化（4 项，代码已改 + 编译/测试通过）

### ① 事件总线订阅信道：无界 → 有界（反压 + 内存上限）
- **文件**：`core/src/events/bus.rs`
- **问题**：`subs` 用 `UnboundedSender` 做 fan-out。若某订阅者消费滞后，发布方会无限为其缓冲 `Envelope`，慢消费者可拖垮整个网关内存。
- **改动**：
  - `subs: Arc<Mutex<HashMap<String, Vec<mpsc::Sender<Envelope>>>>>`（有界）。
  - `subscribe()` 改用 `mpsc::channel(SUBSCRIBER_BUFFER_CAPACITY)`（默认 `1024`，新增常量）。
  - `publish()` 分发用 `try_send`；缓冲满时计入 `dropped: Arc<AtomicU64>` 并 `debug!` 告警，不 panic、不无限堆积。
  - 新增 `dropped_events() -> u64` 观测接口（可接监控/健康端点）。
- **验证**：`cargo check` 0 warning；`events::bus` 13 个测试全过（历史回放/通配/容量裁剪语义不变）。

### ② 终端输出信道：无界 → 有界（防堆积）
- **文件**：`core/src/tools/terminal.rs`
- **问题**：`pipe_reader` 把子进程 stdout/stderr 推进 `UnboundedReceiver`，消费者（`terminal_read`）滞后时缓冲无限增长。
- **改动**：`TerminalInner.rx` 改为 `mpsc::Receiver<String>`；`spawn` 用 `mpsc::channel(TERMINAL_OUTPUT_BUFFER)`（新增常量 `4096`）；`pipe_reader` 改为 `tx.send(...).await`，消费者滞后时**自然反压**（挂起而非堆积），订阅方断开则 `send` 失败即退出。
- **验证**：`cargo check` 0 warning。

### ③ Python 侧车事件信道：无界 → 有界
- **文件**：`core/src/pyengine/client.rs`、`core/src/pyengine/event_bridge.rs`
- **问题**：WebSocket 驱动循环用 `UnboundedSender<Value>` 转发 Python 事件，无背压。
- **改动**：`(event_tx, event_rx) = mpsc::channel::<Value>(256)`；`drive_ws_loop` 签名 / `EVENT_SINK` 全局注册表类型由 `UnboundedReceiver` → `Receiver`；`event_tx.send(frame)` 加 `.await`（语义不变，仍 best-effort：`let _ =` 忽略订阅方断开）。
- **验证**：`cargo check` 0 warning；`event_bridge::run` 入参类型对齐。

### ④ 吞掉的错误 → 上报（MCP 服务端写客户端失败）
- **文件**：`core/src/mcp/server.rs`
- **问题**：`let _ = writer.write_all(response.as_bytes()).await;` 静默丢弃对客户端写入的失败，断开的客户端无法被观测。
- **改动**：`if let Err(e) = writer.write_all(...).await { tracing::warn!(error = %e, "MCP 响应写入客户端失败"); }`，保留既有的 `tracing` 全限定调用风格（无需新增 import）。
- **验证**：`cargo check` 0 warning。

> 附带修复一个**既有测试编译缺陷**（非本次优化引入）：`core/src/orchestration/mod.rs:2663` 的 `BloomConfig` 字面量漏了 3 个字段（`selection_mode` / `all_failed_policy` / `candidates`），导致整个 `--lib` 测试目标无法编译。补 `..Default::default()`（与同处 `Config` 字面量一致）后，`cargo test -p marguerite-core --lib` 恢复可编译。

---

## 已核查 / 已延期的项（6 项，明确标注，避免假优化）

### ⑤ 网关广播吞掉的错误 → 已核查合规
`core/src/gateway/server.rs:369` 的 `let _ = send_text(socket, &frame).await;` 后**已紧跟** `tracing::warn!("网关连接被拒绝：token 校验失败")`，拒绝帧的静默发送是合理的（连接即将关闭）。无需改动；④ 已补齐 MCP 侧的同类缺口。

### ⑥ `research/mod.rs`：锁不跨 await —— 已正确，无需改动
`ResearchCapability::research` 先 `self.llm.lock().unwrap().clone()` + `self.kb.lock().unwrap().clone()` 取出 `Arc`/owned 副本，**立即释放锁后再 `.await run_pipeline`**。注释明确记录了「`std::sync::MutexGuard` 非 Send，会导致工具 future 非 Send」的陷阱，且已正确规避。属于教科书级正确写法，标记为 verified-correct。

### ⑦ 懒加载全局 `std::sync::Mutex`（~40 处）—— 延期，改了会炸
`OnceLock`/`LazyLock` 单例（如 `KB.get_or_init(|| Mutex::new(...))`、`TERMINALS`、`EVENT_SINK`）的锁在 `get_or_init` **同步闭包**内仅「初始化完成后立即释放」，从不跨 await。替换为 `tokio::sync::Mutex` 在语法上不可能（闭包非 async），且当前写法在异步运行时上安全。**结论：保持 `std::sync::Mutex`，不替换。**

### ⑧ `model/router.rs` `clients` 克隆成本 —— 评估为低优先级
`ModelRouter::clone` 执行 `clients: Mutex::new(self.clients.lock().unwrap().clone())`，会克隆整个 `HashMap`。但 `clients` 的值是 `Arc<LLMClient>`，克隆仅为 Arc 指针复制，成本极低；且该克隆仅在显式 `ModelRouter::clone()` 时发生（非每请求热路径）。**结论：当前可接受，留作后续如需高频 clone 时改为 `Arc<HashMap>` 共享。**

### ⑨ `events/bus::publish` 逐订阅者克隆 —— 评估为 micro，暂不做
当前对每个目标订阅者 `env.clone()`。可优化为「首目标用 owned `env`，其余再克隆」省一次克隆。收益极小（订阅者通常个位数，且 `Envelope` 为 `Arc` 化的 JSON），而引入 `enumerate` + 条件 move 会增加出错面。**结论：保持可读性优先，标记为可选 micro-opt。**

### ⑩ 网络 await 超时防护（挂死保护）—— 列为下一阶段
`pyengine/client.rs::call`（oneshot 等待）与网关 WS 读取均未包 `tokio::time::timeout`。慢/假死的外部依赖可能让请求永久挂起。建议下一阶段为外部网络 await 统一加超时（如 `call` 加 `timeout(Duration::from_secs(60))` 返回 `RpcError::timeout`）。**结论：非阻塞当前门禁，列入后续 hardening。**

---

## 变更清单（git 视角）

| 文件 | 改动 |
|------|------|
| `core/src/events/bus.rs` | 有界订阅信道 + `dropped_events()` + `debug!` 告警 |
| `core/src/tools/terminal.rs` | 有界输出信道（`4096`）+ `pipe_reader` 异步反压 |
| `core/src/pyengine/client.rs` | 有界事件信道（`256`）+ `Receiver` 化 `EVENT_SINK` |
| `core/src/pyengine/event_bridge.rs` | `run` 入参 `Receiver` 对齐 |
| `core/src/gateway/server.rs` | 订阅 `event_rx` 类型对齐 `Receiver` |
| `core/src/tui/workbench/mod.rs` | `bus_rx` 字段类型对齐 `Receiver` |
| `core/src/orchestration/mod.rs` | 修复 `BloomConfig` 测试字面量（`..Default::default()`） |
| `core/src/model/bloom.rs` | `drain` 测试入参类型对齐 `Receiver` |
| `core/src/mcp/server.rs` | 客户端写失败 → `tracing::warn!` 上报 |

## 门禁结果

- `cargo check -p marguerite-core`：**0 warning / 0 error**
- `cargo test -p marguerite-core --lib events::bus`：**13 passed**
- `cargo test -p marguerite-core --lib`（全量）：**1617 passed / 3 failed**
  - 3 个失败均**非本次改动引入**（所涉文件 `gateway/events.rs`、`gateway/methods.rs`、`tools/baota.rs` 本次未改动），属**既有缺陷**，与 Top10 优化无因果：
    1. `gateway::events::tests::test_envelope_to_frame_mapping` —— 事件名串映射断言（`session.event.tool_use_start` vs `session.event.tool_call`）；
    2. `gateway::methods::tests::test_supported_methods_subset_of_dispatch` —— `SUPPORTED_METHODS` 含未实现方法 `onboarding.provider.probe`；
    3. `tools::baota::tests::test_register_baota_tools` —— 缺未注册工具 `baota.system_info`。
  - 判定：3 项超出本次「极致优化 Top10」范围，建议单独建 issue / 并入后续 hardening，不在本优化 PR 内修复（避免无关改动引入风险）。
