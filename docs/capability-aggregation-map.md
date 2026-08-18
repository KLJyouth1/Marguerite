# 能力聚合清单与归属映射表

> 汇总各来源仓库可复用能力、其融合归属模块，以及本阶段新增的 MCP / Bloom / 网关修复。
> 对应 marguerite 项目（core= Rust 统一网关 / pyengine= opensquilla Python 引擎 / webui= Vue 控制台 / gui= Tauri 壳）。

## 1. 能力来源与归属总览

| 来源 | 可复用能力 | 融合归属模块 | 融合状态 |
|---|---|---|---|
| `WorkBuddy\xiaoyuangou\tools\baota_bridge` | 宝塔面板站点/数据库/FTP/文件/终端/系统信息操作（面板内部 API + x-http-token / request_token 双轨认证） | **新增** `btpanel-mcp/`（独立 MCP Server 包）→ pyengine MCP discovery 注册 `mcp_baota_*` | ✅ 本阶段完成 |
| `_GUI_Agent_archived_20260713\gwwc`（GWWC） | MaxModePrivileges 特权语义、MAXModeOrchestrator 全子系统激活、四阶段推理轨迹（ReasoningRound） | `core/src/mode/privileges.rs`（BloomPrivileges Rust 化）、`core/src/model/bloom.rs`（ReasoningRunner） | ✅ 已 Rust 化（B 阶段前） |
| `_GUI_Agent_archived_20260713\external\ds4`（antirez C） | DeepSeek V4 Flash 本地推理 + OpenAI 兼容 API + DSML 工具格式 | 候选：`ModelRouter` fallback 本地侧车 | 🔶 未接入（需离线侧车运行时） |
| `opensquilla`（上游） | Router tiers（c0-c3）、B5 融合聚合管线（proposer→aggregator）、provider registry | `pyengine/src/opensquilla/provider/ensemble.py`、`squilla_router`（pyengine 为上游超集） | ✅ 已内化 |
| `E:\新建文件夹 (3)\GUI_Agent\nexustar-api`（NexusStar） | MaxDualLeaderOrchestrator 三阶段 + 共识加权聚合；BrowserRealControl 贝塞尔拟人键鼠 + VLM 定位 + 序列录放 | `core/src/model/bloom.rs`（swarm 双领队）；**`core/src/tools/builtin.rs` system_control 拟人贝塞尔移动** | ✅ 贝塞尔已接入；共识聚合/序列录放待办 |
| `E:\新建文件夹 (3)\_GUI_Agent_archived_20260713` | 模式配置 YAML 快照、NORA 架构设计、fusion-moa spec | 设计参考（模式选择器 UI、MoA 融合引擎规格对照） | 🔶 参考文档 |
| `D:\Downloads\GUI_Agent` | 空壳（nul 占位） | — | ⛔ 无源码 |
| `marguerite\external\` | 目录不存在；9 个调研项目为在线资料（deepseek-tui YOLO / openclaw 浏览器 / agent-s grounding 等仅记入调研文档） | `docs/external-capability-research.md` | 🔶 语义已内化（对齐不搬迁） |

## 2. 宝塔面板 MCP 工具集（本阶段新增）

位置：`btpanel-mcp/`（零第三方依赖，stdio + SSE 双传输，MCP 2024-11-05 协议）。
接入：`opensquilla.toml` 的 `[mcp]` 段（见 `pyengine/opensquilla.toml.example`），boot 顺序 22 discovery 注册为 `mcp_*` 工具。

| 工具 | 用途 | 关键参数 | 端点 |
|---|---|---|---|
| `baota_health` | 配置与面板连通状态 | — | — |
| `baota_sites_list` | 站点列表 | page, limit | `/site/get_site_list` |
| `baota_site_create` | 创建站点（含 PHP 版本） | webname, path, php_version | `/site?action=AddSite` |
| `baota_site_status` | 启停站点 | site_name, status(0/1) | `/site?action=SetSiteStatus` |
| `baota_databases_list` | 数据库列表 | page, limit | `/data?action=getData` |
| `baota_database_create` | 创建 MySQL 库 | name, password?, db_user? | `/database?action=AddDatabase` |
| `baota_ftps_list` | FTP 列表 | page, limit | `/data?action=getData` |
| `baota_ftp_create` | 创建 FTP | name, password, path | `/ftp?action=AddFtp` |
| `baota_files_list` | 列目录 | path | `/files?action=GetDir` |
| `baota_file_read` | 读文件 | path | `/files?action=GetFileBody` |
| `baota_file_write` | 写文件 | path, content, encoding? | `/files?action=SaveFileBody` |
| `baota_file_delete` | 删文件 | path | `/files/delete_file` |
| `baota_file_upload` | 分片上传（2MB/片） | local_path, remote_dir, chunk_size_mb? | `/files/upload` |
| `baota_system_info` | 系统概览 | — | `/system/GetServiceStatus` |
| `baota_network_info` | 网络流量 | — | `/system/GetNetWork` |
| `baota_exec` | 面板终端执行（白名单 + 二次确认） | cmd | `/system?action=ExecShell` |

安全约束：凭据只走环境变量/会话文件（`BAOTA_BASE_URL`/`BAOTA_SESSION_FILE`/`BAOTA_REQUEST_TOKEN`）；`BAOTA_DRY_RUN` 默认 true；`BAOTA_CONFIRM_EXEC` 默认 true + exec 只读命令白名单；SSE 仅监听 127.0.0.1。
未覆盖（声明为不支持）：SSL 证书、计划任务、防火墙、反向代理、站点日志、PHP 版本热切、数据库备份/删除、站点删除。

## 3. Bloom 模式整合（现状 + 本阶段增量）

Bloom = 「AI 智能融合路由」下一模式，整合五类能力：

| 能力来源 | Bloom 内承载 |
|---|---|
| GUI_Agent MAX 模式 | `BloomPrivileges`（GWWC MaxModePrivileges Rust 化）、`ReasoningRunner`（Generation→Self-Critique→Refinement→Verification）、`BloomOrchestrator` Captain+Copilot |
| deepseek-TUI YOLO 模式 | `bloom.yaml` auto_approve + skip_permissions 门控（yolo 为 legacy 别名映射 Bloom）、`bloom_auto.yaml` 预算闭环 |
| opensquilla 融合路由 | pyengine 四态路由第 4 档 `bloom` = 融合路由 + `orchestration_mode=auto` + 强制 Router 底座（`model_routing.py`） |
| 内置可控 Agent 浏览器操控 | `browser` 工具 16 action：a11y（非视觉）/ vision+vision_click（VLM 视觉定位）/ relay（CDP 桥真实 Chrome）/ click_ref |
| 第三方软硬件 & 系统级键鼠 | `system_control`（user32/SendInput）：mouse_move/click/scroll/key_press/type_text；**本阶段新增贝塞尔拟人化轨迹移动（`humanize` 参数，默认开）** |

## 4. 本阶段缺陷修复

### 4.1 实时更新不可用（WS/SSE 断线重连 + 消息重放）
- **根因**：断线瞬间在途请求被直接 reject（`Not connected`），且重连退避无抖动、error-only 场景无兜底 → 状态横幅「实时更新暂不可用，请重新连接后再发送」。
- **修复**（`webui/src/lib/rpc.ts`）：
  - 新增 **outbox**：断线期间的幂等调用入队，hello 握手成功后按 FIFO 重放（`queue:false` 可关闭，写型调用仍由上层 durable outbox 重放）。
  - 重连退避加 **抖动**（0.8–1.0 倍，防重连风暴）。
  - `onerror` 若连接仍 OPEN 则主动 close → 统一进入 onclose 重连路径。
- **服务端**（`core/src/gateway/server.rs`）：新增 **180s 空闲读超时看门狗**，客户端静默死亡时主动断开，防僵尸连接。
- 测试：`webui` rpc.test.ts 20/20（含 2 个新增 outbox 用例）。

### 4.2 `sandbox.policy.get` 补齐 + 重试/错误回传
- 现状：`sandbox.policy.get/defaults/update`、`sandbox.run_mode.preference.*`、`sandbox.capability.status`、`sandbox.setup.status` 已在 `RUST_FIRST_METHODS`（Rust 确定性实现，`core/src/sandbox/policy_store.rs` 落盘 `sandbox-policy.json`），UI 沙箱设置页正常消费。（用户此前观察到的 NOT_IMPLEMENTED 来自旧构建产物。）
- 本阶段增强：
  - `RpcError::with_retryable()`（`core/src/gateway/protocol.rs`）：瞬时故障显式标记可重试。
  - `py_call_timeout` 超时错误标记 `retryable=true, retry_after_ms=800`（`methods.rs`）。
  - `useRpcCall` 对 retryable 错误**退避重试一次**（`webui/src/composables/useRpc.ts`）。
- 顺带修复：`core/src/orchestration/mod.rs` `connect_configured_mcp_clients` 既有 E0382 所有权编译错误。

### 4.3 复杂任务自动拆解 + 长续航（本阶段新增）
- **问题**：不论何种模式，简单任务（查资料/写日报）可用，但复杂任务（编程/项目/科研）不会自动拆解、不能长续航——规划/长任务零件（`plans.*`、`LongRunningScheduler`、上下文压缩）此前未接入聊天主路径。
- **实现**（`core`）：
  - `core/src/plan/runner.rs`（新增）`run_autonomous`：复杂任务计划驱动长执行闭环——自动拆解（`plans.create`）→ 发布 `session.event.plan_revision`（前端 PlanRunRibbon 自动出现）→ `plans.run` 启动 → 长时任务注册（心跳/检查点）→ 逐步执行（每步 Orchestrator 子任务驱动，进度经 `session.event.plan_run` 实时推送）→ 步骤间上下文渐进压缩防窗口爆掉 → 步骤失败发布错误并终止（已完成结论仍回传）。
  - `core/src/gateway/chat.rs`：`chat.send` 主路径接入 `should_auto_decompose`（纯规则复杂度判定），复杂任务自动进入计划驱动执行；简单/中等任务完全保持原单轮路径（向后兼容）。
  - `core/src/gateway/methods.rs`：新增 `long_running.list / pause / resume / checkpoint` RPC（暴露 `LongRunningScheduler`：心跳/检查点/暂停/续跑）。
  - `core/src/persistence/config.rs`：新增 `[plan]` 配置段（`auto_decompose` 默认 true、`max_steps` 12、`checkpoint_interval_steps`、`heartbeat_interval_s`）。
  - `core/src/events/envelope.rs`：`EventType` 新增 `PlanRevision`/`PlanRun`（对齐 Web UI `session.event.plan_revision`/`plan_run` 契约）。
- 测试：`cargo test -p marguerite-core` 全量 1157 通过（新增 runner 4 用例 + long_running 原有用例）。

## 5. 验证方式

| 项 | 命令/脚本 | 预期 |
|---|---|---|
| 宝塔 MCP 协议自测 | `py -3.14 btpanel-mcp/smoke_stdio.py` | PASS（initialize/tools/list/错误/ping） |
| 宝塔 MCP 接入 opensquilla | `py -3.14 scripts/mcp_btpanel_smoke.py`（PYTHONPATH=pyengine/src） | PASS（16 个 `mcp_baota_*` 注册 + 无凭据友好错误） |
| 实时更新修复 | `npx vitest run src/lib/rpc.test.ts` | 20/20 |
| core 编译 | `cargo check -p marguerite-core` | 通过 |
| 沙箱策略 | `cargo test -p marguerite-core sandbox` | 通过 |
| 长续航闭环 | `cargo test -p marguerite-core runner long_running` | 全部通过 |
| GUI 端到端 | 重建 `marguerite-gui.exe` 后启动：① 模型路由弹窗可见第 4 档 Bloom；② 沙箱设置页可读策略；③ 断网恢复后消息不再丢失；④ 发送复杂任务（如「从零构建一个 Rust 编译器项目…」）时聊天区自动出现计划步骤 Ribbon 并逐步推进、进度可见，任务可在 `long_running.list` 查到 | 全部可用 |

## 6. 待办 / 可继续融合

- ds4 本地推理侧车接入 `ModelRouter` fallback（需离线运行时）。
- NexusStar 蜂群**共识加权聚合**（`_compute_consensus`）并入 `model/bloom.rs` swarm。
- 浏览器 **record/replay 序列录放**（BrowserRealControl 范式）并入 `browser` 工具。
- `BloomPrivileges` 补 `skip_safety_precheck` 字段与 `extreme/nexus` 预设宏。
