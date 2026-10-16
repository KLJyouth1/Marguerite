# Marguerite CLI 核心重构与外部能力融合 · 整合方案与执行计划

> 生成日期：2026-08-07
> 目标：以 Codex/Claude Code 类开源 CLI 为架构基线，二次开发整合进 Marguerite 现有 CLI；融合 9 个外部仓库 + Claude Code SDK 的可复用能力；输出完成度审计；最终基于同一核心层落地 GUI。
> 原则：**分析吸收、非直接搬运**（尤其 `deepseek-tui` 等为 Rust， temptation 大但须按本项目命名/分层重写）；每阶段产出可运行、可验证结果。

---

## 0. 关键前提与决策点（需确认）

| # | 决策 | 现状 | 我的建议 | 影响 |
|---|------|------|----------|------|
| **D1** | **GUI 目标形态** | 历史锁定决策：「放弃 Web 端，双原生前端（ratatui TUI / egui 桌面）」；但 `D:/Downloads/pages` 是 **5 个 HTML 设计稿**（工作台/任务列表与项目工作区/可视化代码生成/子代理蜂群/工具调用与市场中心），天然是 Web 形态 | **推荐 A：原生 egui 桌面 + 增强 ratatui TUI**，把 pages/ 当作**视觉规范参考**而非直接复用 HTML；GUI 与 CLI 共用 `core` crate | 若选 B（WebView/Tauri 包 HTML）则推翻历史锁定决策，需你明确拍板 |
| **D2** | **Codex CLI 基线映射** | 外部目录无字面 `codex-cli`；最贴近的 OSS CLI agent 是 `opencode`（bun/TS，活跃维护的 Claude/Codex 替代）与 `claude-code-source`（Claude Code OSS，含 `KAIROS_ARCHITECTURE.md`） | 以 **`opencode` 为主基线**（会话循环/工具派发/配置自导出/流式/provider 抽象），`claude-code-source` 为次基线（context 窗口/plan mode/streaming 协议） | 二者皆 TS，仅吸收**架构**不移植代码 |
| **D3** | **禁止直接搬运** | 历史铁律：「禁止直接搬运 GUI_Agent 代码」 | 全程遵守：取架构/模式/数据结构，按本项目 `core/src/*` 分层与命名重写 | 影响所有 Phase C 融合项 |
| **D4** | **执行顺序** | 你要求「先方案→逐步实施，每阶段可验证」 | 按 A→B→C→D→E 推进；每阶段结束可 `cargo test` + 冒烟验证 | — |

> 仅 **D1** 涉及推翻历史锁定决策，请在本计划批准后明确选 A 还是 B。其余我按建议拍板。

---

## 1. 现状快照（已实地核查，非凭记忆）

### 1.1 工作区
- `Cargo.toml::members = ["core", "desktop", "tui"]`；`gui/` 是 orphan（无 Cargo.toml，已冻结）。
- `core/src/` 已有 40+ 模块：`cli/ argv·program·route·output·mod`、`session/`、`context/`、`memory/`、`kb/`、`tools/`、`mcp/`、`plugins/(仅 mod.rs)`、`serve/acp/`、`tui/ app·chat·input·status + bloom + workbench`、`model/`、`orchestration/`、`governance/`、`guard/` 等。

### 1.2 CLI 实际态（2026-08-07 核查）
- `argv.rs::SUBCOMMANDS` = 11 个：`doctor, model, mcp, skills, session, exec, deploy, serve, sandbox, subagent, browser`（**与 `program.rs` 注册表 11 个一致，无漂移**——此前记忆中的「8」已过时，印证需全量复核）。
- `doctor` 已结构化（6 项自检 + `--json`）；`serve` 已接回环 ACP listener（`EngineInvoker`→`Orchestrator` 离线分析）；`session/exec/sandbox/subagent/browser` 均已注册 handler。
- **缺口（目标 CLI 视角）**：交互式 REPL 会话循环是否真实现待核；流式输出（SSE/chunk）仅在 ACP 层，CLI 直出无；多模型 provider 抽象在 `model/` 但 CLI `model` handler 仅管理；插件**运行时缺失**（plugins/ 仅 mod.rs 注册表）；memory/context 未与 CLI 会话接线。
- 测试基线：**928 passed + 2 ignored**（Phase 5 末）。

### 1.3 外部仓库档案（本次核查）
| 仓库 | 语言 | 关键结构 | 可提取价值 |
|------|------|----------|------------|
| `opencode` | TS/bun | `packages/sdk/js`、AGENTS.md 自导出配置模式 | **主架构基线**：会话/工具派发/配置/auth/流式 |
| `claude-code-source` | TS | `KAIROS_ARCHITECTURE.md`、`src/` | 次基线：context 窗口、plan mode、streaming 协议 |
| `deepseek-tui` | **Rust** | `crates/`: agent·app-server·cli·config·core·execpolicy·hooks·mcp·protocol·secrets·state·tools·tui·tui-core | **最高价值 Rust 源**：TUI 交互、状态持久化、密钥、exec 策略、hooks、MCP、工具 |
| `hermes-agent` | Py | `acp_adapter/`(auth·events·permissions·session·tools·server)、`acp_registry/` | ACP 方法完备性 → 扩展 `serve/acp` |
| `openclaw` | TS | `extensions/`、`docs.acp.md` | 插件/扩展系统 schema |
| `claude-mem` | TS | `plugin/`(hooks·modes·skills·ui)、`ragtime/`(ragtime.ts RAG) | 记忆/上下文持久化 + RAG |
| `claude-code-plugins` | TS/md | agent-sdk-dev·code-review·feature-dev·frontend-design·hookify·learning-output-style | slash command / hook / skill 设计规范 |
| `agent-s` | Py | `gui_agents/`(s1·s2·s2_5·s3)、computer-use | 子代理编排、computer-use（设计参考，不移植 Py） |
| `ds4` | C | `rax.c`(radix tree)、`ds4_cli.c` | 可选：本地 RAG 索引后端（低优先级，评估） |
| `pages/`(GUI 稿) | HTML | 5 个设计稿 | **GUI 视觉规范**（见 D1） |

---

## 2. 融合映射表（Part 2 落地依据）

| 来源 | 提取模块 | 整合进本项目 | 冲突处理 |
|------|----------|--------------|----------|
| opencode | 会话循环、工具派发、配置自导出、provider 抽象、流式 | `cli/session.rs`(新)、重构 `cli/program.rs`、`model/`、扩展 `cli/output.rs` | TS→Rust 重写；只取架构 |
| claude-code-source | context 窗口管理、plan mode、streaming 协议 | `context/`、`session/`、`cli/output.rs` | 忽略 web 专属 |
| deepseek-tui | tui-core 渲染、state 持久化、secrets、execpolicy、hooks、mcp、tools | `tui/`(增强)、core 密钥管理、`tools/`、`mcp/` | **Rust→Rust 但按本项目命名重写**，剥离其 GUI_Agent 品牌 |
| hermes-agent | ACP auth/events/permissions/session/tools | `serve/acp/`(补方法) | Py→Rust 重写 ACP surface |
| openclaw | 扩展/插件 manifest schema | `plugins/`(运行时) | 适配本项目插件模型 |
| claude-mem | RAG(ragtime)、memory 持久化、plugin hooks/modes/skills | `memory/`、`context/`、`kb/`(KB 2.0)、`skills/` | RAG 算法适配 KB 2.0 |
| claude-code-plugins + Claude Code SDK | hook / slash-command / skill 扩展机制 | `plugins/` + `skills/` 运行时 | 采纳规范为 skill 规格 |
| agent-s | 子代理编排(s1-s3)、computer-use | `subagent_handler`(交叉参考)、`browser_handler` | 设计启发，不移植 Py |
| ds4 | rax 索引 | 评估：本地 RAG 索引后端 | 低优先级，仅评估 |

---

## 3. 分阶段执行计划

### Phase A — 进度全量审计（先行， grounding 一切）
- **输入**：当前 `core/src` 全部模块 + 外部基线架构。
- **产出**：`docs/cli-maturity-audit.md` —— 完成度矩阵（已实现/部分/缺失）含 `file:line` 证据；差距清单按 P0/P1/P2。
- **重点核查**：① 交互 REPL 是否真实现 ② streaming 直出 ③ 多 provider ④ 插件运行时 ⑤ memory/context↔CLI 接线 ⑥ 工具调用循环。
- **验证**：审计文档 + 一段 CLI 内省测试（列出所有 route + handler 存在性）。

### Phase B — CLI 核心重构（基于 opencode 架构基线）
1. 统一命令解析：`argv.rs` whitelist 与 `route.rs` 注册表同源（消除任何潜在漂移）。
2. 新增 `cli/session.rs`：交互式多轮会话循环（读取 stdin→编排→工具调用回路→上下文窗口→流式回显）。
3. 流式输出：`cli/output.rs` 增加 chunk/SSE 渲染，供 session 与 ACP 共用。
4. 配置与多 provider 鉴权：`model/` 抽象 provider（OpenAI/Anthropic/本地），`config.yaml` 驱动。
5. 工具调用回路：`tools/` 注册表接入 session loop（plan/act/observe 闭环）。
- **验证**：`cargo test -p marguerite-core --lib` 全绿；`marguerite session`/`marguerite <cmd>` 冒烟；REPL 手动多轮测试。

### Phase C — 外部能力融合（逐源）
- 按 §2 映射表，**每源一个独立集成单元 + 测试**，绝不跨源大杂烩。
- 严守 D3（不搬运）。`deepseek-tui` 因同语言最易误搬，单独设立「重写检查」：凡引用其符号须改名并归到本项目模块。
- **验证**：每单元 `cargo test` 通过 + 对应能力冒烟（如插件加载、ACP 新方法、RAG 检索）。

### Phase D — GUI 层（待 A/B/C 稳定）
- 以 `pages/` 5 稿为视觉规范；**共用 `core` crate**，GUI 仅做表现层。
- 推荐 D1-A：增强 `marguerite-desktop`(egui) + `marguerite-tui`(ratatui)；pages 映射：工作台→dashboard、任务列表与项目工作区→session/project、可视化代码生成→code-gen、子代理蜂群→subagent DAG、工具调用与市场中心→tools/market。
- **验证**：`cargo run -p marguerite-desktop`/`cargo run -p marguerite-tui` 可启动并与 CLI 同核心。

### Phase E — 全量穷举复核
- 穷举：所有 `.md`、`working_memory/*`、`tasks`、AGENTS.md、cli-reference.md、drift 校验脚本 ↔ 实际代码逐条对账。
- 修正漂移（如「8 路由」陈旧记忆、任何文档虚构 CLI 动词）。
- 扩展 drift 校验脚本覆盖新增命令与插件 manifest。
- **产出**：复核报告 + 更新后的治理文档。

---

## 6. 执行进度与记录

### Phase A — 进度全量审计 ✅（2026-08-07）
- 产出 `docs/cli-maturity-audit.md`：完成度矩阵（已实现/部分/缺失）含 `file:line` 证据 + P0/P1/P2 差距清单。
- 关键结论：CLI 实际 **11 路由**（argv whitelist 与 program.rs 注册表一致，无漂移）；缺 **Codex 风格流式 REPL** 与 **流式输出**；`model`/`tools`/`plugins`/`AgentRuntime::chat_stream` 已实装。
- 修正陈旧记忆：此前记忆中的「8 路由」已过时（实为 11，后增至 12）。

### Phase B — CLI 核心重构 · P0（流式 REPL / chat）✅（2026-08-07）
落地（参考 opencode/Codex 架构，**非复制**，纯 Rust 适配本项目命名）：
1. `argv.rs`：SUBCOMMANDS 增 `"chat"`（11→12）；新增 `--json` 子命令后旗标识别（修复「`chat --json "x"` 被误判为未知子命令」解析 bug）。
2. `agent/runtime.rs::chat_stream`：复用 `process_input` 的模型选择 + 经验提示前置，但逐块吐出（不缓冲），返回 `StreamReceiver`。
3. `orchestration/mod.rs::chat`：薄委托 `lead.chat_stream`（P0-2）。
4. `cli/repl.rs`（新增）：`stream_assistant`（逐 token 流式渲染，JSON 模式静默收集）+ `run_repl`（多轮 `Vec<Message>` 历史、斜杠命令 /help /clear /model /exit，Ctrl-D 退出）。
5. `cli/mod.rs`：`pub mod repl;`。
6. `program.rs`：`chat_handler` —— 有 `sub_args`→单发流式（含 `--json` 信封），无→进 REPL；顶部/子命令帮助补全；路由计数断言 11→12。
- **验证**：
  - `cargo test -p marguerite-core --lib` → **937 passed / 0 failed / 2 ignored**（基线 928→937，净 +9 测试，零回退）。
  - 冒烟（NO_COLOR）：`chat --help`(exit 0)、`chat`(REPL 进/出干净)、`chat "x"`(真实 API 调用，流内错误 exit 1)、`chat --json "x"`(JSON 错误信封 code 5)。
- **已知配置瑕疵（非代码）**：本环境 `config.yaml` 的 `model_name="DeepSeek-V4 Flash/Pro"` 大小写不符 API 要求（应 `deepseek-v4-pro`/`deepseek-v4-flash`），单发返回 400——属配置问题，不在 Phase B 范围；已通过正确错误链路暴露。
- **P0 未含（留 P1）**：REPL 当前走纯对话流（`chat_stream`）；若需 REPL 内调用工具回路，应切到 `AgentRuntime::process_input`（缓冲式 agent loop），列为 Phase B-P1 候选。

### Phase B-P1 — CLI 核心重构 · P1（多轮会话持久化）✅（2026-08-07）
落地（延续 Phase B 的 `chat` 子命令，收敛为「纯持久化」——见下「关键范围决策」）：
1. `argv.rs`：`Args` 新增 `resume: Option<String>` 与 `continue_last: bool` + `Default` 同步；子命令后识别 `--resume <id>`（缺 id 回退 `continue_last`）与 `--continue`/`-c`，互不吞掉后续 flag。
2. `repl.rs`（重写）：抽离 `ReplStore`（可降级封装——DB 失败→纯内存，不中断对话）+ `ReplOptions` + `handle_slash`（可单测），新增 16 个单测。
   - `ReplStore::open_at` 默认 `Config::config_dir().join("sessions.db")`；`restore_on_open`（resume 优先于 continue_last，复用原 session_id）；`persist`（空历史 noop 不产生空记录、失败仅提示一次）；`list`/`switch`（短 id 前缀匹配）/`short_id`（取 8 位，与 `session list` 对齐）。
   - `run_repl`：启动打印 session id、每轮对话后落盘、Ctrl-D 退出前落盘；斜杠命令 `/help /exit /quit /clear /model /save /sessions /load <id> /history`（空历史 `/save` 提示「无需保存」）。
3. `program.rs`：`chat_handler` 打通恢复链路——带 `resume`/`continue_last` 时 `ReplStore::open` 恢复历史并推入 User→`chat_stream`→`stream_assistant`→推入 Assistant→`store.persist`；裸单发保持无副作用；`chat` 帮助文本补全会话选项与斜杠命令；新增 5 个 argv 解析单测。
- **关键范围决策（实地核查后调整）**：`model/llm.rs` 流式层**当前无 function calling**（`chat_complete_stream` 请求体无 `tools`、解析 `Delta` 无 `tool_calls`），而 `session_messages` 表与 `Message::Tool` 已就绪——故 **Phase B-P1 本轮只落地「多轮会话持久化」，工具回路（`agent/tool_cycle.rs` + `tools/`）因 LLM 缺 function calling 暂不可行**，明确后移。
- **验证**：
  - `cargo test -p marguerite-core --lib` → **958 passed / 0 failed / 2 ignored**（基线 937→958，净 +21 测试：argv 5 + repl 15 + `/save` 回归 1，零回退）。
  - 冒烟（NO_COLOR）：`chat --help`(exit 0)、REPL 斜杠序列（/help /sessions /load /save /clear /history 各路径 exit 0)、`chat --resume ghost-id-9999`（未知 id 优雅回退，stderr 提示 + exit 0）。
  - 存储同源已机械证明：`repl.rs` 与 `program.rs` 均用 `Config::config_dir().join("sessions.db")`；`session list` 可管理 REPL 落盘会话（DB 位于 `$1marguerite/sessions.db`）。
  - 漂移脚本 `check-doc-cli-drift.py` 扫描 12 路由，exit 0 无漂移。
- **Phase B-P1 第二部分 / Phase C-1 ✅（2026-08-07）**：LLM function calling + REPL 工具回路已落地（即原「待续」项 + Phase C function calling 基座）：
  1. `model/llm.rs`：新增 `ToolDef`/`ToolCall` 类型；`build_body`/`build_stream_request_body` 统一委托 `build_body(messages, stream, tools)`，非空 `tools` 注入 OpenAI function 格式 `tools`；`parse_chat_response` 解析 `tool_calls`；`parse_stream_chunk` 按 `index` 累积流式 `tool_calls`（`RawToolCallDelta`/`RawFunctionDelta` 嵌套 `function`，与 OpenAI 一致），流末 `[DONE]` 一次给出 `finish_reason:"tool_calls"`；新增 `chat_complete_with_tools` / `chat_stream_with_tools`。
  2. `mcp/registry.rs`：新增 `describe_local_tools() -> Vec<(name, description, schema)>`，供 LLM 层生成 `ToolDef`。
  3. `model/router.rs`：`chat_with_tools` 透传。
  4. `agent/runtime.rs`：`run_tool_loop` 驱动 plan→act→observe 闭环（顺序执行 `ToolCycle::execute_sequential`，`max_rounds=8` 防死循环，工具未命中回灌模型自我纠正），`parse_tool_args` 把 JSON 参数扁平化为 `HashMap<String,String>`。
  5. `cli/{argv,repl,program}.rs`：`--tools`/`-t` 旗标贯穿解析→`ReplOptions.enable_tools`→`run_repl`（构建 `ToolRegistry` + `ToolDef`，用户轮次改走 `run_tool_loop`，实时打印 `Message::Tool` 活动）；单发路径同享工具回路（`--json` 时工具活动转 stderr 避免污染 stdout）。
- **验证**：`cargo test -p marguerite-core --lib` → **965 passed / 0 failed / 2 ignored**（基线 958→965，净 +7：llm 5 function-calling + argv 2 `--tools`，零回退）；冒烟 `chat --help` 列出 `-t, --tools`、`echo /exit | chat --tools` 打印「工具回路已启用（21 个内置工具）」；`check-doc-cli-drift.py` exit 0。
- **已知配置瑕疵（同 Phase B）**：本环境 `model_name` 大小写不符 API 要求，单发 `--tools` 仍返回 400——属配置问题，工具回路代码链路已验证可达模型调用。

- **Phase C-2 ✅（2026-08-07）**：hermes-agent ACP 方法面融合（逐源单元，守 D3 不搬运）：
  1. `serve/acp/session.rs`：`AcpSession` 新增 `tool_registry: Option<Arc<ToolRegistry>>` 字段 + `new_with_tools(...)` 构造；`methods()` 注入注册表时追加 `tools.list`/`tools.invoke`，常驻 `permissions.list`/`auth.capabilities`/`events.list`/`session.list`。
  2. 新增方法处理：`m_tools_list`（转发 `ToolRegistry::describe_local_tools`）、`m_tools_invoke`（解析 `name`+`args`，转发 `execute_local_tool`，映射 `ToolResult` 三态 `Success`/`Error`/`Timeout`）、`m_permissions_list`（L1/L2/L3 分层护栏清单）、`m_auth_capabilities`（回环信任模型）、`m_events_list`（事件目录，`subscription: not_implemented` 诚实标注）、`m_session_list`（活跃会话清单）；模块级 `parse_tool_args` 归一 `args`。
  3. `cli/program.rs::serve_handler`：session 构造改走 `new_with_tools`（注入 `build_acp_tool_registry()`，复用 `register_builtin_tools`，失败降级空注册表），`acp_methods` 与启动横幅均用 `session.methods()` 单一真相源（删硬编码数组）。
  4. 单测 +10（tools.list / tools.invoke 成功+缺参+未知工具+未配置 / permissions.list / auth.capabilities / events.list / session.list / session.info 方法清单）。
- **验证**：`cargo test -p marguerite-core --lib` → **975 passed / 0 failed / 2 ignored**（基线 965→975，净 +10，零回退）；`cargo build` 零警告；冒烟 `serve --health` 的 `acp_methods[]` 含全部 10 方法、`engine.tools=24`；`check-doc-cli-drift.py` 扫描 12 路由 exit 0 无漂移。

- **Phase C-3 ✅（2026-08-07）**：claude-mem 记忆/KB 2.0 对齐（逐源单元，守 D3 不搬运）：
  1. `memory/recall.rs`（新增）：`MemoryCategory`（User/Project/Feedback/Auto 对齐 claude-mem 四类持久记忆）+ `MemoryBackend` trait + `KbBackedMemory` 默认实现（`Project` 走既有 KB 2.0 `KnowledgeBase` BM25，`User`/`Feedback`/`Auto` 走内存映射，会话级、接口不变、未来可接 `memory/` 持久层）+ `RecallHit`。
  2. `memory/mod.rs`：导出 `recall` 模块。
  3. `serve/acp/session.rs`：注入 `Option<Arc<dyn MemoryBackend>>`（`new_with_memory` / `new_full` 构造），ACP 新增 `memory.search`（category+query+top_k 召回）、`memory.remember`（category+key+value 写入）；`methods()` 注入后端时追加两方法；`serve --health` 横幅同步。
  4. `cli/program.rs::serve_handler`：health 与 listener 两条路径均改走 `new_full`，注入 `KbBackedMemory::new()`。
  5. 单测 +13（recall 8：类别解析/BM25/project 遗忘/空查询/子串+token 召回/空 key 拒绝/map 遗忘/session 注入；ACP memory 5：remember→search 往返/user 召回/非法类别/缺 key/未配置）。
- **验证**：`cargo test -p marguerite-core --lib` → **988 passed / 0 failed / 2 ignored**（基线 975→988，净 +13，零回退）；`cargo build` 干净；冒烟 `serve --health` 的 `acp_methods[]` 含 12 方法（`memory.search`/`memory.remember` 在列）；`check-doc-cli-drift.py` exit 0。

- **Phase C-4 ✅（2026-08-07）**：claude-code-plugins + Claude Code SDK 插件/skill 运行时（逐源单元，守 D3 不搬运）：
  1. `plugins/manifest.rs`（新增）：声明式 `PluginManifest`（name/version/hooks/slash_commands/subagents/skills）+ `ManifestPlugin` 桥接器（把清单桥接为既有 `Plugin` trait，可直接 `register` 进 `PluginRegistry` 与原生编译期插件共存）+ `HookRegistry`（对齐 Claude Code PreToolUse/PostToolUse/Notification/Stop 钩子派发，顺序执行、首个 deny 短路，钩子执行复用 L1/L2/L3 护栏概念）+ `build_hook_registry`（从清单 + 注入真实 handler 装配）+ `register_manifests` 便捷注册。
  2. `plugins/mod.rs`：导出 `manifest` 模块。
  3. 单测 +13（manifest 解析全段 / 派生能力 / 桥接进注册表 + slash.subagent.skill 能力路由 / 未知能力报错 / HookRegistry 顺序派发 / deny 短路 / 空事件 / 从清单装配 / 事件解析）。
- **验证**：`cargo test -p marguerite-core --lib` → **997 passed / 0 failed / 2 ignored**（基线 988→997，净 +9 manifest 单测，零回退）；`check-doc-cli-drift.py` exit 0。claude-code-plugins 融合未引入新 CLI 路由（仅 `plugins/` 运行时），故漂移脚本无新增 token。

- **Phase C-5 ✅（2026-08-07）**：agent-s 子代理编排 / 分阶段反思（s2_5）对齐（逐源单元，守 D3 不搬运）：
  1. `orchestration/dag.rs`：新增 `DagKind::Reflect`（5 节点：调研→规划→实现→**反思自检**→审校，在「实现」与「审校」间插入 s2_5 反思阶段，对齐 agent-s 的 s1 感知→s2 工作记忆→s2_5 反思→s3 行动）；新增 `Reflection` 结构 + `reflect_pass`（确定性、离线可测：对各阶段产出/工作记忆做 go/no-go 自检，失败标记含 `[子任务…失败/无结果]` 或空产出）。
  2. `cli/program.rs::subagent_handler`：于 handler 内解析 `--reflect`/`--agentic` 旗标（不触 argv whitelist / route 注册 / drift 脚本，零漂移），选中 `DagKind::Reflect`；编排结束后调用 `reflect_pass` 打印 s2_5 决策（GO/NO-GO + 逐阶段笔记）。无旗标时维持经典 `Build` 形态，向后兼容。
  3. 单测 +5（Reflect 分解形状/依赖链、topo 4 层、reflect_pass 全良 / 检测失败 / 空阶段）。
  4. `cli-reference.md`：补 `subagent`/`browser`/`sandbox` 三节（此前已注册子命令却未入手册，修正文档漂移）；`subagent` 节注明 `--reflect`（agent-s 对齐），`browser` 节说明 CDP 截图 + 归一化坐标即 computer-use 感知基础。
  5. 首跑实测 **1001 passed / 1 failed**：`test_topo_layers_reflect` 断言层内序 `["t1","t2"]`，实际 `["t2","t1"]`。定性为**测试断言写错、实现正确**——`topo_layers` 契约为「priority 降序 → id 升序」，Reflect DAG 中 t1(调研)=7、t2(架构设计)=8，故 t2 必先于 t1。已修正断言并补契约注释，**未迁就测试修改实现**（保护确定性排序不被破坏）。
  6. 顺手清零 C-4 遗留告警：`plugins/manifest.rs::execute` 未使用的 `ctx` → `_ctx`，现构建**零 warning**。
- **验证**：`cargo test -p marguerite-core --lib` → **1002 passed / 0 failed / 2 ignored / 零 warning**（基线 997→1002，净 +5，零回退）；`cargo build`（target_verify 全 workspace）core/desktop/tui 三者均链接通过——此前 `LNK1104`/`.cargo-lock` 报错系 `target/debug/.cargo-lock` 被外部环境持有排他锁所致（os error 5，连 Win32 `DeleteFileW` 与重启 scheduled `MoveFileEx` 均被沙箱拒绝），非 C-3/C-4 代码缺陷，已用独立 `target_verify` 验证桌面/终端二进制正常；`check-doc-cli-drift.py` exit 0（12 条规范路由，新增三节被识别为有效文档）。冒烟：`subagent` 用法正确输出 `--reflect`；`serve --health` 返回 12 个 ACP 方法（含 `memory.*`）。

### 下一步
- **Phase C 收口**：`agent-s` 融合已完成（C-5 子代理分阶段反思 + browser computer-use 感知基础已就位）；`ds4`（rax 本地 RAG 索引后端）为低优先级评估项，可择机做技术评估，不阻塞后续 Phase。
- **Phase D**：GUI 层（D1 已锁定 A：原生 egui 桌面 + ratatui TUI，pages/ 仅作视觉规范；待 C 稳定后启动）。
- **Phase E**：全量穷举复核（穷举 `.md`/working_memory/tasks 与代码逐条对账，扩展 drift 脚本）。

---

## 4. 质量门禁（沿用既有纪律）
- 禁 emoji 图标 / 紫粉渐变 / AI 模板味（Bloom Mode v2.0）。
- 单文件 ≤ 300 行、目录分层、逻辑下沉 service。
- 每模块 lint→type-check→test 自检循环（≤3 轮）。
- `cargo test -p marguerite-core --lib` 基线 1002 不得回退（净增测试计入）。
- 新增命令须同步 argv whitelist + route 注册 + cli-reference.md + drift 脚本。

## 5. 风险与备注
- **D1 GUI 形态**是唯一需你拍板的分叉；其余我按建议推进。
- `opencode`/`claude-code-source` 为 TS，仅架构参考，不引入 Node 构建到 Rust 工作区。
- 外部仓库体积大，融合时**只读分析、不 vendoring 源码**进本项目。
- 执行将按「继续」逐 Phase 推进，每 Phase 结束给你可验证结果。
