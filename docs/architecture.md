
# Marguerite 架构

> 版本 v1.1 · 2026-09-03（M0/B1 文档收敛重写）
> 本文档与 `core/src/cli/program.rs` 路由注册表、`core/src/cli/argv.rs::SUBCOMMANDS` 保持严格一致（`scripts/check-doc-cli-drift.py` 门禁覆盖）。

## 总体结构

Marguerite 是「Rust 核心 + 多端壳层 + Python 侧车」的大型单仓：

```
marguerite/
├── core/             # Rust 核心引擎（库 + 二进制）
│   ├── src/
│   │   ├── cli/          # 轻量 CLI 路由（argv / route / program / dslash / output）
│   │   ├── agent/        # Agent 运行时（状态机 / 工具循环 / 子代理 / 团队 / 目标循环）
│   │   ├── model/        # LLM 封装（llm / router / dual 双模型）
│   │   ├── intent/       # 意图系统（classifier / router / rules）
│   │   ├── mcp/          # MCP 客户端与服务端（client / server / registry）
│   │   ├── memory/       # 三层记忆（sensory / short_term / long_term）
│   │   ├── guard/        # 幻觉护栏（syntax / semantic / fact / contract / repair）
│   │   ├── context/      # 上下文引擎（paging / cache / budget）
│   │   ├── skills/       # 技能系统（engine / loader / registry）
│   │   ├── plugins/      # 插件系统（轻量契约 + 注册表）
│   │   ├── persistence/  # 持久化（config / session / task_queue）
│   │   ├── browser/      # 真实浏览器 CDP 操控（gateway / mod）
│   │   ├── automation/   # 自动化/调度（host_board 等）
│   │   ├── chain/        # 链式执行（executor 等）
│   │   ├── cache/        # KV/语义缓存（mod / semantic）
│   │   ├── tui/          # Ratatui 终端界面（App / WorkbenchApp）
│   │   ├── types.rs      # 公共类型（Message 等）
│   │   └── lib.rs        # 库入口
│   ├── tests/            # E2E 与性能基准测试
│   └── Cargo.toml
├── webui/           # Vue3 控制台（Pinia / vue-i18n / vue-router），含侧栏契约测试
├── gui/             # Tauri GUI 壳（React 18 前端 + Rust 薄封装）
├── tui/             # 终端 UI 相关（core 内嵌 TUI 亦复用）
├── pyengine/        # Python 侧车能力
├── workbench/       # 工作台相关
├── skills/          # 内置技能 YAML
├── third_party/     # 第三方引入
├── docs/            # 文档（顶层为当前权威；历史/孤本在 docs/archive/）
├── config.yaml      # 默认配置模板
└── Cargo.toml       # 工作区配置
```

## CLI 路由层（`core/src/cli/`）

- `argv.rs` — 自研参数解析器（`-p` / `-f` / `--pipe` / 子命令），不依赖 clap；`SUBCOMMANDS: &[&str]` 为权威子命令清单。
- `route.rs` — `RouteTable` 注册表：`register(name, handler, desc)` 将子命令映射到处理函数；分发三种模式 `Interactive`（TUI）/ `Direct`（单指令）/ `Subcommand`。
- `program.rs` — 主程序 `Program::run()`：解析参数 → 顶层发现（`--version` / `--help` / 未知参数）→ 加载配置 → 初始化日志 → 路由分发。
- `dslash.rs` — 工作台 `/` 斜杠命令解析。
- `output.rs` — 统一输出（JSON 成功/错误、退出码）。

### 权威子命令表（19 个，与 `program.rs` 注册一致）

| 子命令 | 处理函数 | 说明 |
| --- | --- | --- |
| `doctor` | `doctor_handler` | 系统诊断 |
| `model` | `model_handler` | 模型管理 |
| `mcp` | `mcp_handler` | MCP 工具管理 |
| `skills` | `skills_handler` | 技能管理 |
| `session` | `session_handler` | 会话管理 |
| `exec` | `exec_handler` | 沙箱命令执行 |
| `sandbox` | `sandbox_handler` | 沙箱执行与状态 |
| `subagent` | `subagent_handler` | 子代理团队 DAG 编排 |
| `team` | `team_handler` | 专家团队并行 spawn（`Team::run`） |
| `browser` | `browser_handler` | 真实浏览器 CDP 操控 |
| `deploy` | `deploy_handler` | 一键部署（SSH） |
| `serve` | `serve_handler` | ACP 服务（回环） |
| `chat` | `chat_handler` | 对话（流式 REPL / 单发） |
| `gateway` | `gateway_handler` | 统一网关（回环 Web 控制台） |
| `goal` | `goal_handler` | 目标循环（自治续轮 + blocked 门控） |
| `capabilities` | `capabilities_handler` | 能力自检（真实落地状态） |
| `cache` | `cache_handler` | KV 缓存命中率与可缓存比 |
| `dispatch` | `dispatch_handler` | 无头分发任务到外部 agent CLI（meta-agent 调度一体内置） |
| `games` | `games_handler` | 内置小游戏（`list` / `play <id> <动作>`） |

> 新增子命令必须同步 `argv.rs::SUBCOMMANDS`、`program.rs` 注册与本文档表格，否则 `check-doc-cli-drift.py` 失败。

### 运行时角色说明

- `doctor` — 诊断入口，输出运行环境/模块健康度。
- `chat` — 对话主链路（流式 REPL / 单发补全）。
- `goal` — 目标驱动循环：自治续轮 + blocked 门控，支撑长任务。
- `gateway` / `serve` — 回环服务（网关 / ACP），为 Web 控制台与外部客户端提供访问面。
- `subagent` / `team` — 编排层：DAG 子代理团队 / 专家团队并行。
- `browser` — 经 CDP 操控真实浏览器（网关接入）。
- `dispatch` — 无头调度：把任务分发到外部 agent CLI，meta-agent 调度一体内置。
- `cache` — KV/语义缓存可观测（命中率、可缓存比）。
- `capabilities` — 能力自检（报告各能力真实落地状态）。
- `games` / 桌面「实验室」玩具类能力 — 实验性/娱乐能力：CLI 侧保留入口，GUI 侧折叠进「实验室」分组，不占主屏。

## 分层设计

核心引擎采用「输入 → 意图 → 规划 → 执行 → 质量校验 → 输出」的单向数据流，各层职责单一、可独立替换。

### 1. CLI 路由层（`cli/`）

- `argv.rs` / `route.rs` / `program.rs` 分工见上。
- 三种模式：`Interactive`（三栏工作台 TUI）、`Direct`（单指令）、`Subcommand`（19 路由分发）。

### 2. 意图系统（`intent/`）

- `classifier.rs` — 本地规则分类器（关键词 + 正则），识别 explain/refactor/fix/test/doc/commit/search 等意图，输出置信度；< 0.6 降级 LLM 消歧。
- `router.rs` — 意图路由表：注册 `IntentHandler`，意图 → 技能映射，支持热加载。
- `rules.rs` — 规则引擎，支撑分类器匹配规则。

### 3. Agent 运行时（`agent/`）

- `runtime.rs` — Agent 状态机 `Init → Ready → Running → Paused → Done → Error`，支持中断与恢复。
- `tool_cycle.rs` — 工具循环：工具调用 → 归并上下文；并行执行（`tokio::spawn` + `Semaphore`），超时控制。
- `subagent.rs` / `team.rs` / `trellis.rs` — 子代理、专家团队并行、编排（配合 `marguerite subagent|team`）。
- `stream_loop.rs` / `multi_strategy.rs` — 流式循环与多策略执行。

### 4. 模型层（`model/`）

- `llm.rs` — OpenAI 兼容 API 封装：流式（`tokio mpsc`）、指数退避重试。
- `router.rs` — 模型路由：按任务复杂度选模型。
- `dual.rs` — 双模型协作：规划模型生成结构化计划，执行模型逐步执行，异常回传重规划。

### 5. MCP 层（`mcp/`）

- `client.rs` — MCP 客户端（连接池 / 工具发现 / 缓存 TTL）。
- `server.rs` — MCP 服务端（HTTP JSON-RPC，工具暴露与权限）。
- `registry.rs` — 工具注册表：`ToolExecutor` trait 统一封装。

### 6. 记忆层（`memory/`）

三层记忆，分层检索（感知 → 短期 → 长期），带置信度与自动降级：
- `sensory.rs` — 当前会话上下文，SQLite 快速读写。
- `short_term.rs` — FTS5 全文检索，最近 N 个会话索引。
- `long_term.rs` — 本地特征哈希向量化（无外部 embedding 依赖默认），余弦检索、自动摘要索引。

### 7. 质量保障（`guard/`）

- `syntax.rs` — tree-sitter 语法检查（JS/TS/Python/Rust/Go）。
- `semantic.rs` — 未定义变量等轻量静态分析，LSP trait 扩展点。
- `fact.rs` — 内置 API 用法规则库 + 可选 MCP 在线查询。
- `contract.rs` — `@pre` / `@post` / `@invariant` 契约覆盖检查。
- `repair.rs` — 自动修复循环：规则修复 → LLM 修复 → 重新过护栏。
- 统一入口 `mod.rs`，可配置开关与阈值。

### 8. 上下文引擎（`context/`）

- `paging.rs` — 按语义边界分页（函数/类/段落），摘要页自动生成。
- `cache.rs` — 摘要缓存 LRU + hash 键，命中率统计。
- `budget.rs` — Token 预算：> 80% 告警，≥ 100% 触发自动压缩。

### 9. 技能与插件（`skills/` `plugins/`）

- `skills/engine.rs` / `loader.rs` / `registry.rs` — 技能加载（本地 YAML / 远程 GitHub + 签名）/ 执行沙箱 / 注册。
- `plugins/mod.rs` — 插件契约：`meta()` / `on_load()` / `capabilities()`，原生 trait 静态加载 + `Arc` 注册表热插拔。

### 10. 持久化与缓存（`persistence/` `cache/`）

- `persistence/config.rs` — `$MARGUERITE/config.yaml`（或 `~`）读取与默认模板初始化。
- `persistence/session.rs` — 会话快照保存 / 恢复 / 列表管理。
- `persistence/task_queue.rs` — 持久化任务队列：SQLite，优先级、失败重试、状态跟踪、后台 worker。
- `cache/mod.rs` / `cache/semantic.rs` — KV 与语义缓存（配合 `marguerite cache`）。

### 11. 用例能力（`browser/` `automation/` `chain/`）

- `browser/gateway.rs` / `browser/mod.rs` — 真实浏览器 CDP 操控与网关（配合 `marguerite browser`）。
- `automation/host_board.rs` — 自动化 / 宿主看板能力。
- `chain/executor.rs` — 链式 / 流水线执行（配合 `marguerite exec` 与自动化）。

### 12. 前端壳层

- **TUI（`core/src/tui/`）** — Ratatui 三栏工作台，CLI 交互 / 直接模式的默认终端界面；`MARGUERITE_LEGACY_UI` 环境变量可回退 v1 单栏。
- **GUI（`gui/`）** — Tauri 应用：React 18 前端 + Rust 薄封装。导航统一分组（核心 / 治理 / 实验室，实验室默认折叠）；首启无演示假数据（空状态引导）。遵循薄壳原则，逻辑在 core。
- **WebUI（`webui/`）** — Vue3 + Pinia + vue-i18n + vue-router 控制台，经 `gateway` / `serve` 回环服务接入 core；侧栏契约由 `App.sidebar-contract.test.ts` 守护。

## 数据流示例

```
用户输入
  │
  ▼
CLI 路由 (cli/) ── 交互/直接/子命令 → 19 路由（doctor/model/mcp/skills/session/
                     exec/sandbox/subagent/team/browser/deploy/serve/chat/
                     gateway/goal/capabilities/cache/dispatch/games）
  │ 交互 / 单指令
  ▼
意图分类 (intent/) ── 置信度 < 0.6 → LLM 消歧
  │
  ▼
模型路由 (model/) ── 复杂任务 → 规划模型 → 执行模型
  │
  ▼
Agent 运行时 (agent/) ── 工具循环 → MCP 工具 / 技能（subagent / team / goal）
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
输出（TUI / GUI / WebUI / 管道）
```

## 配置

默认模板见 `config.yaml`，首次运行复制到用户配置目录：

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
- L2: `cargo test` —— 保持 2426+ 测试基线不回退（当前 AGENTS.md 记录的权威基线）
- L3: `cargo clippy` 零 warning
- L4-security: 安全审计门禁
- L5: E2E / Playwright 场景通过
- L6: 文档/CLI 零漂移（`scripts/check-doc-cli-drift.py`）
- Q1: `vue-tsc` 0 错（WebUI typecheck）
- 统一闸门：任何改动须过 `verify.sh`
*（内容由AI生成，仅供参考）*
