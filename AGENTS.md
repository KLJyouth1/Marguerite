# AGENTS.md — Marguerite 仓库指南（供 AI 智能体与贡献者）

> Marguerite 是终端优先的智能体工作台：单 Rust 引擎 + 三前端（ratatui TUI / **Tauri React 桌面端** / **Vue3 Web 控制台**）。Web 端仅由回环网关（127.0.0.1）托管，**不开放公网**；Python 引擎（OpenSquilla 系）以侧车方式共存，能力经统一网关触达。

## 1. 仓库结构（权威）

- `Cargo.toml`：`[workspace] members = ["core","gui/src-tauri","tui"]`，`resolver = "2"`。
- `core/`：核心引擎 + CLI（`marguerite-core` bin）+ TUI 引擎（`marguerite-tui` 由 `tui/` 构建）+ ACP 薄壳（`serve/acp/`）。
- `tui/`：ratatui 终端工作台 bin。
- `gui/`：Tauri (React + TypeScript) 桌面端，唯一原生桌面入口（bin: `marguerite-gui`，Rust 壳在 `gui/src-tauri`）；原 egui 桌面 `desktop/` 已删除。
- `webui/`：Vue3 控制台（Vite + Pinia + vue-i18n，原 opensquilla-webui 全量迁入并品牌化为 Marguerite），构建产物出 `webui/dist`，由统一网关回环托管。
- `pyengine/`：Python 引擎侧车（OpenSquilla Python 微内核迁入，含 migrations/tests），经网关代理方法组，UI 无需感知双引擎。
- `docs/`：文档（`cli-reference.md` 为 CLI 权威手册；`mcp.md` / `development.md` 为技术指南）。

## 2. 构建与测试（权威命令）

```bash
cargo build -p marguerite-core --bin marguerite-core   # 构建 CLI 二进制
cargo test -p marguerite-core --lib                     # 单元测试
cargo build --workspace                                 # L1 构建门禁
cargo clippy --workspace --all-targets                  # L3 零 warning
python3 scripts/check-doc-cli-drift.py                 # 文档/代码 CLI 漂移门禁
```

Web UI 门禁（`webui/` 下执行）：

```bash
npm run check:architecture   # 10 项守卫（colors/motion/radius/theme-contract/…）
npm run typecheck            # check:architecture + vue-tsc --noEmit
npm run test:unit            # Vitest 单测（当前 3044 passed）
npm run build                # typecheck + vite build + 产物校验（dist/）
```

- **测试基线（2026-08-21 v5）**：`marguerite-core` lib **2015 passed + 14 ignored**（含 Bloom 三阶段单测 + dispatch 扇出 + subagent trellis 事件驱动 + DSH 融合 anchoring/deferred/编排器接线 + credentials/authorization + projection fold + 权限溯源 + 凭证缝 LLM 接线 + tokenUsage 投影共 39 项 DSH 融合新单测；测试基建加固见 §4.1）。任何改动后不得回退；若基线变化，同步更新本文件 §2。门禁运行前若 shell 带 `NO_COLOR=1` 需先移除（否则 2 个既有主题测试走单色回退失败，非代码缺陷）。
- **构建环境坑**：后台 shell 设置 `CARGO_TARGET_DIR` 可能不被采纳，二进制会落到默认 `target/debug/`；冒烟测试前请用默认 target 重新 `cargo build`。D 盘空间不足会导致 `ENOSPC` 写入失败，注意 target / 临时目录位置。

## 3. 架构不变量（invariants — 违反即缺陷）

1. **Web UI 仅回环托管**：`webui/`（Vue3 控制台）与统一网关只监听 `127.0.0.1`（默认 18790），**禁止** Tauri 前端与公网监听；三端（TUI/桌面/Web）共享蓝橙语义令牌。
2. **ACP 仅回环**：`serve` 永远只监听 `127.0.0.1`，绝不 `0.0.0.0` / 公网。
3. **离线可运行**：编排与 `agent.invoke` 走 `Orchestrator::analyze`（不调 LLM），无 API Key 降级为离线回显。
4. **内部可变性范式**：跨 `await` 共享状态用 `Arc<Mutex<T>>`；克隆锁再跨 await，避免 `MutexGuard` 非 Send。
5. **治理双闭环**：执行前 `PlanGate`（宪法 C1–C4）、产出后 `CompletionGate`（四检查 + 自报文件落盘核验）。
6. **CLI 输出契约**：结构化错误走 stderr + 退出码；`--json` 输出 `{ok,data,meta}`；尊重 `NO_COLOR`；文本标记用 ASCII `[OK]/[FAIL]`，**禁止 emoji 图标**。
7. **路由三处同步**：`cli/argv.rs` 的 `SUBCOMMANDS`、`program.rs` 的 `route_table.register(...)`、`test_program_*` 的路由计数断言必须一致。当前 **18 个路由**：`doctor` / `model` / `mcp` / `skills` / `session` / `exec` / `deploy` / `serve` / `sandbox` / `subagent` / `team` / `browser` / `chat` / `gateway` / `goal` / `capabilities` / `cache` / `dispatch`。

### 3.1 Bloom 模式与插件系统的设计红线（anti-patterns；策划 §三 + bloom.yaml）

> 以下两条来源于 dsh-forge 的反模式警告，已在 `core/src/mode/modes/bloom.yaml` 的 `anti_patterns` 字段声明，同时由代码侧**强制守卫**；任何绕过守卫的实现都视作缺陷。

8. **红线 #1：禁止全上下文重注入**（dsh-forge："禁止全上下文重注入"）
   - 会话内只允许**增量追加**消息；禁止替换 system 提示、禁止回滚或重置整段历史。
   - 插件侧实现：`plugins/di.rs::PluginLoader::discover` 会扫描 `plugin.json` 是否包含 `system_prompt` / `full_context` 字段，并把插件名加入 `context_leak_suspects()`；上层启动时必须拒绝启用该列表中的插件（或至少显式告警 + require-ack）。
   - 持久注册表 `PluginRegistryFile::entries` 的 schema 中**不得**出现以上字段；新增序列化字段时代码评审要对照本 invariant。

9. **红线 #2：避免中途替换工具面**（dsh-forge："避免中途替换工具面"）
   - 会话内工具/服务集合必须**单调扩展**；Bloom 三段锚定（`core/src/mode/anchoring.rs::AnchorStage::{Anchored→Ptc→Full}`）只加工具不撤回；PTC 阶段不得回退到锚定工具集；不得随机重置已提供的服务。
   - 插件侧实现：`plugins/di.rs::PluginLoader::guarded_provide::<T>` 在 `T` 的 `TypeId` 已出现于 `applied_service_types` 时**返回 Err 并指明红线违规**；只有 `reload_plugin()` 作为显式管理动作会清空该集合并允许替换。
   - 任何直接 `ServiceContainer::provide`（无 guard 包装）替换了同一 TypeId 的代码，在 CI lint 中应被标注为「潜在红线违规」。

## 4. 已知坑（gotchas）

- **系统代理劫持 loopback（测试隐雷）**：本机常开 Windows 系统代理（Clash 127.0.0.1:7890），reqwest 会把部分 127.0.0.1 请求也走代理——「关闭端口必被拒」等网络假设会失效（被代答 502，而 502 是可重试错误）。测试一律用「本地 mock 监听器回明确状态码」（400 不可重试 / 404 非可达），禁止用连接拒绝作为确定性失败源；单次 accept 的 mock 必须循环应答（对话流程含经验抽取等第 2 次请求，单发 mock 会让流程挂死）。公共基建见 `core/src/tests/mock_http.rs`（完整读请求后再应答，防分段读半截）。
- **D 盘写满 ENOSPC**：构建产物目录若落在 D 盘且空间不足会失败。
- **tokio::test TCP 端到端**：客户端必须用 `tokio::net::TcpStream`（异步），**绝不**用同步 `std::net::TcpStream`——后者阻塞 runtime worker，单核环境致 accept 饿死、连接超时（错误码 10060）。
- **`rm` 被 safe-delete 钩子拦截且 fail-closed**：本环境 `rm` 经 genie-trash 包装，相对路径会被拒、且失败时**拒绝删除（文件保留）**。确需删除请用绝对路径，或直接用托管 Python `os.remove()` 真删；脚本/CI 删除不可用裸 `rm`。
- **`std::thread::JoinHandle` 无 `join_timeout`**：有界网络探测用 `tokio::time::timeout` + `tokio::net::TcpStream::connect`，不要用 `std::thread::spawn().join_timeout()`（std 没有此方法）。
- **子命令后 `--flag` 曾静默丢弃**：`cli/argv.rs` 的 sub_args 收集循环在 `raw[j].starts_with('-')` 提前 break 前需识别已知 `--flag`（如 `--health` / `--port`），否则标志被吞。
- **`serve --help` 误启动 listener**：旧二进制若不含 Phase 5 代码会把 `--help` 当启动。确认二进制已重建（`argv.rs` 正确设置 `subcommand_help` 后 `run()` 会先拦截帮助再路由）。

## 5. 贡献 workflow

1. 改动前先读 `docs/cli-reference.md`（CLI 契约）与 `docs/mcp.md` / `docs/development.md`，确保理解现状。
2. 改动 CLI 路由 / 子命令 → 同步更新 `argv.rs` 的 `SUBCOMMANDS`、`program.rs` 的路由注册与测试断言，并补 `docs/cli-reference.md` 对应小节。
3. 改动后运行 `cargo test -p marguerite-core --lib`，保证 2015 基线不回退；必要时更新本文件 §2 基线数字。
4. **文档与代码冲突时以代码为权威，回改文档**——不要为了文档去虚构 CLI 动词（如 `mcp connect` 未实现就不应写进手册）；也不要为“凑齐文档示例”去新增未经验证的实现。
5. 改动 CLI 路由 / 文档示例后，运行 `python3 scripts/check-doc-cli-drift.py`，确保手册不再漂离代码（详见 §6）。

## 6. 文档/代码 CLI 漂移校验（CI 门禁）

`scripts/check-doc-cli-drift.py` 自动校验 `docs/*.md` 里出现的 `marguerite <sub>` / `marguerite <sub> <verb>` 是否都有真实 CLI 实现，防止手册漂离代码（本次优化即修复了 `mcp connect` 虚构、「`gui/` 是成员」两处漂移）。

- **Tier 1（FAIL）**：文档里的顶层子命令必须在 `argv.rs::SUBCOMMANDS` 或 `program.rs::route_table.register(...)` 中注册。
- **Tier 2（FAIL）**：对会派发嵌套动词的 handler（如 `mcp` 读 `sub_args`），文档里的嵌套动词必须在该 handler 体内作为字符串分支出现。

```bash
python3 scripts/check-doc-cli-drift.py   # exit 0 = 无漂移；exit 1 = 漂移（打印明细）
```

CI 接入（checkout 后一步）：

```yaml
- name: Doc/Code CLI drift check
  run: python3 scripts/check-doc-cli-drift.py
```

实现要点：脚本只扫描 fenced code block 与 inline `code`（忽略散文）；flag（`--health` 等）与非派发参数（session id / 路径）不检查，避免误报。根因是 CLAUDE/AGENTS 类文档与代码易漂移，本脚本把“对账”固化成可重复的门禁。

## 7. 第三方集成层（third_party/）

`third_party/` 是**参考 + 适配层**，不编译进 cargo workspace。27 个二开仓库（18 harness + 9 computer_use，见 `third_party/manifest.json`）的价值通过两条路径进入 Marguerite：

1. **模式 → Rust 实现**：computer use 三层重构已落地于 `core/src/tools/computer_use.rs`（`ScreenPerception` 截图 / `InputControl` 键鼠 / `GuiGrounding` 视觉闭环），`cargo check -p marguerite-core` 已通过（构建任务 IlbVAa exit=0）。
2. **能力 → 清单 / 适配器**：harness 类项目以声明式 `PluginManifest` 接入既有插件系统（`core/src/plugins/manifest.rs` 的 `PluginManifest` + `ManifestPlugin` + `HookRegistry`），computer use 类项目以外部 mcp/视觉适配器接入。

### 7.1 插件清单（harness → PluginManifest）

- 位置：`third_party/plugin-examples/*.plugin.json`，**18 个**，一一对应 `manifest.json` 的 harness 仓（文件名 = 仓名小写 + `.plugin.json`）。
- Schema：`core/src/plugins/manifest.rs::PluginManifest`（必填 `name`/`version`/`description`；可选 `capabilities`/`hooks`/`slash_commands`/`subagents`/`skills`）。
- 接线：`PluginManifest::from_json(&text)` 解析 → `register_manifests(&mut PluginRegistry, &[manifest])` 桥接进既有 `PluginRegistry`；`build_hook_registry(...)` 把 `hooks[].handler` 注入真实 L1/L2/L3 护栏。
- 索引与逐仓映射：`third_party/harness-manifests-index.md` + `third_party/harness/HARNESS_BRIEF.md`。

### 7.2 JIT 工程纪律技能包（Skills）

- 位置：`third_party/skills/*.yaml`，**5 个**：`engineering-discipline` / `tdd` / `systematic-debugging` / `verification-before-completion` / `write-plan-first`。
- Schema：`core/src/skills/registry.rs::Skill`（YAML 经 `core/src/skills/loader.rs::SkillLoader` 加载；字段 `name`/`description`/`version`/`prompt_template` 必填，`intent`/`tools`/`dependencies` 可选）。
- 接线：`SkillLoader::new(skills_dir).load_all()` 扫描 `*.yaml`；`SkillRegistry::register` 入册；`find_by_intent` 按关键词 JIT 触发。最便宜高杠杆 add（纯 markdown + 路由）。
- 来源：HARNESS_BRIEF 综合模式 #7（LeAgent Agent Skills v1.0 + superpowers-reasonix）。

### 7.3 校验（CI 门禁，免编译）

```bash
python3 third_party/validate_integration.py   # exit 0 = manifest 自洽 + 18/18 清单 + 5 技能结构 OK
```

脚本检查：manifest.json 计数自洽、每个 harness 仓有对应可解析清单、清单/Skill 字段结构合法、builtin 技能引用可解析。注意：当前环境未装 PyYAML，技能 YAML 用启发式校验；CI 环境建议 `pip install pyyaml` 走严格解析。

### 7.4 升级路线（优先级，详见 INTEGRATION.md §5.4）

- **P0（已落地）**：JIT 工程纪律技能包 + 18 份 harness 清单 + computer_use 三层重构 + 校验脚本。
- **P1**：扩展 `PluginManifest`（dependencies / 能力级 model）；自演化闭环（`harness/distill.rs` + `automation/cron.rs`）；fleet 模型路由（`multi_strategy.rs`）；P2P 团队（`agent/team.rs`）。
- **P2**：memory + distill 自动蒸馏；`tool_cycle` + `stream_loop` 的 DAG 并发 + 断点续跑；`events` + `bus` + tui/workbench 结构化 model↔UI 协议。
