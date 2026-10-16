# CLI 成熟度审计报告 (Phase A)

> 审计对象：`marguerite/` Rust workspace（`core` 包 CLI 层）
> 审计时间：2026-08-07  基线：`cargo test -p marguerite-core --lib` → **935 passed / 0 failed / 2 ignored**
> 方法：直接读码 + grep 证据，逐 handler 走读，对照用户目标 CLI（opencode / claude-code-source 架构基线）
> 结论可经 `file:line` 回查。本审计**不做任何代码改动**，仅输出完成度矩阵与差距清单。

---

## 1. 现状快照（事实基线）

| 维度 | 实测值 |
|------|--------|
| 工作区成员 | `core` / `desktop` / `tui`（`Cargo.toml`）；`gui/` 为 orphan，已冻结 |
| CLI 路由数 | **11**（`argv.rs::SUBCOMMANDS` 白名单与 `program.rs` 注册表一致，无漂移） |
| CLI 模块 | `cli/{argv,route,program,output,mod}.rs` |
| 引擎模块 | `agent/` `orchestration/` `tools/` `model/` `context/` `memory/` `persistence/` `skills/` `plugins/` `mcp/` `chain/` 等 30+ 顶层模块 |
| 交互入口 | `Program::run()` 三分支：`Interactive`（→ ratatui 工作台）/ `Direct`（`-p` 单发）/ `Subcommand` |
| 测试基线 | 935 passed（比上一阶段 928 又增 7，期间有外部改动，核心引擎零回归） |

### 11 个路由 handler 实装度（逐一看过源码）

| 路由 | 子命令 | 状态 | 关键证据 |
|------|--------|------|----------|
| `doctor` | list / `--json` | ✅ 真实 | `program.rs:418` 6 项结构化自检（Phase 5） |
| `model` | list/current/use/copilot/add/remove/path | ✅ 真实 | `program.rs:748`，`ModelSelector` 全功能 |
| `mcp` | list | 🟡 部分 | `program.rs:848` 仅列配置；`mcp/client.rs` 存在但未接线到 handler |
| `skills` | list/install | 🟡 部分 | `program.rs:869` 加载/下载注册；无 `skills run` 执行入口 |
| `session` | list/delete | 🟡 部分 | `program.rs:910`；**无交互续聊/恢复**；`persistence/session.rs` 有 `restore_session` 但未被 CLI 调用 |
| `exec` | `-- <argv>` | ✅ 真实 | `program.rs:958` 真实沙箱（白名单+路径+审计） |
| `sandbox` | exec/workspace | ✅ 真实 | `program.rs:1004` |
| `subagent` | team | ✅ 真实 | `program.rs:1060` → `Orchestrator::dispatch_team_dag` 真实 DAG 编排 |
| `browser` | open | ✅ 真实 | `program.rs:1104` 真实 CDP（headless Chromium + 截图 + DOM 坐标） |
| `deploy` | ssh/baota | ✅ 真实 | `program.rs:1195`（Phase I2/I3） |
| `serve` | --health/--port | ✅ 真实 | `program.rs:688` 回环 ACP listener（Phase 5） |

---

## 2. 六维度能力成熟度矩阵（用户点名）

> 状态图例：✅ 已实现（真实逻辑 + 可测）｜🟡 部分实现（原语在，接线/范围缺口）｜❌ 缺失

| # | 用户点名维度 | 状态 | 证据（file:line） | 差距 |
|---|------------|------|------------------|------|
| 1 | **命令解析** | ✅ | `cli/argv.rs`（CliMode: Direct/Interactive/Subcommand）、`cli/route.rs`（RouteTable）、11 路由一致 | 无；whitelist 与注册表已统一 |
| 2 | **会话循环 (REPL)** | 🟡 | `program.rs:121` `CliMode::Interactive` → **ratatui 工作台**（非纯终端 REPL）；无任何 `read_line`/逐行 stdin 循环（`program.rs:162` 仅 `--pipe` 一次性读） | **缺 Codex 风格纯终端流式对话 REPL**；`session` 无续聊 |
| 3 | **工具调用** | ✅ | `agent/tool_cycle.rs:120` `ToolCycleExecutor::execute`（并行/顺序）、`call_tool`；`tools/mod.rs:8` `register_builtin_tools`；`ToolRegistry` | 无（引擎层工具回路完整） |
| 4 | **上下文管理** | ✅ | `context/{budget,cache,paging}.rs`；`memory/{experience,long_term,sensory,short_term}.rs`；`persistence/session.rs:54` `SessionManager` | 无 |
| 5 | **流式输出** | 🟡 | `model/llm.rs:293` `chat_complete_stream` → `StreamReceiver`(mpsc)；`parse_stream_chunk:456` 真实 SSE 解析 | **CLI 从不调用流式**：`handle_direct` `program.rs:200-209` 缓冲整段 `resp.to_string()` 后一次性 `println!` |
| 6 | **配置与鉴权** | ✅ | `persistence/config.rs` `Config::load/config_dir`；`model/byok.rs`（BYOK）；`collab/auth.rs` | 无 |

### 扩展维度（用户「重点关注」）

| 维度 | 状态 | 证据 | 差距 |
|------|------|------|------|
| 多模型适配 | ✅ | `model/{registry,byok,dual,bloom,router,selector,llm}.rs` | 无 |
| 插件体系 | 🟡 | `plugins/mod.rs:301` `PluginRegistry::register/load_all/invoke/list_meta` 真实 | 无 CLI 钩子/slash/skill 扩展接线（`skills run` 缺失；hook 机制未暴露到 CLI） |
| 记忆/上下文持久化 | ✅ | `memory/*` + `persistence/session.rs:115` `save_snapshot` / `:349` `restore_session` | 无（基础设施齐备，仅未被 REPL 驱动） |
| Agent 编排 | ✅ | `orchestration/mod.rs:395` `dispatch` / `:482` `dispatch_team_dag`；`AgentRuntime`（`handle_direct:198`） | 无 |
| MCP 工具接入 | 🟡 | `mcp/{client,registry,server}.rs` 存在 | `mcp` handler 仅 `list`，无 `connect`/工具调用；引擎未接 MCP 工具源 |

---

## 3. 与目标 CLI（opencode 基线）的差距清单（按优先级）

### P0 — 核心缺失（必须补，否则不算「核心 CLI 重构」达标）
1. **纯终端流式 REPL（Codex 风格）** ❌
   - 现状：无 `read_line` 循环；`Interactive` 直接进 ratatui，无「纯文本终端对话」路径。
   - 目标：无参/`marguerite chat` 进入逐行 REPL，维护 `Vec<Message>` 多轮历史，逐 token 流式打印，支持 `/help` `/model` `/clear` `/session` 等斜杠命令。
   - 落点：`core/src/cli/repl.rs`（新），接入 `run()` 的 `Interactive` 分支（新增 `--plain` 开关与 `chat` 子命令）。
2. **引擎多轮对话 API** ❌
   - 现状：`Orchestrator::dispatch(text)` 单次触发（`orchestration/mod.rs:395`）；无 `chat(messages) -> stream`。
   - 目标：新增 `Orchestrator::chat(&self, history: &[Message]) -> StreamReceiver`（或 `chat_complete_stream` 包装），供 REPL 与 `handle_direct` 共用。
3. **流式输出接线** 🟡→✅
   - 现状：原语在（`model/llm.rs:293`），但 CLI 缓冲输出。
   - 目标：`handle_direct` 与 `repl` 都走 `chat_complete_stream`，逐块 flush 到 stdout（stderr 仍走日志）。

### P1 — 部分能力接线（扩展 CLI 表面，复用已有引擎）
4. **`session` 续聊**：`session resume <id>` 调 `SessionManager::restore_session`（`persistence/session.rs:349`）载入历史 → 喂 REPL。
5. **`skills run <name>`**：`skills/engine.rs` 已有执行能力，补 CLI 入口。
6. **`mcp connect` + 工具接入**：`mcp/client.rs` 真实客户端，补 `mcp connect <name>` 并把 MCP 工具注册进 `ToolRegistry`（这是 Phase 6 文档对账曾「虚构」、现需真做的项）。
7. **插件 CLI 钩子**：`PluginRegistry::invoke` 已能按 capability 调用，补 `plugins` 子命令 + hook 生命周期接线。

### P2 — 健壮性（参考 opencode 的 tool-result 缓存、权限确认、压缩）
8. 上下文窗口压缩（`context/paging.rs` 已有分页原语，需在 REPL 长对话里接 budget）。
9. 工具调用结果缓存与权限二次确认（参考 opencode 的 tool-approval）。
10. 中断恢复（Ctrl-C 安全落盘当前会话）。

---

## 4. 给 Phase B 的规格草稿（中央缺口的设计基线）

> 仅作 Phase B 实施前的规格对齐，本阶段不写码。

```
core/src/cli/
├── argv.rs        # 现状：CliMode 加 `Plain` 变体（或 `--plain` flag）；Subcommand 加 "chat"
├── program.rs     # Interactive 分支：有 --plain 或 `chat` 子命令 → 进 repl；否则维持 ratatui
├── repl.rs        # 新增：纯终端 REPL
│   ├── loop: 逐行 read_line → Message → Orchestrator::chat(history) → 流式打印
│   ├── slash: /help /model /clear /session <id> /exit
│   └── persist: 每轮 save_snapshot 到 SessionManager
└── output.rs      # 扩展：stream_to_stdout(receiver) 逐块 flush

core/src/orchestration/mod.rs
└── impl Orchestrator
    └── pub async fn chat(&self, history: &[Message]) -> Result<StreamReceiver>  # 包装 chat_complete_stream
```

**对齐 opencode 基线的取舍**（不复制，只取架构）：
- opencode 用 `Session` 对象持有 messages + tools + model；本项目用 `Orchestrator` + `SessionManager` 组合，等价。
- opencode 的 `Conversation` 流式用 `async-stream`；本项目已有 `StreamReceiver`（mpsc），直接复用，**不引入新流式库**。
- opencode 的 slash-command 注册表；本项目用 `plugins` + `skills` 已有机制，**不另建注册表**。

---

## 5. 审计结论

- **核心引擎（编排/工具/上下文/模型/记忆/安全）已高度成熟**，935 测试守护；CLI 重构**不是从零造引擎**，而是把已有能力「暴露成 Codex 风格的交互面」。
- **唯一真正的硬缺口是 P0 三项**：纯终端流式 REPL + 引擎多轮 `chat()` API + 流式输出接线。其余多为「接线/表面扩展」，引擎原语均已存在。
- 外部 9 仓库融合（Phase C）重点吸收：**deepseek-tui（Rust TUI 状态机/repl 模式）** 直接可对应本 Phase B 的 `repl.rs`；**hermes/openclaw 的 ACP auth/events** 扩充 `serve/acp`；**claude-mem 的 RAG 持久化** 对应 `memory/` 接线。

> 下一步：用户确认后进入 **Phase B —— CLI 核心重构**，按 §4 规格先落 P0 三项（repl.rs + Orchestrator::chat + 流式接线），每步 `cargo test` + 子命令冒烟验证。
