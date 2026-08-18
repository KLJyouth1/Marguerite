# OPEN-DECISIONS — Marguerite 悬而未决登记册

> 依据项目 SOP（memory 系统 · 通道二）维护。只追加 + 就地关闭（OPEN → RESOLVED，补 Resolution）。
> 每次 Phase 开始时自动复现未决项到工作上下文最前面，逐条判断能否关闭。

| Date | Source | Open Item | Related Constraints | Current Leaning | Blocked By | Resolves When | Status |
|------|--------|-----------|---------------------|-----------------|------------|---------------|--------|
| 2026-08-07 | Phase E 收尾 | A 桶：KB 三件套(graph/denoiser/integrity) + security L2/L3(malware/guardrail) 落地登记 | 治理双闭环 + 编排器离线 analyze | 已实装并补登记到能力注册表 | — | 代码勘察 + 测试判定完成 | RESOLVED |
| 2026-08-07 | Phase E 收尾 | B 桶：auto_fix / auto_review / verifier（自动修复/自动评审/验证器） | 依赖 LLM 后端闭环；经 2026-08-08 勘察 model.cloud/router=Real、model.dual=Partial、仅 model.local=Narrative，不再硬阻塞 | 已实装三引擎（依赖注入式 apply/verify，离线可测），注册为 Partial | — | 三引擎单测全过 + 接入能力注册表 | RESOLVED（2026-08-08） |
| 2026-08-07 | Phase E 收尾 | C 桶：browser_bridge（浏览器自动化原生桥） | 重型原生桥，需本机 Chromium | 代码已完整实装并接入 GUI | — | 2026-08-08 代码勘察确认 | RESOLVED |
| 2026-08-08 | Phase D 后续推进 | tool.terminal（终端操控）：Narrative → 实装 | 行模式命令执行器 + ANSI 安全剥离；PTY/ConPTY 交互式待补 | 已实装行模式 + Windows ConPTY 交互式（desktop/src/conpty.rs + terminal_panel PTY 模式 + vt100 渲染），注册表升 Real | — | 单测全过（含 ConPTY 实跑 echo 回显） | RESOLVED（2026-08-08，PTY 交互式实装） |
| 2026-08-08 | 终端增强 | tool.terminal 行模式补强：持久工作目录(cd/cd /d 切换并跨命令生效) + 命令历史上/下回溯 | 不引入新依赖、可离线测试 | 已实装（core handle_cd 持久化 + 面板 history_index 回溯） | — | 单测全过 | RESOLVED（增强部分） |
| 2026-08-08 | 记忆系统增强 | mem.persistent（跨会话记忆持久化）：User/Feedback/Auto 三类 JSON 落盘 | 原仅 project(BM25) 持久，其余会话级内存；recall.rs 注释预留持久层入口 | 已实装 PersistentKV + KbBackedMemory::with_persistence，ACP serve 注入 workspace 目录 | — | 单测全过（持久化往返） | RESOLVED |

## 关闭记录

### C 桶 browser_bridge → RESOLVED（2026-08-08）
- **判定**：`core/src/browser/mod.rs`（CDP 桥：launch/navigate/evaluate/screenshot，含离线测试）+ `desktop/src/browser_control.rs`（双模式控制面板，16 单测）已完整实现并接线（`lib.rs` 声明、`capabilities.rs` 注册 `tool.browser`、被 `cli/program.rs`/`desktop/src/tools_market.rs` 引用）。
- **前置误判修正**：此前记忆称"browser_bridge 仍 OPEN（依赖重型原生桥）"——实际代码早已实装且集成，仅运行期需本机 Chromium（本机无 Chromium，故未做 live 验证，但离线测试全绿）。故登记为 RESOLVED。
- **残留约束**：运行期依赖本机 Chromium 可执行文件，能力级维持 `Partial`（非 `Real`）。

### tool.terminal → 实装并升 Partial（2026-08-08）
- 新增 `core/src/terminal/mod.rs`（行模式提交命令经 OS shell 执行、捕获输出、`strip_ansi` 剥离 ANSI 供 egui 安全显示；9 单测含 Windows cmd + 离线路径）+ `desktop/src/terminal_panel.rs`（终端面板，2 单测）+ 接入 `tools_market.rs` "终端" 标签页。
- 能力注册表 `tool.terminal` 先由 `Narrative` 升 `Partial`（行模式 Real，PTY 交互式待补）；PTY 实装后于 2026-08-08 升 **Real**（行模式跨平台 + Windows ConPTY 交互式）。

### tool.terminal PTY 交互式（ConPTY）→ RESOLVED（2026-08-08，重型 Phase 收口）
- **新增原生依赖**：`desktop/Cargo.toml` 引入 `windows-sys = 0.59`（ConPTY API：CreatePseudoConsole/ResizePseudoConsole/ClosePseudoConsole + CreateProcessW + PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE）+ `vt100 = 0.15`（终端屏幕解析）。
- **实装 `desktop/src/conpty.rs`**：`CreateFileW("CONIN$"/"CONOUT$")` 取双向管道 → `CreatePseudoConsole` 建伪控制台 → `InitializeProcThreadAttributeList`+`UpdateProcThreadAttribute(PSEUDOCONSOLE)` → `CreateProcessW(cmd.exe)` 带 `EXTENDED_STARTUPINFO_PRESENT` 启动；后台线程 `ReadFile` 从 conout 阻塞读取经 `mpsc` 推上层，`write_input` 写 conin，`resize` 调 `ResizePseudoConsole`；`Drop` 终止子进程 + 回收句柄。实跑集成测试 `conpty_spawn_cmd_and_read_output` 经伪控制台启动 cmd 并成功读到 `echo PTY_HELLO_123` 回显——证明原生集成真实可用（非摆设）。
- **`desktop/src/terminal_panel.rs` 双模式重构**：`TerminalMode{Line,Pty}` 切换；PTY 模式每帧把 conout 字节喂 `vt100::Parser`，逐格绘制屏幕（等宽 12px、xterm 16/256 调色板、背景块按需绘制），尺寸自适应调 `ResizePseudoConsole`+`parser.set_size`；子进程退出自动回到安全态。行模式逻辑与历史/测试完全保留。
- **能力注册表**：`tool.terminal` 由 `Partial` 升 **Real**（行模式跨平台、PTY 限 Windows 10 1809+）。
- **质量门禁**：desktop **71 passed** 零回归（68 原 + 默认模式测试 + 2 ConPTY 集成测试）；release 构建通过（exe 23.9 MB），打包 `marguerite-desktop-v0.2.0-win64.zip`(8.25 MB)。本回合一并交付 B 桶 + GUI 审计 + PTY 三块，三大 OPEN 全部收口。

### tool.terminal 行模式补强 → RESOLVED（增强部分，2026-08-08）
- 在已实装行模式基础上补强两项 agent 高频痛点，**零新依赖、可离线测试**：
  - **持久工作目录**：`handle_cd` 拦截 `cd [dir]` / `cd /d [dir]`（含 Windows 开关），`canonicalize` 后直接更新 `session.cwd`，后续命令在新目录执行；支持引号路径（`cd "C:\path with space"`）；无参 `cd` 打印当前目录。
  - **命令历史上/下回溯**：面板 `history_index` 游标 + `recall_prev`/`recall_next`，输入框上/下箭头回填历史；提交后复位游标。
- 单测覆盖：`cd_persists_working_directory` / `cd_supports_d_flag_windows` / `test_panel_history_recall_walks_back_and_forth` / `test_panel_submit_resets_history_cursor` 等全过。desktop 68 passed、core lib 1018 passed 零回归。

### mem.persistent 跨会话记忆持久化 → RESOLVED（2026-08-08）
- **新增 `PersistentKV`**（`core/src/memory/recall.rs`）：HashMap + JSON 文件落盘，零外部依赖；落盘失败静默保证主流程健壮；`user.json`/`feedback.json`/`auto.json` 落盘到指定目录，启动时加载既有记忆。
- **`KbBackedMemory` 升级**：User/Feedback/Auto 三类由 `RwLock<HashMap>` 改为 `RwLock<PersistentKV>`；新增 `with_persistence(dir)`，`new()`/`from_project_kb()` 保持纯内存向后兼容；`MemoryBackend` trait 接口完全不变，上层（ACP `memory.*`、编排器）零改动。
- **CLI 注入**：`cli/program.rs` 的 serve 两条路径（health + listener）改用 `with_persistence(&memory_dir())`，`memory_dir()` 优先读环境变量 `MARGUERITE_MEMORY_DIR`，回退 `当前目录/.marguerite/memory`。
- **注册表**：新增 `mem.persistent`（Real）。
- **收口"记忆 5 层仅 project 实装"差距**：原 User/Feedback/Auto 进程退出即丢，现跨会话持久沉淀。
- **单测**：`persistent_kv_survives_restart` / `kb_backed_memory_persists_user_across_instances` 全过；core lib 1020 passed、desktop 68 passed 零回归。

### B 桶（auto_fix/auto_review/verifier）→ RESOLVED（2026-08-08）
- **阻塞误判修正**：此前登记"硬阻塞于 LLM 后端（model.* 全 Narrative）"。经代码勘察，`model.cloud`(`core/src/model/llm.rs`，OpenAI 兼容客户端，已真接 DeepSeek，流式/function calling/重试全有)=**Real**、`model.router`=**Real**、`model.dual`=**Partial**，仅 `model.local`=Narrative。auto_fix/auto_review/verifier 只需云端 LLM 即可闭环，不再硬阻塞。
- **实装三引擎（依赖注入式，离线可测，沿用 `llm.rs` mock server 模式）**：
  - `core/src/automation/autofix.rs`：错误上下文 → LLM 生成修复完整文件 → 注入式 `FixApplier` 安全落盘 → 结构化裁决（`parse_fix` 提取 ``` 代码块 + 说明；4 单测覆盖解析/应用/无块不应用）。
  - `core/src/governance/review.rs`：diff → LLM 结构化评审裁决（`ReviewVerdict{pass,issues[]}`；`extract_json_object` 容错截 JSON，解析失败降级为 advisory 不静默通过；`pass = pass && blocking_count==0` 信任模型但强制阻断 blocking；4 单测）。
  - `core/src/governance/verifier.rs`：变更 + 验收标准 + 可选 `VerifyRunner` shell 校验命令 → LLM 判定达标 → `Verdict{satisfied,reasoning,evidence[]}`；复用 review 的 JSON 解析；离线 fake runner（3 单测）。
- **接线**：`automation/mod.rs` 声明 `autofix`、`governance/mod.rs` 重导出 `review`/`verifier`；能力注册表升 `auto.fix`/`gov.review`/`gov.verifier` 均为 **Partial**（运行期依赖 model.cloud 出真回复）。
- **收口"B 桶零代码"差距**：三引擎从零实装，解锁 auto_fix/auto_review/verifier 闭环，使编排器/完成门可在运行期启用自动修复与评审。
- **质量门禁**：core lib 1035 passed / 0 failed / 2 ignored（mcp 环境 flake）、desktop 68 passed 零回归；release 构建通过，exe 23.8 MB，打包 `marguerite-desktop-v0.2.0-bbucket-win64.zip`（8.22 MB）。

## 仍 OPEN 项
- **已全部收口**：A 桶(KB+security) RESOLVED；B 桶(auto_fix/auto_review/verifier) RESOLVED；C 桶(browser_bridge) RESOLVED；tool.terminal 行模式增强 RESOLVED；tool.terminal PTY 交互式(ConPTY) RESOLVED（2026-08-08）。当前无遗留 OPEN 项。

## 相关 ADR
- ADR-001: 原生 egui 桌面 + ratatui TUI 双前端（Phase D，已 Accepted）
- ADR-002: 字母徽标为项目锁定图标方案（替代 SVG 图标库，满足 P0-1 精神；egui 难直嵌 SVG）— 2026-08-08
