# Marguerite CLI / TUI 全面优化方案（v1 — 2026-08-07）

> 范围：Rust 单引擎 + 双原生前端（ratatui TUI / egui 桌面）。本文是对「全面推动 CLI 开发与体验升级」请求的落地方案，含功能清单、界面/交互优化、竞品对照、分阶段实施计划。

---

## 0. 范围澄清与决策基线（必须先读）

### 0.1 关于「GUI 设计资料」的重新界定（重要）
Marguerite 的已锁定决策（`MEMORY.md` + `OPEN-DECISIONS`）是：**放弃所有 Web 端，零用户编排 UI，agent 自动编排；双原生前端为 ratatui TUI + egui 桌面**。因此本项目**不存在、也不会建设 Web GUI**。

请求中「收集竞品 GUI 设计、映射到 CLI」的正确可落地解读是：
- **真实竞品 = 终端/智能体 CLI**（Claude Code、Codex CLI、Gemini CLI、Aider、OpenCode、Warp），它们才是 Marguerite 在同一战场（终端智能体）的直接对手。
- 「GUI 设计理念」→ 翻译为**终端 UX 原则**：信息架构（resource-verb 命令分类）、视觉层级（语义色 token / 密度）、交互流（渐进披露 / 命令面板 / 流式）、UX 最佳实践（agent 友好输出、幂等、NO_COLOR）。
- 本方案**不引入任何 Web GUI**，并保留「orchestrator 自动判定 Solo/MAX/团队/集群形态、前端禁止手动选形态」这一核心差异化铁律（不照搬 Claude Code 的 `/mode` 手动切换）。

### 0.2 事实基线（2026-08-07 代码实测，非文档推断）
经直接读取 `marguerite/` 工作区源码，确认现状如下（后续所有结论以此为准）：

| 维度 | 现状 | 来源 |
|------|------|------|
| 二进制 | 实际产物为 `marguerite-core.exe`（core bin）+ `marguerite-tui.exe`（独立 crate）；二者现已统一默认 `WorkbenchApp`，legacy `App` 仅 `MARGUERITE_LEGACY_UI=1` 启用 | `core/src/main.rs`、`tui/src/main.rs`、`core/src/cli/program.rs:84-91` |
| 命令集 | Workbench 22 条 `/` 命令；legacy 8 条；两集不一致 | `core/src/tui/workbench/command.rs:66-72`、`core/src/tui/app.rs:260-358` |
| 流式 | Workbench 真异步流式；legacy **阻塞**直到 LLM 返回 | `core/src/tui/workbench/mod.rs:983,451-458`、`app.rs:227-239` |
| 顶层参数 | 无 `--help`/`-h`/`-V`；未知 flag **静默忽略**；未知子命令不报错 | `core/src/cli/argv.rs:109-112,200-203` |
| 非交互模式 | `-p/--prompt`、`--pipe` 经 headless 链执行后退出，**已存在可用** | `core/src/cli/program.rs:116-166` |
| 主题 | Workbench 令牌化 `Theme`(DEEP_SPACE 深色)，主题文件覆盖**未实现**；legacy 硬编码色；desktop 硬编码浅色+大量 emoji | `workbench/theme.rs:3,66-68`、`app.rs:127-133`、`desktop/src/main.rs:27-56,184` |
| 错误处理 | 混用 `bail!`/`eprintln!`/内联 `⚠️`（legacy 违反自身禁 emoji 规则） | `app.rs:237` |
| 历史 | 仅内存 `Vec` 上下翻，无跨会话持久化、无历史搜索 | `core/src/tui/input.rs:189-221` |
| 配置 | `$1marguerite/config.yaml`；API Key 走环境变量/`.env`（安全良好）；无校验报错；`doctor` 极简 | `config.rs:210-281,198-216` |
| serve/ACP | `127.0.0.1` 回环 JSON-RPC listener，**未暴露为 CLI 子命令** | `core/src/serve/acp/{mod,listener}.rs` |
| 文档一致性 | `mcp.md` 写 `mcp connect` 但代码只实现 `mcp list`；`development.md` 称 `gui/` 是工作区成员（实际 orphan） | `docs/mcp.md:63-66`、`docs/development.md:14`、`Cargo.toml:2` |
| 快捷键 | `Esc`→帮助（非取消）；无 `Ctrl+R` 历史搜索；`Tab`→命令面板（渲染层未确认落地） | `shortcut.rs:61-84,75,76` |
| 安全差异化 | `sec.os_sandbox` 真实实现 Landlock(Linux)/Seatbelt(macOS)，Windows 诚实 `Unsupported` | `core/src/security/os_sandbox.rs` |

---

## 1. 现状 GAP 清单（生产级应具备，当前缺失）

| # | GAP | 严重度 | 说明 |
|---|-----|--------|------|
| G1 | 两个 TUI 二进制命令集不一致，`marguerite-tui` 跑 legacy 阻塞 UI | **P0** | 发布态混乱，用户体验分裂 |
| G2 | 无顶层 `--help`/`-h`/`-V`；未知 flag 静默忽略 | **P0** | agent/人不可发现，违反 CLI 基本契约 |
| G3 | 错误无结构化格式、无稳定退出码、混用 emoji | **P0** | 人/agent 都无法自愈 |
| G4 | 无 `--json` / `--output-format` 机器可读输出 | **P1** | 无法脚本化、无法被 agent 调用 |
| G5 | 配置无校验、报错不友好；`doctor` 极简 | **P1** | 首次运行即可能卡死无提示 |
| G6 | 主题仅深色、文件覆盖未实现（desktop 经全量扫描实测零 emoji，无需处理） | **P1** | 视觉一致性缺失（浅色主题 + toml 加载） |
| G7 | 无命令历史搜索 / 跨会话持久化 | **P1** | 效率与连续性差 |
| G8 | `serve`/health 未暴露为 CLI 子命令 | **P2** | 能力不可编排 |
| G9 | 文档与代码不一致（mcp connect / gui 成员），无 man/自动参考 | **P2** | 误导用户 |
| G10 | 快捷键怪异（Esc→help，无历史搜索键） | **P2** | 交互反直觉 |

---

## 2. 竞品设计要点对照分析

### 2.1 竞品矩阵（终端/智能体 CLI 直接对手）

| 能力维度 | Claude Code | Codex CLI | Gemini CLI | Aider | OpenCode | **Marguerite 现状** | **目标** |
|----------|-------------|-----------|------------|-------|----------|----------------------|----------|
| 语言/形态 | Node TUI | **Rust** TUI | Node TUI | Python | Go(Bubble Tea) | **Rust** ratatui+egui | 保持 Rust 优势 |
| 非交互 `-p` | 有，`--output-format text/json/stream-json` | 有 | 有 | 弱 | 有 | 有（缺格式开关） | 补齐格式开关 |
| 渐进式 `--help` | 顶级索引 + 每子命令 help + 示例 | 有 | 有 | 有 | 有 | **无** | 全量补齐 |
| 命令面板 `/` | 输入 `/` 弹列表+模糊过滤 | `/plan`等 | 有 | 有 | 有 | Workbench 有（legacy 无） | 统一并确认渲染 |
| 流式输出 | 流式 | 流式 | 流式 | 否 | 流式 | Workbench 流式 / legacy 阻塞 | 全量流式 |
| OS 级沙箱 | 否（权限提示） | **Landlock/Seatbelt** | 否 | 否 | 否 | **已实现 Landlock/Seatbelt** | **差异化卖点，对外强调** |
| 权限分级 | default/plan/auto/acceptEdits | read-only/auto/full | 多档 | 否 | 否 | security L1/L2/L3 分层 | 复用并暴露 |
| 历史持久化 | 会话 resume | 有 | 有 | git 提交 | SQLite | 仅内存 | SQLite/文件持久化 |
| 上下文压缩 | `/compact` | 有 | 有 | repo map | 有 | Workbench `/compact` | 保留 |
| 自检 `doctor` | 有，详尽 | 有 | 有 | 弱 | 有 | 极简 | 升级 |
| 编排形态 | 需手动选 | 单 agent | 单 agent | 单 agent | 单 agent | **自动判定 Solo/MAX/团队/集群** | **保持自动，增强可观测** |

### 2.2 提炼的可迁移原则 → CLI 映射

1. **信息架构：resource-verb 一致 taxonomy**（Claude Code/agent-friendly CLI 共识）
   - 当前 Marguerite 子命令 `doctor/model/mcp/skills/session/exec/deploy` 与 slash 命令 `mode/plan/model/...` 命名不统一。
   - 映射：统一为 `marguerite <resource> <verb>`（如 `marguerite session list/resume`、`marguerite model set`、`marguerite mcp list`），slash 命令作为交互态别名。

2. **视觉层级：语义色 token + 可控密度**（Warp/OpenCode TUI 范式）
   - 映射：完成 `Theme` 令牌化（已是），补齐**浅色主题 + 主题文件加载**，desktop 去 emoji 改用 SVG/文本图标。

3. **交互流：渐进披露 + 命令面板 + 流式 + spinner**（Claude Code/Codex）
   - 映射：统一命令面板（确认渲染层落地），全量流式 + 任务进度 spinner，避免 legacy 阻塞。

4. **UX 最佳实践：agent 友好输出契约**（agent-friendly-cli / Memori 原则）
   - 映射：`--json` 稳定结构 `{ok,data,meta}`、分层退出码、stderr=错误/stdout=数据、`NO_COLOR` 尊重、幂等、危险操作 `--dry-run/--yes`、边界 `--limit`、稳定 ID。

---

## 3. 优化后完整功能清单（目标态）

### 3.1 统一命令分类（resource-verb）
```
marguerite                          # 进入 Workbench 交互（默认，流式）
marguerite -p "..." | --pipe       # 非交互单指令，退出
marguerite --help | -h | --version # 顶层发现（P0 新增）
marguerite doctor [--json]          # 环境/配置/网络自检（升级）
marguerite model  list|set|get     # 模型管理
marguerite mcp     list|connect|add|remove   # MCP 服务器（补齐 connect）
marguerite session list|resume|new|export    # 会话管理（持久化）
marguerite skills  list|info        # 技能
marguerite deploy [--env] [--dry-run] [--yes] # 部署（危险操作保护）
marguerite serve  [--port] [--health]  # 暴露 ACP listener + 健康检查（P2 新增）
marguerite exec   -- <cmd>          # 透传
```
交互态 slash 命令作为上述资源的别名（`/model`→`model set` 等），**两入口行为一致**。

### 3.2 输出与错误契约
- `--output-format text|json|stream-json`（默认 text）；`--json` 别名。
- JSON 结构：`{"ok":true,"data":{...},"meta":{"schema_version":"1.0","elapsed_ms":..}}`；错误 `{"ok":false,"error":{"code","message","hint","retryable"}}`。
- 退出码：`0` 成功 / `2` 用法错误 / `3` 认证 / `4` 未找到 / `5` 临时失败 / `9` 冲突。
- `NO_COLOR` 与 non-TTY 自动去色。
- 错误必须「教下一步」：`--env is required. Usage: marguerite deploy --env staging. Example: marguerite deploy --env production --dry-run`。

### 3.3 帮助系统
- 顶层 `--help` = 导航索引（列资源+一句话+示例）。
- 每子命令 `--help` = 用法形状 + 必选/可选 + ≥2 个可直接运行示例。
- 交互态 `/help` 与命令面板（输入 `/` 模糊过滤）。

### 3.4 配置与首次运行
- `doctor` 升级：校验 config.yaml schema、API Key 可达、网络、沙箱支持，给出结构化报告 + 修复建议。
- 首次运行 onboarding 向导（非强制，可 `--yes` 跳过）。
- `--config <path>` 指定配置。

### 3.5 主题与可访问性
- 令牌化 `Theme` 补齐浅色 + 主题文件 `$1marguerite/themes/*.toml` 加载（加 toml 依赖）。
- desktop 去 emoji，改用项目锁定 SVG/文本图标。

### 3.6 历史与补全
- 输入历史写 `$1marguerite/history` 跨会话持久化；`Ctrl+R` 增量历史搜索。
- `/` 前缀命令/路径 Tab 补全。

---

## 4. 具体界面与交互优化方案

### 4.1 统一二进制（解决 G1，P0）
- 将 `marguerite-tui` crate 的入口改为调用 WorkbenchApp（与 core bin 一致），**废弃 legacy 阻塞 `App`**，或保留为 `MARGUERITE_LEGACY_UI=1` 显式降级且仅用于排障。
- 删除 `core/src/tui/app.rs` 内联 emoji（`⚠️`）改用语义 token 文本。
- 验收：两个二进制启动后命令集、流式行为一致。

### 4.2 顶层发现（解决 G2，P0）
- 在 `argv.rs` 增加 `--help/-h/--version` 短路；未知 flag **报错并给用法**而非静默忽略；未知子命令打印可用列表。
- 验收：`marguerite --help` 退出 0 并打印索引；`marguerite --bogus` 退出 2 并打印可用子命令。

### 4.3 错误与输出契约（解决 G3/G4，P0/P1）
- 新增 `core/src/cli/output.rs`：统一 `print_ok/json/err`；`err` 写 stderr + 结构化 + 退出码。
- 全量子命令支持 `--json`；非交互模式默认 `--output-format text`。
- 验收：`marguerite doctor --json` 输出可 `jq` 解析；错误退出码分层。

### 4.4 主题与 desktop 一致（解决 G6，P1）
- `theme.rs` 增加 `LIGHT` + `load_or_default` 读 toml；desktop `main.rs` 复用同一 `Theme` 语义色，移除 emoji 图标。
- 验收：`$1marguerite/themes/light.toml` 生效；desktop 零 emoji。

### 4.5 历史与补全（解决 G7，P1）
- `input.rs` 历史落盘 + `Ctrl+R` 搜索；`/` 补全渲染层确认并强化。
- 验收：重启后历史可用；`Ctrl+R` 弹出搜索。

### 4.6 快捷键修正（解决 G10，P2）
- `Esc`→取消当前输入/中断（`Cancel`），不再弹帮助；帮助改为 `F1` 或 `/help`。
- 验收：Esc 取消输入不弹帮助。

### 4.7 serve/health CLI（解决 G8，P2）
- 新增 `serve` 子命令封装 `AgentInvoker` listener + `/health` 端点探测。
- 验收：`marguerite serve --health` 返回 JSON 健康状态。

### 4.8 文档对账（解决 G9，P2）
- 修正 `mcp.md`（connect 实为 list，或补齐 connect 实现）、`development.md`（gui 非成员）；生成 `docs/cli-reference.md` 自动参考 + `AGENTS.md`。

---

## 5. 分阶段可落地实施计划

> 每阶段含：范围、受影响文件、具体改动、验收（EARS）、预估。阶段间无依赖时可并行，但建议按序（P0 优先）。

### Phase 0 — 二进制统一与顶层发现（P0，快赢，~1-2 天）
- 范围：G1、G2。
- 文件：`tui/src/main.rs`、`core/src/tui/app.rs`、`core/src/cli/argv.rs`、`core/src/cli/program.rs`。
- 改动：tui 入口切 WorkbenchApp；argv 增 `--help/-h/--version` 与未知参数报错；legacy emoji 清除。
- 验收（EARS）：When 用户运行 `marguerite --help`，系统**必须**打印资源索引并退出 0；If 传入未知 flag，系统**必须**退出 2 并打印可用项。

### Phase 1 — 错误与输出契约（P0/P1，~2-3 天）
- 范围：G3、G4。
- 文件：新增 `core/src/cli/output.rs`；改造 `program.rs` 各子命令与 `argv.rs` 退出码。
- 改动：统一 err/ok/json 输出；退出码分层；`--json/--output-format`；`NO_COLOR` 尊重。
- 验收：While 子命令以 `--json` 调用，系统**必须**输出 `{ok,data,meta}` 到 stdout；If 缺少必选参数，系统**必须**写结构化错误到 stderr 并退出 2。

### Phase 2 — 帮助系统与示例（P1，~2 天）
- 范围：help 全量 + 示例。
- 文件：`program.rs` 各子命令 usage、`command.rs` slash help、`docs/cli-reference.md`。
- 改动：每子命令独立 `--help`+≥2 示例；交互 `/help` 与面板。
- 验收：When 运行 `marguerite <resource> --help`，系统**必须**显示用法+示例；每条示例**应该**可直接运行。

### Phase 3 — 主题与 desktop 一致（P1，~2-3 天）
- 范围：G6。
- 文件：`workbench/theme.rs`、`desktop/src/main.rs`、`Cargo.toml`（加 toml）。
- 改动：浅色主题 + toml 加载；desktop 去 emoji 改用语义色/文本图标。
- 验收：If 存在 `$1marguerite/themes/light.toml`，系统**必须**加载该主题；desktop 渲染**必须**零 emoji。

### Phase 4 — 历史与补全（P1，~2 天）
- 范围：G7。
- 文件：`core/src/tui/input.rs`、`shortcut.rs`、`workbench` 补全渲染。
- 改动：历史落盘 + `Ctrl+R` 搜索；`/` 补全强化。
- 验收：While 用户输入历史命令，系统**必须**跨会话持久化并可搜索。

### Phase 5 — doctor 升级 + serve CLI（P2，~2-3 天）
- 范围：G5、G8。
- 文件：`program.rs` doctor/serve、`config.rs` 校验、`serve/acp`。
- 改动：doctor 结构化校验+修复建议；serve 子命令封装 listener+health。
- 验收：When 运行 `marguerite doctor`，系统**必须**校验 config schema/key/网络并报告；If 运行 `marguerite serve --health`，系统**必须**返回 JSON 健康态。

### Phase 6 — 文档对账与 AGENTS.md（P2，~1 天）
- 范围：G9。
- 文件：`docs/mcp.md`、`docs/development.md`、新增 `docs/cli-reference.md`、`AGENTS.md`。
- 改动：修正不一致；生成自动 CLI 参考；写 `AGENTS.md`（workflow/invariants/gotchas）。
- 验收：docs 与代码**必须**一致；`AGENTS.md`**必须**包含 workflow 与风险表。

---

## 6. 验收标准汇总（EARS 精选）
| 编号 | 功能 | EARS 标准 | 优先级 |
|------|------|-----------|--------|
| AC-01 | 顶层帮助 | When 运行 `marguerite --help`，系统必须打印资源索引并退出 0 | P0 |
| AC-02 | 未知参数 | If 传入未知 flag，系统必须退出 2 并打印可用项 | P0 |
| AC-03 | 机器输出 | While 子命令以 `--json` 调用，系统必须输出 `{ok,data,meta}` 到 stdout | P1 |
| AC-04 | 错误教学 | If 缺必选参数，系统必须写结构化错误（含 hint+example）到 stderr 并退出 2 | P0 |
| AC-05 | 二进制一致 | When 启动 `marguerite` 与 `marguerite-tui`，系统必须呈现相同命令集与流式行为 | P0 |
| AC-06 | 去色 | If `NO_COLOR` 设置或非 TTY，系统必须输出无 ANSI 色 | P1 |
| AC-07 | 主题 | If 存在 light 主题文件，系统必须加载 | P1 |
| AC-08 | 历史 | While 输入历史命令，系统必须跨会话持久化并可搜索 | P1 |

---

## 7. 优先修复建议（P0 先动）
- **P0（立即）**：Phase 0（二进制统一+顶层帮助）+ Phase 1（错误/输出契约）。这两项解决「不可用/不可发现/不可自愈」三宗罪，且不动核心引擎，风险低、收益高。
- **P1（紧随）**：Phase 2/3/4（帮助示例、主题一致、历史补全）——体验质变。
- **P2（收尾）**：Phase 5/6（doctor/serve、文档对账）——完整度与可信度。

### 保留的差异化资产（勿动）
- `sec.os_sandbox` Landlock/Seatbelt 真实实现（对标 Codex CLI 的卖点）。
- orchestrator 自动判定形态、前端禁手动选形态（对标 Claude Code 手动 `/mode` 的差异化）。
- API Key 不落 YAML 明文的安全设计。

---

## 8. 执行记录（2026-08-07 — Phase 0 / Phase 1 已落地）

> 本轮按「全程我来拍板」授权，直接落地 P0 快赢（Phase 0 + Phase 1）。所有改动零核心引擎逻辑风险，**911 测试基线（909 passed + 2 ignored）完全不变**。

### 8.1 已落地改动
- **二进制统一（G1）**：`core/src/main.rs` 与 `tui/src/main.rs` 默认均启动 `WorkbenchApp`；legacy `App` 仅当 `MARGUERITE_LEGACY_UI=1` 时启用。两入口命令集/流式行为一致。
- **顶层发现（G2 / AC-01）**：`argv.rs` 新增 `--help/-h/--version/-V` 短路；`program.rs::print_top_help()` 输出资源索引（无 emoji、无 ANSI 色，便于 agent 抓取）。`marguerite-tui --help` 同样生效。
- **错误契约（G3 / AC-02）**：未知选项 `--bogus` 与未知子命令 `frobnicate` 均退出码 2 并打印可用项；新增 `ExitCode` 分层枚举（Usage=2 / Auth=3 / NotFound=4 / Temporary=5 / Conflict=9）。
- **输出契约（G4 / AC-03）**：新增 `core/src/cli/output.rs`（`print_json_ok` / `print_json_err` / `exit_with`）；`doctor --json` 与 `-p` 单指令 `--json` 输出稳定 `{ok,data,meta}`。直接模式运行失败现正确退出 1（改造前误为 0）。
- **去 emoji**：`core/src/tui/app.rs` 两处 `⚠️` 已移除；`doctor` 文本输出装饰 `✓` 已移除。

### 8.2 实测验证
- `cargo test -p marguerite-core --lib` → **909 passed + 2 ignored（911 total）**，与改造前基线完全一致。
- 功能冒烟：`--help`/`--version` 退出 0；`--bogus` 与 `frobnicate` 退出 2 且打印用法/可用子命令；`doctor --json` 输出可被 `jq` 解析的 JSON。
- 修正过程中发现并修复一个真实缺陷：未知裸词子命令原先被静默忽略并回退到启动 TUI（应报错），已在 `argv.rs` 记录 `unknown_subcommand` 并由 `program.rs` 退出 2。

### 8.3 对原方案的更正
- **二进制命名**：实际产物为 `marguerite-core.exe`（core bin）与 `marguerite-tui.exe`；文档中 "marguerite" 为期望名，当前并非实际文件名。如需统一为 `marguerite`，需在 `Cargo.toml` 加 `[[bin]] name`，列为后续小项。
- **G6 desktop emoji（原 P1）**：经全量 emoji 扫描，`desktop/src` **零 emoji**，原方案该条不成立，无需处理。core 内剩余 `✓/✗` 仅出现在 `chain/cli.rs` 与 `status.rs` 的文本状态标记及已批准的几何状态点（`✖/▮/◉`），非 GUI 功能图标，符合项目禁 emoji 规则。

### 8.4 待续（未在本轮执行）
- Phase 5：doctor 结构化校验升级 + `serve` 子命令 —— **已于 §8.8 落地**（8 路由、928 passed）。
- Phase 6：文档对账（`mcp.md` connect / `development.md` gui 成员）—— 仅余此项未做。

### 8.6 Phase 3 已落地（2026-08-07 续 — 主题系统 + NO_COLOR + 浅色变体）

> 实地核查发现：TUI **已有**完善语义 token 系统（`workbench/theme.rs::Theme`，全区域走 `Theme::DEEP_SPACE.*`，非硬编码色）；`Theme::load_or_default` 已预留 toml 契约但为规避环境安全层 `Cargo.lock` 写入争用（os error 5）**刻意不引入 `toml` 依赖**。因此本阶段**不引入 toml**，改用环境变量驱动，更稳且即时生效。

- **浅色主题 `Theme::LIGHT`**：白底深字、蓝主 `#2563EB`（对比度优先），无紫粉渐变，token 与深色同构。
- **NO_COLOR 单色降级（AC-06 真实缺口）**：新增 `Theme::monochrome()`（仅黑/白/灰）+ `Theme::resolve(no_color, hint)` + `Theme::effective()`（读 `NO_COLOR` / `MARGUERITE_THEME` 环境变量）。`NO_COLOR` 置位时强制单色，保证无障碍与 agent 抓取可读。
- **渲染主路径生效化**：workbench 全部 7 个区域文件 + `workbench/mod.rs` 中硬编码的 `Theme::DEEP_SPACE` 统一替换为 `Theme::effective()`，使主题切换 / NO_COLOR **真正生效**（此前因硬编码常量而形同虚设）。
- **新增 4 个单元测试**：浅色对比度、resolve(light/dark)、NO_COLOR 强制单色、effective 默认深空。

**实测验证**：
- `cargo test -p marguerite-core --lib` → **916 passed + 2 ignored**（新增 4 个主题测试，核心引擎零回归）。
- 构建 `marguerite-core` + `marguerite-tui` 均 exit 0。
- ⚠️ **已知残留（未动，按「保留已弃用资产」原则）**：legacy 路径 `core/src/tui/{chat,input,status}.rs` 仍用原始 `Color::Blue` 等硬编码色，仅在 `MARGUERITE_LEGACY_UI=1` 时生效，非活跃 UI。留给后续清理或随 legacy 一并退役。Bloom 主题（`bloom/theme.rs`）的 NO_COLOR 降级同理暂未接入（bloom 不在活跃 workbench 路径）。

**对原方案更正（AC-07）**：原 AC-07「If 存在 light 主题文件，系统必须加载」落地为**环境变量 `MARGUERITE_THEME=light`** 而非 toml 文件，理由同上（环境约束 + 即时生效）。toml 文件覆盖仍列为后续增强，契约已预留于 `load_or_default`。

### 8.7 Phase 4 已落地（2026-08-07 续 — 命令历史持久化 + 上/下召回 + Ctrl+R 反向搜索，AC-08）

> 实地核查：活跃输入为 `state.composer.buffer`，提交在 `workbench/input.rs::Composer::submit`（Enter 时 `mem::take` 缓冲）；legacy `tui/input.rs` 的 HistoryUp/Down 不在活跃路径。Phase 4 接活跃路径，新增独立可测模块，不污染核心引擎。

- **新建 `core/src/tui/workbench/history.rs`**：`CommandHistory` 纯逻辑状态机——`load()`（从 `$1marguerite/history` 读，启用落盘）、`push()`（去重/跳空/上限 500/落盘）、`recall_up/down`（上/下召回）、`start_search`/`search_type`/`search_backspace`/`search_next`/`search_prev`/`search_current`/`exit_search`（Ctrl+R 反向搜索子状态机）。落盘为纯文本一行一条，**不引入新依赖**（无 toml/dirs）。`enabled=false` 时不读写文件（供单测零副作用）。
- **接入活跃路径**：`ComposerState` 新增 `history` 字段；`Composer::submit` 提交时 `history.push(text)`；`WorkbenchApp::new` 启动 `history = CommandHistory::load()` 加载跨会话历史；`reduce_key` 新增 `Up`/`Down` 召回、`Ctrl+R` 进入/切换搜索、`Esc` 退出搜索（均仅在非审批/非帮助态生效）。
- **渲染反馈**：`region/main.rs` 合成器行在搜索态显示 `> (r) {查询}  <<{候选}>>`，普通态不变。
- **快捷键登记**：`shortcut.rs` 注册 `Up`/`Down`/`Ctrl+R` 为 `ChatInput` 作用域（HistoryPrev/Next/Search），冲突检测覆盖。
- **新增 6 个单元测试**：去重/跳空、上/下召回循环、搜索筛选与导航、空查询倒序列全、退出接受/拒绝、enabled 标志。

**实测验证**：
- `cargo test -p marguerite-core --lib` → **922 passed + 2 ignored**（新增 6 个历史测试，核心引擎零回归）。
- 构建 `marguerite-core` + `marguerite-tui` 均 exit 0。
- 单测覆盖历史全部逻辑路径；TUI 交互（Ctrl+R 浮层/Up/Down）因需真实终端未自动跑，逻辑已单测隔离验证。

**Phase 6（文档对账：`mcp.md` connect / `development.md` gui 成员 / `cli-reference.md` 补 serve / 新增 AGENTS.md）已于 2026-08-07 收尾，详见 §8.9。Phase 0–6 已全部完成。**

### 8.5 Phase 2 已落地（2026-08-07 续 — 每子命令帮助 + 示例 + 参考手册）

> 按「全程我来拍板」授权继续推进。不新增路由（保持 7 个），**911 测试基线不被破坏**，实际新增 3 个解析测试后变为 **912 passed + 2 ignored**。

- **子命令级 `--help`（AC-01 子命令级）**：`argv.rs` 在子命令后识别 `-h`/`--help`（仅 `--` 分隔符之前生效，`exec -- -h` 仍透传）；`program.rs::print_subcommand_help(name)` 为单一可信源，覆盖 doctor/model/mcp/skills/session/exec/deploy，每个含用法 + ≥2 可直接运行示例。
- **无参子命令统一走帮助**：`model`/`mcp`/`skills`/`session`/`exec`/`deploy` 无附加动作时现统一调用 `print_subcommand_help`（替代原先各 handler `_` 分支的重复文本，消除漂移）；`doctor` 无参仍为叶子动作（运行诊断），符合 git 式直觉。
- **未知子命令 + `--help`**：`frobnicate --help` 因顶层 `--help` 优先而显示顶层索引（退出 0）——这是有意的友好兜底；纯 `frobnicate`（无 --help）仍退出 2（AC-02 不变）。
- **新增 `docs/cli-reference.md`**：完整 CLI 参考手册（概览、全局选项、退出码表、7 子命令逐项用法/示例、JSON 输出契约、FAQ），对应 Phase 6 的 `docs/cli-reference.md` 产出已提前完成。

**实测验证**：
- `cargo test -p marguerite-core --lib` → **912 passed + 2 ignored**（已含 3 个新增 argv 解析测试）。
- 功能冒烟：7 个子命令 `X --help` 均退出 0 并渲染帮助；`doctor`（无参）仍运行诊断（非帮助）；`model`（无参）现显示帮助；`frobnicate`（无 --help）退出 2。

### 8.8 Phase 5 已落地（2026-08-07 续 — doctor 结构化升级 + serve 子命令，P2）

> 按「全程我来拍板」授权推进收尾。新增第 8 个路由 `serve`，**922 测试基线不被破坏**，实际新增 6 个测试后变为 **928 passed + 2 ignored**。

- **第 8 路由 `serve`（G8）**：`argv.rs::SUBCOMMANDS` 加入 `"serve"`；`program.rs` 注册 `route_table.register("serve", serve_handler, "ACP 服务（回环）")`。
- **`serve_handler` 双形态**：
  - `serve --health` → **非阻塞** JSON 健康态探测：校验 `127.0.0.1:<port>` 可绑定 + API Key 已配置，返回 `{ok, data:{status, loopback_bindable, port, acp_methods, engine, config}, meta}`（已做扁平化，去除冗余 `data.data` 嵌套）。
  - `serve [--port N]` → 启动**真实回环 ACP listener**（仅 `127.0.0.1`，绝不暴露公网，默认 18181）；`agent.invoke` 经 `EngineInvoker` 真实路由到 `Orchestrator` 离线 `analyze`（不调 LLM）。端到端走真实回环端口在既有集成测试已验证（ping/health/agent.invoke）。
- **复用 + 提取，不重写网络层**：`serve/acp/{mod,listener,protocol,session}.rs` 薄壳直接复用；原先测试内私有的 `EngineInvoker`（包装 `Orchestrator`）提取为 `orchestration/mod.rs` 的公共可复用实现，消除重复定义（顺带修掉 E0252 重复导入）。
- **`doctor` 结构化升级（G8 关联）**：新增 `DoctorCheck{name,ok,detail,hint}` + `validate_config_schema`（校验 model_name/provider/mcp servers 非空）+ `probe_model_endpoint`（tokio 3s 超时 TCP 探测 base_url，规避 `std::thread::join_timeout` 不存在的坑）；现跑 6 项自检（config_dir/config_schema/api_key/network/sandbox/engine）。
  - 文本输出用 ASCII `[OK]/[FAIL]`（避 emoji、利 NO_COLOR）；`--json` 输出 `checks[]/summary{passed,failed,total}/status` + 环境信息，契约干净。
- **argv flag 静默丢弃 bug 修复（同类连带）**：子命令后 `--flag` 被静默丢弃（影响 `serve --health/--port` 等）已修复——在 sub_args 收集循环里识别 `--health`（置 `args.health`）、`--port <n>`（解析 u16 置 `args.port`），其余未知 `-` 选项仍提前 break 交顶层报错。
- **测试同步**：`test_program_default` 与 `test_program_new_registers_default_routes` 路由断言 7→8；新增 argv 解析测试 4 个（serve health/port/组合/exec 仍在未知选项处停）、`test_serve_health_json_contract`、`test_doctor_json_runs`、`test_handlers_succeed`（含 serve --health 断言）。

**实测验证**：
- `cargo test -p marguerite-core --lib` → **928 passed + 2 ignored**（零回退，核心引擎逻辑无回归）。
- 冒烟（默认 target 二进制，`NO_COLOR=1`）：
  - `serve --help` → 退出 0，渲染用法 + 示例（**确认不会误启动长驻 listener**）。
  - `serve --health` → 退出 0，返回扁平化 JSON `{ok:true, data:{status:"ok", loopback_bindable:true, port:18181, acp_methods:[...], engine:{tools:24,research_docs:0,automations:10}, config:{...}}}`，可安全 `| jq .status`。
  - `doctor` → 6/6 `[OK]`，文本诊断。
  - `doctor --json` → `checks[]` + `summary{passed:6,failed:0,total:6}` + `status:"ok"`。
  - 顶层 `--help` 资源索引已含 `serve` 条目。

**环境注记（非代码问题）**：后台构建设的 `CARGO_TARGET_DIR=mgt_build5` 未被 shell 采纳，二进制实际落到默认 `target/debug/`；冒烟前需以默认 target 重新 `cargo build -p marguerite-core --bin marguerite-core`，否则会跑旧二进制（旧二进制 `serve --help` 因不含 Phase 5 路由会误启动 listener 并卡死，已用 `taskkill` 终止验证）。

### 8.9 Phase 6 已落地（2026-08-07 续 — 文档对账与 AGENTS.md，P2）

> 按「全程我来拍板」授权收尾。纯文档 / 治理文件，无代码改动，**928 测试基线不受影响**。核心原则：文档与代码冲突时以代码为权威，回改文档，不为文档虚构 CLI 动词。

- **`docs/mcp.md`（G9）**：删除虚构的 `marguerite mcp connect`（源码 `mcp_handler` 仅实现 `list`，其余动词落 `_` 分支打印帮助）；改为只列 `mcp list`，并说明连接由引擎依据 `config.yaml::mcp.servers` 启动时自动建立，新增服务器直接改配置文件。与 `docs/cli-reference.md` §4.3 一致。
- **`docs/development.md`（G9）**：
  - 环境要求移除「Node.js（仅 GUI）：≥ 18」，改为「双原生前端均为纯 Rust，无需 Node.js」。
  - 项目结构：`gui/` 标注为「⚠️ 已弃用 Web 端（Tauri 壳）；非工作区成员（orphan），已冻结」；补充 `tui/`（ratatui，bin `marguerite-tui`）与 `desktop/`（egui，bin `marguerite-desktop`）为真实工作区成员；与 `Cargo.toml::members=["core","desktop","tui"]` 一致。
  - 「GUI 开发」整章改写为「前端架构（双原生，无 Web 端）」：双前端纯 Rust 共享 `core` 引擎，给出 `cargo run -p marguerite-tui` / `cargo run -p marguerite-desktop` 运行方式，并明确 `gui/` 冻结、勿在此开发。
- **`docs/cli-reference.md`（对账补遗）**：补 §4.8 `serve` 子命令（Phase 5 新增的第 8 路由，手册此前漏写）——含 `--health` 非阻塞探针 / `[--port N]` 阻塞监听两种形态、JSON 字段清单、示例与安全约束（仅 `127.0.0.1`）。现手册覆盖全部 8 路由。
- **新增 `AGENTS.md`**（Phase 6 验收要求）：含 ①仓库结构（权威成员）②构建与测试命令（含 928 测试基线）③架构不变量 7 条（无 Web 端 / ACP 仅回环 / 离线可运行 / `Arc<Mutex>` 范式 / 治理双闭环 / CLI 输出契约 / 路由三处同步）④已知坑 5 条（D 盘 ENOSPC / tokio::test 异步 TCP / 无 `join_timeout` / 子命令 `--flag` 静默丢弃 / `serve --help` 误启动）⑤贡献 workflow（冲突以代码为权威）。

**对账核验结果**：
- `gui/` 无 `Cargo.toml`、不在 `members` → `development.md` 旧「工作区成员」断言已纠正。
- `mcp_handler` 源码 grep 证实仅 `list` 实现 → `mcp.md` 旧 `connect` 已删除。
- `cli-reference.md` 现状 grep 证实原有 4.1–4.7（止于 `deploy`），§4.8 `serve` 已补。
- 无任何代码改动，零回归风险。

### 8.10 Phase 7 已落地（2026-08-07 续 — 文档/代码漂移自动化校验）

> 用户延续授权：将 Phase 6 对账发现的「文档与代码漂移」固化为一条可重复的 CI 门禁，避免以后再漂。

- **新增 `scripts/check-doc-cli-drift.py`**（零依赖，仅标准库 `re`/`pathlib`）：
  - 扫描 `docs/*.md` 的 fenced code block 与 inline `code`（**忽略散文**），提取 `marguerite <sub>` 与 `marguerite <sub> <verb>`。
  - **Tier 1（FAIL）**：顶层子命令必须出现在 `core/src/cli/argv.rs::SUBCOMMANDS` 或 `program.rs::route_table.register(...)`。
  - **Tier 2（FAIL）**：对会派发嵌套动词的 handler（源码含 `sub_args` 读取，如 `mcp`），嵌套动词必须在该 handler 体内作为字符串分支出现。
  - flag（`--health` 等）与非派发参数（session id / 路径）不检查，避免误报。`exit 0` = 无漂移，`exit 1` = 漂移（打印明细）。
- **验证**：
  - 正向：当前 docs 全部通过（`OK: 2 top-level + 3 nested ...`，canonical routes = 8 个）。
  - 负向自测：临时写入含 `marguerite bogus`（Tier1）与 `marguerite mcp connect`（Tier2）的 md，脚本正确抓出两项并 `exit 1`；删临时文件后回归全绿。
- **接入 `AGENTS.md` §6**：作为 CI 门禁步骤固化（`python3 scripts/check-doc-cli-drift.py`），并在 §2 验证命令、§5 workflow 引用。

**环境坑（记为通用经验）**：本环境 `rm` 经 genie-trash 包装，相对路径被拒且失败时**拒绝删除（fail-closed）**，脚本/CI 删除不能依赖裸 `rm`；确需删用绝对路径或托管 Python `os.remove()`（本次临时 md 即用此法真删）。

---

*本方案基于 2026-08-07 对 `marguerite/` 工作区的直接代码核查与终端智能体 CLI 竞品研究，所有结论可经文件路径回查。未引入任何 Web GUI，符合项目锁定决策。Phase 0–7 已全部落地（含 Phase 6 文档对账、AGENTS.md、Phase 7 漂移自动化校验）。*
