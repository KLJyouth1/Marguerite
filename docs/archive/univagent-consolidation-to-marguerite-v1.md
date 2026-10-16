# UnivAgent 统一整改架构方案 —— 外部项目能力整合进 Marguerite（Rust 单一事实源）

> 生成日期：2026-08-06  
> 作者：大湾区靓仔（项目总监）  
> 输入：记忆库（2026-08-04/05/06）+ 战略计划 v2.0 + K2.2 重复/冲突/互补矩阵 + K3 全域整合文档 + Marguerite 架构文档 + 各外部项目真实目录实证  
> 状态：**v1.0 草案（待用户确认后进入实施）**  
> 取代关系：本方案** supersede（推翻）战略计划 v2.0 的 D2/D3**（三仓不合并、GUI_Agent 作主仓库）→ 改为 **Marguerite(Rust) 为唯一整合目标，放弃 web 端，GUI_Agent 能力审查后二次开发进 Marguerite**。

---

## 〇、战略转向声明（本次整改的宪法级约束）

| 旧决策（被推翻） | 新决策（本方案） | 理由 |
|---|---|---|
| D2：三仓不合并代码，建统一索引 | **合并到 Marguerite(Rust)**：共享能力以 Rust 原生实现为唯一事实源 | 三套平行实现（Rust/TS/Py）长期漂移成本 > 合并成本；Rust 核心已成熟（79 测试绿） |
| D3：GUI_Agent 作主仓库 | **Marguerite 为唯一主仓库/整合目标** | TUI 原生是 UnivAgent 差异化根；web 端（nexustar-ui/desktop/website/admin）整体放弃 |
| — | **放弃 web 端**：nexustar-ui(React)、desktop(Tauri WebView)、nexustar-website(-next)、admin-backend(Java) 全部不进入整合目标 | 用户明确「放弃 web 端」；web 渲染层与 TUI 形态互斥 |
| — | **不能直接复用 GUI_Agent 代码**：仅审查其**能力/架构**，在 Marguerite 内**原生二次开发**（Rust 重写，非搬运） | 技术栈不同（Rust vs TS/Py/Java）；搬运会带入 web 依赖与既有债务 |

> **整合方法论（贯穿全案）**：  
> ① 以 Marguerite 现有 Rust 核心为底座；  
> ② 共享能力 → 统一抽象进 Marguerite 对应 crate（不再维护 Python/TS 平行版）；  
> ③ 差异能力 → 审查后**原生二次开发**为 Marguerite 新模块（Rust 重写）；  
> ④ web 专属能力（官网/管理后台/WebView 壳）→ 放弃，其账号/会员等数据能力下沉为 Marguerite 原生模块；  
> ⑤ 归档竞品（hermes/deepseek-tui/openclaw）仅采纳**设计模式**，不搬代码。

---

## 一、外部项目完整清单（功能定位 + 核心能力）

> 相对整合目标 Marguerite，以下 9 类均为「外部项目」。逐一说明定位与能力。

### P1 · Marguerite（整合目标 / TUI 核心）`D:\Downloads\Marguerite\marguerite\`
- **定位**：UnivAgent 的 Rust 原生 Agent 框架核心 + 终端工作台。CLI 为第一公民，TUI 为原生形态。
- **核心能力（已原生实现，79 测试绿）**：
  - CLI 路由（`cli/`：argv/route/program，含 doctor/model/mcp/skills/session 子命令）
  - 意图系统（`intent/`：classifier/router/rules）
  - Agent 运行时（`agent/`：状态机 runtime + tool_cycle + subagent）
  - 双模型协作（`model/dual.rs`：规划模型+执行模型）
  - MCP 客户端+服务端（`mcp/`：client/server/registry，ToolExecutor trait）
  - 三层记忆（`memory/`：sensory/short_term(FTS5)/long_term）
  - 幻觉护栏（`guard/`：syntax/semantic/fact/contract/repair 五层）
  - 上下文引擎（`context/`：paging/cache/budget）
  - 技能系统（`skills/`：engine/loader/registry，YAML+远程安装+签名）
  - 插件系统（`plugins/`：meta/on_load/capabilities trait）
  - 持久化（`persistence/`：config/session/task_queue）
  - TUI 工作台（`tui/`：WorkbenchState + EventBus + 36 SlotId 契约）

### P2 · nexustar-api（Python FastAPI 后端）`GUI_Agent/nexustar-api/`
- **定位**：原四合一的「后端真相源」，421 py 文件 / 70 router / 46 tool。**本次整改的能力主供给源**（审查后二次开发进 Marguerite）。
- **核心能力（与 Marguerite 重叠 / 差异并存）**：
  - 认证：`auth.py` + `official_sync.py` + `sync_engine.py`（登录/注册/设备登录/官网同步）— **差异**
  - 双栈切换：`backend_switcher.py` + `local_model_loader.py`（线上/本地自动降级+自研模型加载）— **差异**
  - 会话/流式：`agent.py`/`chat`/`session_restorer` + SSE/WS — **差异（传输层）**
  - MAX 双带队：`max_orchestrator.py` + `model_pairing.py` + `expert_team_orchestrator.py` — **差异**
  - 编排/集群：`orchestration/` + `cluster.py` + `allocator.py` + `worker_pool.py` — **差异**
  - 工具（46）：`tools/`（exec/browser/file/terminal/research/rollback/sandbox）— **差异（广度）**
  - 记忆：`memory_layers.py`(4 层) + `project_memory.py` + `memory_consolidator.py` — **重叠（Marguerite 三层）**
  - 护栏：`guard.py`/`guard_gateway.py`/`hallucination.py`/`code_contract.py`/`auto_fix_loop.py` — **重叠（Marguererite 五层）**
  - 意图：`intent_analyzer.py`/`intent_router.py` — **重叠**
  - 上下文：`context_*.py`/`context_engine.py`/`context_paging.py` — **重叠**
  - 技能：`skills/` + `skills_security.py` + `skill_switcher.py` — **重叠**
  - 插件：`plugins.py` — **重叠**
  - 知识库/RAG：`kb_*.py`/`knowledge_graph.py`/`rag_*.py` — **差异**
  - 安全扫描：`malicious_code_scanner.py`/`ssrf_guard.py`/`security_scanner_l2/l3.py`/`operation_guard.py` — **差异**
  - 恢复/快照：`recovery/` + `session_restorer` — **差异**
  - 自动化：`automations` + `control_tool.py` + `control_recorder.py`（UIA 桥）— **差异**
  - 多模态：`vision.py`/`vision_adapter.py` — **差异**
  - 研究：`research.py`/`research_pipeline.py`/`codebase_search.py` — **差异**
  - 治理：`plan_gate.py`/`plan_mode.py`/`constitution.py`/`decision_protocol.py` — **差异**
  - 协议：`acp/`（Agent-Client-Protocol）— **差异**
  - 可观测/事件：`event_bus.py`/`eventbus.py`/`observability.py` — **差异**
  - 领域模型：已存在 `unified_domain.py`（可作 Marguerite 统一领域模型参考）
  - 财务路由：`fin_router.py`（领域特定，低优先）

### P3 · nexustar-ui（React/Tauri GUI 前端）`GUI_Agent/nexustar-ui/`
- **定位**：原四合一的 GUI 形态（355 tsx 文件）。**web 端 → 放弃**。
- **核心能力（仅提取逻辑，不搬代码）**：
  - 聊天体验 / MAX 双带队 UI / 编排 DAG 画布 / 音乐粒子(Bloom)系统 / 60+ 屏幕 / 38 store / 4 主题
  - **整改处理**：UI 渲染层丢弃；其底层**能力逻辑**（编排模型、音频 FFT 分析、状态机、主题令牌）审查后**原生二次开发**进 Marguerite TUI/Rust。Bloom 视觉身份**改造为 TUI 原生美学**（终端色彩/动画，非 WebGL）。

### P4 · desktop（Tauri 桌面壳）`GUI_Agent/desktop/`
- **定位**：Tauri WebView 原生壳（src-tauri + sidecar + sandbox-cli）。**web 端 → 放弃**。
- **核心能力**：文件网关（`fs_*`）、sidecar（UIA 自动化）、托盘、原生对话框。
- **整改处理**：窗口/对话框壳丢弃；文件网关 → Marguerite 文件系统工具；UIA 自动化（RPA）→ 评估为 Marguerite 原生自动化模块（超出纯 TUI 范围，低优先）。

### P5 · admin-backend（Java SpringBoot 管理后台）`GUI_Agent/admin-backend/`
- **定位**：官网账号/会员/库表统管（pom.xml，Java）。**web 端 → 放弃**。
- **核心能力**：用户管理、会员定价、库表管理。
- **整改处理**：Java 后台整体放弃；其**账号单一事实源**能力下沉为 Marguerite 原生 `auth/` 模块（D9 本地优先）。会员/定价逻辑按需二次开发进 Marguerite（P2 阶段）。

### P6 · nexusops（跨仓运维脚本）`GUI_Agent/nexusops/`
- **定位**：部署/启动/运维工具（api/desktop/nexustar-ui/scripts）。
- **核心能力**：运维/启动/部署编排。
- **整改处理**：web 运维层丢弃；部署/自检逻辑二次开发为 Marguerite CLI 子命令（复用现有 doctor 模式）。

### P7 · nexustar-website / -next（官网）`GUI_Agent/nexustar-website(-next)/`
- **定位**：技术交流区+聊天室+会员中心（Vite / Next.js）。**web 端 → 放弃**。
- **整改处理**：整体放弃。社区/交流需求不在 TUI 整合范围（或后置为可选独立服务，不进 Marguerite 主仓）。

### P8 · gwwc（遗留）`GUI_Agent/gwwc/`
- **定位**：GWWC 遗留（skills/ + web/）。
- **核心能力**：遗留技能包、遗留 web。
- **整改处理**：web 丢弃；遗留技能清单审查后并入 Marguerite `skills/`。

### P9 · _GUI_Agent_archived/external/*（竞品参考）`D:\Downloads\_GUI_Agent_archived_20260713\`
- **定位**：只读归档的竞品源码（hermes-agent / deepseek-tui / openclaw / opencode / claude-mem 等）。
- **核心能力（仅采纳设计模式，不搬代码）**：
  - **hermes**：5 层记忆 + FTS5 + 自进化闭环（技能自动生成）
  - **deepseek-tui**：`agent_spawn` 取代固定 swarm 实体 / LSP 后编辑诊断 / `web_run` 无头浏览 / OS 级沙箱（Landlock/Seatbelt）
  - **openclaw**：Gateway + 多 Runtime 插件化 / ClawHub 技能市场治理（版本化+哈希校验）
- **整改处理**：作为 Marguerite 二次开发的设计范本（见第三节「采纳模式」）。

---

## 二、跨项目能力重叠 / 冲突 / 差异分析

### 2.1 重叠（Marguerite Rust 已有原生等价，Python/TS 为平行实现 → 统一进 Rust）
| 能力 | Marguerite(Rust) | 外部平行实现 | 统一动作 |
|---|---|---|---|
| 意图 | `intent/` | nexustar `intent_analyzer/router` | 以 Rust 为 SSOT，弃 Python |
| Agent 运行时 | `agent/`(runtime/tool_cycle/subagent) | nexustar `agent_runtime/subagent/tool_cycle` | 以 Rust 为 SSOT |
| 双模型 | `model/dual.rs` | nexustar `dual_model/model_pairing` | 以 Rust 为 SSOT，吸收 model_pairing 配对策略 |
| 记忆 | `memory/`(3 层) | nexustar `memory_layers`(4 层)+`project_memory` | Rust 升为 4 层+项目记忆（采纳 hermes） |
| 护栏 | `guard/`(5 层) | nexustar `guard/hallucination/code_contract/auto_fix_loop` | Rust 吸收 code_contract/auto_fix_loop |
| 上下文 | `context/` | nexustar `context_*.py` | 以 Rust 为 SSOT |
| 技能 | `skills/` | nexustar `skills/`+`skills_security` | Rust 吸收 skills_security（供应链检测） |
| 插件 | `plugins/` | nexustar `plugins.py` | 以 Rust 为 SSOT |
| 持久化/会话 | `persistence/` | nexustar `session_restorer/task_queue/scheduler` | Rust 吸收 scheduler/long_running |

### 2.2 差异（外部有、Marguerite 缺 → 审查后原生二次开发进 Rust）
| 能力 | 来源 | Marguerite 目标模块 | 优先级 |
|---|---|---|---|
| 账号认证+官网同步 | nexustar `auth/official_sync` | `auth/`(本地优先 D9) | P0 |
| 双栈切换+本地模型降级 | nexustar `backend_switcher/local_model_loader` | `backend/` | P0 |
| 会话流式传输(SSE/WS) | nexustar `agent/chat` | `transport/`(服务态) | P0 |
| MAX 双带队+专家团队 | nexustar `max_orchestrator/expert_team_orchestrator` | `orchestration/`(扩展 agent) | P0 |
| 集群/分配/worker池 | nexustar `cluster/allocator/worker_pool` | `orchestration/` | P1 |
| 工具广度(46) | nexustar `tools/` | 原生工具 behind `ToolExecutor` trait / MCP server | P1 |
| KB/RAG+知识图谱 | nexustar `kb_*/knowledge_graph/rag_*` | `kb/` | P1 |
| 恶意代码/SSRF/操作护栏 | nexustar `malicious_code_scanner/ssrf_guard/security_scanner_*` | `security/` | P1 |
| 恢复/快照/回滚 | nexustar `recovery/session_restorer` | `recovery/` | P1 |
| 自动化/UIA 录制回放 | nexustar `automations/control_tool` | `automation/`(TUI 原生/RPA) | P2 |
| 多模态视觉 | nexustar `vision/vision_adapter` | `vision/` | P2 |
| 研究管线/代码检索 | nexustar `research/research_pipeline/codebase_search` | `research/` | P2 |
| 治理(计划门/宪法/决策协议) | nexustar `plan_gate/constitution/decision_protocol` | `governance/` | P2 |
| ACP 协议 | nexustar `acp/` | `serve/acp` | P2 |
| 可观测/事件总线服务 | nexustar `event_bus/observability` | `serve/observability`(复用 TUI EventBus) | P2 |
| 财务路由 | nexustar `fin_router` | 领域特定，暂缓 | P3 |

### 2.3 冲突（需消解的架构二义性）
| 冲突 | A | B | 统一裁决 |
|---|---|---|---|
| 账号事实源 | nexustar `auth`(Py) + admin(Java) 双写 | Marguerite 本地优先(D9) | **Marguerite `auth/` 为唯一事实源**；Java/Py 后端仅作同步目标（甚至随 web 放弃而消失） |
| 记忆层数 | Marguerite 3 层 | nexustar 4 层+项目记忆 | **统一 4 层 + 项目记忆**（采纳 hermes/nexustar 更优模型） |
| 蜂群模型 | nexustar `swarmStore` 固定实体 | deepseek-tui `agent_spawn` | **采纳 spawn 模型**（D7 已决） |
| 状态/事件机制 | nexustar 5 套状态机制+规划 EventBus | Marguerite 单向归约+EventBus | **Marguerite WorkbenchState+EventBus 为唯一机制** |
| 视觉引擎 | nexustar WebGL/Three.js Bloom | Marguerite 终端渲染 | **Bloom 改造为 TUI 原生美学**（终端色彩/动画，非 WebGL） |

### 2.4 互补（archived 竞品 → 设计模式采纳，非代码）
- hermes → 5 层记忆 + 自进化闭环
- deepseek-tui → spawn 模型 + LSP 诊断 + OS 沙箱 + web_run
- openclaw → Gateway/runtime 拆分 + 技能市场治理

---

## 三、融合方案 —— 统一抽象 / 标准化整改 + 差异能力二次改进

### 3.1 共有能力：统一抽象与标准化（Marguerite 为 SSOT）
1. **统一领域模型**：以 nexustar `unified_domain.py` 为参考，在 Marguerite 建 `domain/` crate（Message/Session/Agent/Task/Tool/Memory/Account 等），全仓（CLI/TUI/transport/工具）共用同一类型。
2. **统一 Agent 运行时**：合并 Marguerite `agent/` + nexustar `agent_runtime/subagent/tool_cycle/max_orchestrator` → 单一 `agent/` crate，支持单 agent / 双带队 / 团队（spawn 模型）。
3. **统一记忆**：Marguerite `memory/` 升为 **4 层 + 项目记忆**（sensory/short_term/project/long_term），采纳 hermes 自进化闭环（技能自动生成）。
4. **统一护栏**：Marguerite `guard/` 吸收 nexustar `code_contract`(契约解析) + `auto_fix_loop`(修复循环) + `skills_security`(供应链检测)。
5. **统一工具抽象**：所有 46 工具以 Marguerite `mcp::ToolExecutor` trait 原生实现（或暴露为 MCP server），消除「MCP 工具 vs 内部工具」双轨。
6. **统一事件/状态**：Marguerite `WorkbenchState` + `EventBus` 为全形态唯一机制；TUI/CLI/transport 三态共享 SlotId + 事件类型。

### 3.2 差异能力：二次改进与增强（原生 Rust 重写）
- **`auth/`**：本地优先账号（D9），Rust 实现 SQLite `user_id` 分区 + 设备登录 + 可选官网同步（web 放弃后仅本地）。**改进**：比 Python 版更强的类型安全与离线优先。
- **`backend/`**：双栈切换（线上/本地）+ 本地模型加载器（llamacpp 集成）。**改进**：借鉴 deepseek-tui OS 沙箱模型做模型加载隔离。
- **`transport/`**：在 Marguerite 增加 **服务态**（SSE/WS HTTP server），使其既能 CLI/TUI 又能作后端服务（取代 nexustar-api 的传输职责）。**改进**：单一二进制兼具客户端与服务端。
- **`orchestration/`**：MAX 双带队 + 专家团队 + 集群调度，统一走 spawn 模型。**改进**：采纳 deepseek-tui LSP 后编辑诊断（接 Marguerite `guard/syntax` tree-sitter）。
- **`kb/`**：RAG + 知识图谱，原生 Rust（向量检索用 sqlx + 本地 embedding）。
- **`security/`**：恶意代码扫描（YARA/正则）、SSRF 守卫、操作护栏——**改进**：采纳 deepseek-tui OS 级沙箱（Landlock/Seatbelt）替代纯 denylist（hermes 反模式）。
- **`recovery/`**：快照/回滚，原生 Rust + SQLite WAL。
- **`automation/`**：录制/回放 + UIA 桥——**改进**：TUI 原生或 sidecar，审批门 + 操作审计（采纳 openclaw exec 审批门）。
- **`vision/`**：多模态，原生 Rust（图像理解走模型 API）。
- **Bloom TUI 原生化**：音乐/粒子系统**二次改造**——音频 FFT 分析用 Rust 音频库（如 cpal/rodio）实现，可视化改为终端渲染（crossterm 色彩/Unicode 块粒子/braille），保留 Bloom 视觉身份（品牌色 + 粒子溶解隐喻）但脱离 WebGL。**改进**：零 WebGL 依赖，纯终端可运行。

### 3.3 采纳的竞品设计模式（仅模式，不搬代码）
| 模式 | 来源 | 落点 |
|---|---|---|
| 5 层记忆 + 自进化闭环 | hermes | `memory/` 升级 |
| spawn 模型取代固定 swarm | deepseek-tui | `orchestration/` |
| LSP 后编辑诊断 | deepseek-tui | `guard/syntax` + `orchestration` |
| OS 级沙箱(Landlock/Seatbelt) | deepseek-tui | `security/` + `tools/` 执行隔离 |
| Gateway + 多 Runtime 插件化 | openclaw | `serve/` + `plugins/` |
| 技能市场治理(版本化+哈希) | openclaw ClawHub | `skills/` registry |

---

## 四、统一整改后架构（Marguerite 为唯一二进制/唯一事实源）

```
┌──────────────────────────────────────────────────────────────┐
│                   Marguerite (Rust 单一二进制)                    │
│   形态：CLI（第一公民） + TUI（原生工作台） + 服务态(SSE/WS)        │
├──────────────────────────────────────────────────────────────┤
│ [L0] domain/        统一领域模型（Message/Session/Agent/Task/    │
│                        Tool/Memory/Account）—— 全仓共用类型        │
├──────────────────────────────────────────────────────────────┤
│ [L1] cli/            argv/route/program（doctor/model/mcp/      │
│                        skills/session/auth/serve 子命令）        │
│ [L1] tui/            WorkbenchState + EventBus + 36 SlotId       │
│                        + Bloom TUI 原生美学（终端粒子/色彩）        │
├──────────────────────────────────────────────────────────────┤
│ [L2] intent/         classifier/router/rules                    │
│ [L2] agent/          运行时 + tool_cycle + subagent + 双带队/团队 │
│ [L2] model/          llm/router/dual + backend/(双栈+本地模型)    │
│ [L2] orchestration/  MAX 双带队 + 专家团队 + 集群/分配(spawn 模型) │
├──────────────────────────────────────────────────────────────┤
│ [L3] mcp/            client/server/registry + ToolExecutor      │
│ [L3] tools/          46 工具原生实现（exec/file/terminal/browser/ │
│                        research/rollback）—— behind ToolExecutor │
│ [L3] skills/         engine/loader/registry + 供应链检测          │
│ [L3] plugins/        meta/on_load/capabilities                  │
├──────────────────────────────────────────────────────────────┤
│ [L4] memory/         4 层 + 项目记忆 + 自进化闭环                  │
│ [L4] guard/          5 层 + code_contract + auto_fix + skills_sec │
│ [L4] context/        paging/cache/budget                        │
│ [L4] kb/             RAG + 知识图谱                              │
│ [L4] security/       恶意代码/SSRF/操作护栏 + OS 沙箱             │
│ [L4] recovery/       快照/回滚                                   │
│ [L4] automation/     录制/回放 + UIA 桥                          │
│ [L4] vision/         多模态                                     │
│ [L4] research/       研究管线/代码检索                           │
│ [L4] governance/     计划门/宪法/决策协议                         │
├──────────────────────────────────────────────────────────────┤
│ [L5] auth/           本地优先账号（D9）                           │
│ [L5] persistence/    config/session/task_queue/scheduler        │
│ [L5] transport/      SSE/WS 服务态（取代 nexustar-api 传输）      │
│ [L5] serve/          ACP + observability + health               │
└──────────────────────────────────────────────────────────────┘
```

**关键变化 vs 现状**：
- Marguerite 从「CLI+TUI 引擎」升级为「CLI+TUI+服务 三态统一 Agent 框架」。
- 原 nexustar-api 的**传输/账号/双栈/编排/工具/KB/安全/恢复**职责被 Marguerite 原生模块取代。
- web 端（nexustar-ui/desktop/website/admin）整体消失；其数据能力（账号/会员）下沉为 Marguerite `auth/`。

---

## 五、能力映射说明（外部能力 → Marguerite 目标模块 → 动作）

| 外部能力（来源） | Marguerite 目标模块 | 动作 |
|---|---|---|
| 意图(intent_analyzer/router) | `intent/` | 统一（Rust SSOT） |
| Agent 运行时(agent_runtime/subagent/tool_cycle) | `agent/` | 统一 |
| 双模型(dual_model/model_pairing) | `model/dual` + `model/backend` | 统一+吸收配对 |
| 三层记忆(memory_layers 4层+project) | `memory/`(升 4 层) | 统一+升级 |
| 护栏(guard/hallucination/code_contract/auto_fix) | `guard/` | 统一+吸收 |
| 上下文(context_*) | `context/` | 统一 |
| 技能(skills/+skills_security) | `skills/` | 统一+吸收供应链检测 |
| 插件(plugins.py) | `plugins/` | 统一 |
| 持久化(session_restorer/task_queue/scheduler) | `persistence/` | 统一+吸收 scheduler |
| 账号认证(auth/official_sync) | `auth/` | **二次开发**（本地优先） |
| 双栈切换(backend_switcher/local_model_loader) | `backend/` | **二次开发** |
| 会话流式(agent/chat SSE/WS) | `transport/` | **二次开发**（服务态） |
| MAX 双带队(max_orchestrator/expert_team) | `orchestration/` | **二次开发** |
| 集群/分配(cluster/allocator/worker_pool) | `orchestration/` | **二次开发** |
| 46 工具(tools/) | `tools/`(ToolExecutor/MCP) | **二次开发** |
| KB/RAG(kb_*/knowledge_graph/rag_*) | `kb/` | **二次开发** |
| 安全扫描(malicious/ssrf/security_scanner) | `security/` | **二次开发**+OS 沙箱 |
| 恢复(recovery/session_restorer) | `recovery/` | **二次开发** |
| 自动化(automations/control_tool) | `automation/` | **二次开发**(RPA) |
| 多模态(vision) | `vision/` | **二次开发** |
| 研究(research/codebase_search) | `research/` | **二次开发** |
| 治理(plan_gate/constitution/decision_protocol) | `governance/` | **二次开发** |
| ACP(acp/) | `serve/acp` | **二次开发** |
| 可观测(event_bus/observability) | `serve/observability` | **二次开发**(复用 EventBus) |
| Bloom 音乐/粒子(WebGL) | `tui/`(Bloom TUI 原生) + `audio` 模块 | **二次改造**(去 WebGL) |
| 官网(website/-next) | — | **放弃** |
| 桌面壳(desktop Tauri WebView) | — | **放弃**（文件网关→tools，UIA→automation） |
| 管理后台(admin-backend Java) | `auth/`(账号下沉) | **放弃 Java**，能力下沉 |
| 运维(nexusops) | `cli/`(doctor 式子命令) | **二次开发**(CLI 化) |
| gwwc 技能 | `skills/` | 审查并入 |
| hermes/deepseek-tui/openclaw 模式 | 各对应模块 | **采纳模式**(不搬代码) |

---

## 六、实施统一整改的分期计划（二次开发路线）

> 每期均以 Marguerite `cargo build/test` 零错误零警告 + 79+ 测试为门禁；web 端项目同步冻结。

| 期 | 名称 | 动作 | 取代的外部能力 | 门禁 |
|---|---|---|---|---|
| **A** | 地基：统一领域模型 + 确认 SSOT | 建 `domain/`；确认 intent/agent/memory/guard/context/skills/plugins 以 Rust 为唯一实现（弃 Python 平行版） | nexustar 重叠模块 | cargo 测试全绿 |
| **B** | 服务态 + 账号 + 双栈 | 新增 `transport/`(SSE/WS) + `auth/`(本地优先) + `backend/`(双栈+本地模型) | nexustar auth/backend_switcher/session 流式 | doctor 自检通过 |
| **C** | 编排：双带队 + 团队 + 集群 | 扩展 `agent/` → `orchestration/`(MAX+专家团队+spawn 模型) | max_orchestrator/expert_team/cluster | 编排单测 |
| **D** | 工具 / KB / 安全 | `tools/`(46 原生) + `kb/`(RAG) + `security/`(扫描+OS 沙箱) | tools/kb_*/malicious_code_scanner/ssrf | 工具/安全单测 |
| **E** | 差异能力 | `recovery/` + `automation/`(RPA) + `vision/` + `research/` + `governance/` + `serve/`(ACP/observability) | recovery/automations/vision/research/plan_gate/acp | 各模块单测 |
| **F** | Bloom TUI 原生化 | 音频 FFT(Rust) + 终端粒子/色彩美学，去 WebGL | nexustar-ui Bloom/音乐 | TUI 视觉门禁 |
| **G** | 退役 | 冻结并归档 nexustar-ui/desktop/website/admin-backend/gwwc web；nexustar-api 仅留作参考 | 全部 web 端 | 无回归 |

---

## 七、决策记录（新增）

| # | 决策 | 原因 | 影响 |
|---|---|---|---|
| **D12** | **推翻 D2/D3：Marguerite(Rust) 为唯一整合目标，放弃 web 端** | 用户明确「放弃 web 端」「不能直接复用 GUI_Agent 代码，审查后二次开发到 Marguerite」；三套平行实现长期漂移成本过高；Rust 核心已成熟 | nexustar-ui/desktop/website/admin-backend 不进入整合；其数据能力下沉 Marguerite；GUI_Agent 代码仅作能力/架构审查参考，禁止直接搬运 |
| **D13** | **共享能力以 Marguerite Rust 为 SSOT，弃 Python/TS 平行实现** | 避免三态漂移；Rust 已实现意图/Agent/双模型/MCP/记忆/护栏/上下文/技能/插件/持久化 | nexustar-api 重叠模块退役；仅差异能力二次开发进 Rust |
| **D14** | **Bloom 视觉身份改造为 TUI 原生（去 WebGL）** | web 端放弃，WebGL 不可用于 TUI；保留品牌色+粒子溶解隐喻 | 音频 FFT 用 Rust 音频库；可视化改终端渲染 |

---

## 八、产品定义拍板（项目总监决策，用户授权"全程拍板"）

> 用户原话：「你全程拍板，我只要你确定做得和 codex、cursor 一样的工具、桌面端，而且能力和模块、工具、逻辑比他们还厉害数倍的 GUI 和 TUI 版。」

据此锁定以下不可复议决策：

| # | 决策 | 含义 |
|---|---|---|
| **D15** | **产品定位 = 桌面 AI 编码工具（类 Codex/Cursor），双原生前端** | Marguerite 不再是"框架/引擎"，而是**成品**：TUI（Ratatui，类 Codex）+ 桌面 GUI（原生 egui，类 Cursor）。两前端共用一个 Rust 引擎，无 web。 |
| **D16** | **GUI 技术 = 原生 egui/eframe（非 Tauri/WebView）** | 严格遵循"放弃 web"：不嵌入浏览器内核，单一原生二进制，直接 `use marguerite_core`。首版 `desktop/` crate 已落地并挂载到 workspace。 |
| **D17** | **能力必须数倍于 Codex/Cursor** | 以 `domain/capabilities.rs` 注册表为具象化标准：在 CoreLoop（等价）之上，强制包含 Orchestration(双带队/专家团队/集群/swarm)、46 工具原生集、KB/RAG、OS 级沙箱、自动化/RPA、多模态、研究管线、治理门禁、韧性恢复、双栈模型、5 层记忆——这些在 Codex/Cursor 中不存在（标记 Exclusive）。 |
| **D18** | **范围 = 9 类全纳入；边整边冻 web** | gwwc/nexusops 吸收为技能/CLI；web 端（nexustar-ui/desktop Tauri/website/admin-backend）在对应 Rust 模块落地后立即冻结，不再统一留到最后。 |

**起步动作（已执行）**：A 期首步——`core/src/domain/` 统一领域模型 + `capabilities.rs` 能力注册表（超集具象化）；B 期前置——`desktop/` 原生 GUI crate 挂载并渲染能力树；TUI 沿用现有 Ratatui workbench。下一轮进入 B 期（transport/auth/backend）与 C 期（orchestration）的实质二次开发。

*文档结束。本方案为 UnivAgent 整合的战略转向落地蓝本；决策 D12–D18 已锁定，实施按第六期计划推进，当前进度见 `marguerite-verify/` 增量构建。*
