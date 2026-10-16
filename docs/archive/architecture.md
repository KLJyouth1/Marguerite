# Marguerite 架构

> 极致精简的 Agent 工作台，CLI 为第一公民。

## 总体结构

```
marguerite/
├── core/            # Rust 核心引擎（库 + 二进制）
│   ├── src/
│   │   ├── cli/         # 自研轻量 CLI 路由（argv / route / program）
│   │   ├── agent/       # Agent 运行时（状态机 / 工具循环 / 子代理）
│   │   ├── model/       # LLM 封装（llm / router / dual 双模型）
│   │   ├── intent/      # 意图系统（classifier / router / rules）
│   │   ├── mcp/         # MCP 客户端与服务端（client / server / registry）
│   │   ├── memory/      # 三层记忆（sensory / short_term / long_term）
│   │   ├── guard/       # 幻觉护栏（syntax / semantic / fact / contract / repair）
│   │   ├── context/     # 上下文引擎（paging / cache / budget）
│   │   ├── skills/      # 技能系统（engine / loader / registry）
│   │   ├── plugins/     # 插件系统（轻量契约 + 注册表）
│   │   ├── persistence/ # 持久化（config / session / task_queue）
│   │   ├── tui/         # Ratatui 终端界面（app / chat / input / status）
│   │   ├── types.rs     # 公共类型（Message 等）
│   │   └── lib.rs       # 库入口
│   ├── tests/           # E2E 与性能基准测试
│   └── Cargo.toml
├── gui/             # Tauri GUI 壳（React + Rust 薄封装）
├── skills/          # 内置技能 YAML
├── config.yaml      # 默认配置模板
└── Cargo.toml       # 工作区配置
```

## 分层设计

核心引擎采用「输入 → 意图 → 规划 → 执行 → 质量校验 → 输出」的单向数据流，各层职责单一、可独立替换。

### 1. CLI 路由层（`cli/`）

- `argv.rs` — 自研参数解析器，支持 `-p`、`-f`、`--pipe` 与子命令，不依赖 clap 等重量级库。
- `route.rs` — 命令路由表，注册与分发三种模式：`Interactive`（TUI）、`Direct`（单指令）、`Subcommand`。
- `program.rs` — 主程序：解析参数 → 加载配置 → 初始化日志 → 路由分发。内置 `doctor` / `model` / `mcp` / `skills` / `session` 五个子命令。

### 2. 意图系统（`intent/`）

- `classifier.rs` — 基于关键词 + 正则的本地规则分类器，识别 explain / refactor / fix / test / doc / commit / search 等高频意图，输出置信度；置信度 < 0.6 时降级到 LLM 消歧。
- `router.rs` — 意图路由表：注册 `IntentHandler`，按优先级匹配，实现意图 → 技能映射，支持热加载（save / load / reload）。
- `rules.rs` — 规则引擎，支撑分类器的匹配规则。

### 3. Agent 运行时（`agent/`）

- `runtime.rs` — Agent 状态机 `Init → Ready → Running → Paused → Done → Error`，持有会话上下文，支持中断与恢复（`AtomicBool` + pause/resume）。
- `tool_cycle.rs` — 工具循环：工具调用 → 结果解析 → 归并上下文；并行执行（`tokio::spawn` + `Semaphore`），超时控制（默认 30s）。
- `subagent.rs` — 子代理：生成与销毁、结果归并，支持并发上限与超时。

### 4. 模型层（`model/`）

- `llm.rs` — OpenAI 兼容 API 封装：HTTP 流式响应（`tokio mpsc`）、错误重试（指数退避 1s/2s/4s，最多 3 次）。
- `router.rs` — 模型路由：按任务复杂度在模型间选择。
- `dual.rs` — 双模型协作：规划模型（强推理）生成结构化计划，执行模型（轻量）逐步执行，异常回传重新规划，对用户透明。

### 5. MCP 层（`mcp/`）

- `client.rs` — MCP 客户端：异步连接池（reqwest 复用）、工具发现与缓存（5 分钟 TTL）。
- `server.rs` — MCP 服务端：HTTP JSON-RPC 标准协议，工具暴露与权限控制。
- `registry.rs` — 工具注册表：`ToolExecutor` trait 统一封装，供工具循环调用。

### 6. 记忆层（`memory/`）

- `sensory.rs` — 感知记忆：当前会话上下文，SQLite 快速读写（< 1ms）。
- `short_term.rs` — 短期记忆：FTS5 全文搜索，最近 N 个会话索引。
- `long_term.rs` — 长期记忆：本地特征哈希向量化（无外部 embedding 依赖），余弦相似度检索，自动摘要与索引。
- 分层检索：感知 → 短期 → 长期，带置信度分数与自动降级。

### 7. 质量保障（`guard/`）

三层幻觉护栏 + 自动修复：

- `syntax.rs` — 语法检查：tree-sitter 支持 JS / TS / Python / Rust / Go，AST 错误检测与修复建议。
- `semantic.rs` — 语义检查：未定义变量检测（轻量静态分析），LSP trait 扩展点。
- `fact.rs` — 事实检查：内置 API 用法规则库（regex），可选 MCP 在线查询。
- `contract.rs` — 代码契约：解析 `@pre` / `@post` / `@invariant`，检查实现覆盖。
- `repair.rs` — 自动修复循环：规则修复（最多 2 次）→ LLM 修复（可选）→ 重新过护栏验证。
- `mod.rs` — 护栏网关：可配置开关与阈值，统一入口。

### 8. 上下文引擎（`context/`）

- `paging.rs` — 上下文分页：按语义边界切分（函数 / 类 / 段落），摘要页自动生成，支持展开 / 折叠。
- `cache.rs` — 摘要缓存：LRU + 内容 hash 键，命中率统计。
- `budget.rs` — Token 预算：会话级上限，> 80% 告警（Warning），≥ 100% 超限（Exceeded）触发自动压缩。

### 9. 技能与插件（`skills/` `plugins/`）

- `skills/engine.rs` — 技能引擎：加载 / 卸载、执行沙箱、效果评分。
- `skills/loader.rs` — 本地 YAML 描述解析、远程 GitHub 安装、签名验证。
- `skills/registry.rs` — 技能注册表。
- `plugins/mod.rs` — 插件契约：`meta()` / `on_load()` / `capabilities()`，原生 trait 静态加载 + `Arc` 注册表热插拔。

### 10. 持久化（`persistence/`）

- `config.rs` — 配置管理：读取 `$1marguerite/config.yaml`，不存在时用默认模板初始化。
- `session.rs` — 会话快照保存 / 恢复、列表管理。
- `task_queue.rs` — 持久化任务队列：SQLite，优先级排序、失败重试（最多 2 次）、状态跟踪、后台 worker。

### 11. GUI 壳（`gui/`）

Tauri 应用：Rust 后端薄封装 core 能力（护栏检查 / 代码修复 / 双模型规划），React 前端展示。遵循 PPA 架构：壳层只做薄封装，逻辑全在 core。

## 数据流示例

```
用户输入
  │
  ▼
CLI 路由 (cli/) ── 子命令 → doctor / model / mcp / skills / session
  │ 交互 / 单指令
  ▼
意图分类 (intent/) ── 置信度 < 0.6 → LLM 消歧
  │
  ▼
模型路由 (model/) ── 复杂任务 → 规划模型 → 执行模型
  │
  ▼
Agent 运行时 (agent/) ── 工具循环 → MCP 工具 / 技能
  │
  ▼
质量护栏 (guard/) ── 语法 → 语义 → 事实 → 契约 → 自动修复
  │
  ▼
上下文引擎 (context/) ── 分页 / 缓存 / 预算压缩
  │
  ▼
记忆沉淀 (memory/) ── 感知 → 短期 → 长期
  │
  ▼
输出（TUI / GUI / 管道）
```

## 配置

默认模板见 [config.yaml](../../config.yaml)，首次运行复制到 `$1marguerite/config.yaml`：

| 配置项 | 说明 |
| --- | --- |
| `model.provider` | 模型提供商（如 deepseek） |
| `model.model_name` | 模型名 |
| `model.api_key` | API Key（留空则提示配置） |
| `model.base_url` | OpenAI 兼容 API 地址 |
| `mcp.servers` | MCP 服务器列表 |
| `memory.*` | 记忆存储路径 |
| `skills.skills_dir` | 技能目录 |
| `log.*` | 日志级别与文件 |

## 质量门禁

- L1: `cargo build` 零错误零警告
- L2: `cargo test` 100% 通过
- L3: `cargo clippy` 零 warning
- L4: `cargo audit` 零已知漏洞
- L5: E2E 场景全部通过
- L6: 调用链路基准 < 50ms（不含 LLM 推理）
