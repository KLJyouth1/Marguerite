# ADR：插件动态加载后端选型

- 日期：2026-10-24
- 状态：已采纳（MCP 子进程 v1 已实现）
- 决策人：用户确认「只要好用、稳定、高质量，体积不是约束」

## 背景

`plugins/di.rs` 定义了 `DynamicLoad` trait：

```rust
fn load(&self, entry: &PluginRegistryEntry) -> Result<Arc<dyn ServicePlugin>>;
```

仓内只有 `NoopDynamicLoader`（返回空插件），真实插件无法被加载。此前 16 个
`dsh-*` 插件的清单只是"被装配成槽位"，没有任何原生代码被执行。

## 候选方案

| 方案 | 优点 | 代价 | 失败模式 |
|---|---|---|---|
| **MCP 子进程** | 语言无关；进程隔离，崩溃不拖垮宿主；协议成熟；可真热重载（kill/重启）；零新依赖（复用 `mcp` 客户端语义） | 插件只能暴露 MCP 工具，不能 `provide` 宿主 Rust 服务；每插件一份进程内存 | 子进程不响应 → 超时（已有 1–120s 钳制）；stdout 非 JSON 行 → 跳过 |
| **WASM（wasmtime）** | 沙箱最强；可安全跑不可信代码；可热重载 | 新增重量级依赖；插件需编译 wasm；要设计 WIT 宿主接口；调试门槛高 | 宿主接口不匹配；资源限额需自己实现 |
| **abi_stable 原生 DLL** | 插件可直接提供 Rust 服务（TypeId 级） | 插件必须 Rust；与 ABI 版本强耦合；Windows 上 DLL 卸载不可靠，"热重载"实际接近重启 | ABI 版本漂移 → 加载崩溃（宿主同进程，风险最高） |
| **不接入** | 零风险 | 插件系统停留在声明式，无法执行 | — |

## 决策

选择 **MCP 子进程**，理由按权重排序：

1. **稳定**：崩溃隔离是最强的稳定性保证。abi_stable 崩溃会带走宿主；wasmtime 虽安全，但宿主接口设计不当同样会长期不稳定。
2. **好用**：插件作者可用任意语言；示例插件是纯 Python 标准库（80 行），无需任何 SDK。
3. **可热重载**：`stop` = kill，`start` = 重新拉起，语义清晰且是**真热替换**（不是"重启生效"）。
4. **体积不是约束，但不是理由**：即便不考虑体积，MCP 的协议成熟度与失败模式的可预期性仍优于自研 WASM 宿主接口。

## 落地形态

- 清单新增可选字段 `mcp: { command, args, env, cwd, timeoutMs }`（`plugins/manifest.rs`）。
  刻意**不复用** `entry`：`entry` 是 DLL/WASM 语义（同步），MCP 是长驻进程（异步生命周期）。
- `plugins/mcp_launch.rs`：
  - 行分隔 JSON-RPC；`initialize` → `notifications/initialized` → `tools/list`；
  - **不经 shell**（`Command` 直接 spawn，args 原样传），无命令注入面；
  - 超时 1–120s 钳制；`Drop` 时 `start_kill`，不留僵尸进程；
  - 进程表用 `tokio::sync::Mutex`（可跨 await），另维护**同步快照**给 `plugins.coverage` 读。
- RPC（async-only，登记三同步）：`plugins.mcp.start / stop / list / call`。
- `plugins.coverage` 的每条 `PluginRecord` 增加 `mcp: { declared, command, args, timeoutMs, running, tools, error }`。

## 已知边界（不假装）

1. **不能 provide 宿主服务**：MCP 后端只暴露工具。若将来确有插件要注入 Rust 服务，
   再按本 ADR 的候选表引入第二个后端（WASM 优先于 abi_stable），`DynamicLoad` 仍是扩展点。
2. **快照非实时**：`plugins.coverage` 的 `running/tools` 来自 start/stop/call 时刷新的快照，
   不主动探活；子进程自行退出后需下一次调用或重启才会更新。
3. **串行调用**：`plugins.mcp.call` 期间持有进程表锁，同插件并发调用会排队（MVP 取舍；
   后续可按插件粒度拆分锁）。
4. **无资源限额**：未限制子进程内存/CPU。MCP 进程是**受信插件**模型；不可信插件应等 WASM 后端。

## 验证

- 单测：超时钳制、命令解析、cwd 解析、缺可执行文件、缺工作目录、未运行调用、空列表。
- 端到端：`plugins.mcp.start`（command 覆盖指向示例 server.py）→ `list` → `call echo` → `stop`，
  实测记录见交付报告。
