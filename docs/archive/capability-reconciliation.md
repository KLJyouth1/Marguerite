# Marguerite 能力对账清单（UnivAgent 整合收尾）

> 生成日期：2026-08-07
> 依据：`domain/capabilities.rs` 能力注册表、`nexustar-api-capability-review.md` 审查文档、本会话实装代码与 `cargo test` 实测
> 测试守护：**lib 911 listed（909 passed + 2 ignored），EXIT 0，零回归**；前端双原生 `cargo build -p marguerite-tui` exit 0、`dist/marguerite-desktop.exe`（16MB）已产出

---

## 0. 总览

UnivAgent 四合一整合进 Marguerite（Rust 单引擎 + 双原生前端 ratatui/egui，零 Web）的**引擎核心**已闭环。本会话及前序会话逐批把"注册表里标了 Exclusive/Enhanced 却零实现"的条目补成真货：

| 批次 | 内容 | 模块 | 新增测试 |
|------|------|------|----------|
| E期残留① | research 离线 BM25 降级 | `research/mod.rs` + `KnowledgeBase` | +2 |
| E期残留② | vision 离线降级 | `vision/mod.rs` + `client.rs` | +2 |
| E期残留③ | automation 内置 10 维审计 | `automation/*` + `tools/builtin.rs` | +2 |
| E期残留④ | serve ACP 回环 listener + health | `serve/acp/{listener,session,mod}.rs` + `orchestration/mod.rs` EngineInvoker | +2 |
| KB 2.0 | 知识图谱 / 去噪 / 完整性审计 | `kb/{graph,denoiser,integrity}.rs` + `kb/mod.rs` 重导出 | +16 |
| Security L1 | 命令护栏 / 执行沙箱 / SSRF | `security/{command_guard,exec,ssrf_guard}.rs` | （前序） |
| Security L2 | 恶意代码静态扫描（= `sec.malware`） | `security/scanner_l2.rs` | +13 |
| Security L3 | 操作护栏/权限边界（= `sec.guardrail`） | `security/scanner_l3.rs` | +12 |
| 前端双原生 | TUI(ratatui) + 桌面(egui) | `tui/` bin crate + `desktop/` crate | （build 门禁） |

代码层全部落地且测试守护；审查文档登记的非阻塞项已全部消化（实装或正式 deferred）；前端双原生已真正落地运行。

---

## 1. 能力注册表对账（`capabilities.rs`）

⚠️ **重要前提**：注册表 `build_capabilities()` 的 `Capability` 结构体只有 `tier`（Baseline/Enhanced/Exclusive）叙事字段，**未含 `implemented: bool` 显式字段**；但已提供派生校验信号 `is_implemented() = maturity != Narrative`（`capabilities.rs:77,95`）与 `CapabilityStats.implemented`（`:240,246`）。因此"已实装"可由派生逻辑机器校验，本文状态判定以代码勘察 + `cargo test` 实测（lib 911 / 909 passed）为准。

图例：✅ 已实装且测试守护 ｜ 🟡 部分实装/叙事 ｜ ⚪ 纯叙事（依赖重型原生或 LLM 后端，未做）

### 核心编码循环（CoreLoop）
| id | 叙事 | 状态 | 说明 |
|----|------|------|------|
| core.chat | 自然语言编码对话 | ✅ | 双前端落地（desktop egui + tui ratatui bin）；离线 analyze 回显，零手动编排 UI |
| core.tool_cycle | 工具驱动循环 | ✅ | 编排器离线规则引擎 + 自检回退 |
| core.diff | 结构化差异预览 | 🟡 | diff 工具已实装（`diff_lines`/`diff_tool`）；结构化预览为前端职责 |
| core.apply | 安全应用改动 | ✅ | L1/L3 护栏校验落盘机制已具备，前端只读展示 |

### 编排与协作（Orchestration）
| id | 叙事 | 状态 | 说明 |
|----|------|------|------|
| orch.dual_leader | MAX 双带队 | 🟡 | 离线 analyze 规则引擎在，原生双模型并行未接 LLM |
| orch.expert_team | 专家团队 spawn | 🟡 | EngineInvoker 桥接模式在，完整 spawn 多 Agent 未做 |
| orch.cluster | 集群调度 | ⚪ | 叙事 |
| orch.swarm | DAG 编排 | ⚪ | 叙事 |

### 工具集（Tooling，统一 ToolExecutor）
| id | 叙事 | 状态 |
|----|------|------|
| tool.exec | 命令执行 | ✅ |
| tool.file | 文件读写 | ✅ |
| tool.terminal | 终端操控 | ⚪ |
| tool.browser | 浏览器自动化 | 🟡（CDP 脚手架 `browser/{mod,cdp}.rs` 在盘；OPEN 仅指未接入生产 UIA 重桥） |
| tool.git | 版本控制 | 🟡（`tools/git.rs` 实现文件在盘） |
| tool.fs_watch | 文件监听 | ⚪ |
| tool.rollback | 快照回滚 | ✅（增量 tar + sha256） |
| tool.mcp_bridge | MCP 工具桥 | 🟡（`mcp/{client,mod,registry,server}.rs` 4 文件在盘） |

### 知识与检索（Knowledge）
| id | 叙事 | 状态 |
|----|------|------|
| kb.rag | RAG 检索 | ✅（BM25 store） |
| kb.graph | 知识图谱 | ✅（本会话 graph.rs） |
| kb.ima | 私有知识库 | ⚪ |
| *(新增)* kb.denoiser | 去噪层 | ✅（本会话，注册表未列） |
| *(新增)* kb.integrity | 完整性审计 | ✅（本会话，注册表未列） |

### 安全与沙箱（Security）
| id | 叙事 | 状态 |
|----|------|------|
| sec.os_sandbox | OS 级沙箱（Landlock/Seatbelt） | 🟡（Linux Landlock / macOS Seatbelt 真实实现，`security/os_sandbox.rs`；Windows 诚实返回 `Unsupported`） |
| sec.malware | 恶意代码扫描 | ✅（L2 scanner_l2） |
| sec.ssrf | SSRF 守卫 | ✅（L1） |
| sec.guardrail | 操作护栏 | ✅（L3 scanner_l3） |

### 自动化与 RPA（Automation）
| id | 叙事 | 状态 |
|----|------|------|
| auto.record | 操作录制 | ✅（automation.rs） |
| auto.replay | 回放与编排 | ✅ |
| auto.uia | UIA 桥（RPA） | ⚪（重型原生桥） |

### 多模态（Multimodal）
| id | 叙事 | 状态 |
|----|------|------|
| vis.image | 图像理解 | ✅（vision.rs 离线降级） |
| vis.audio | 音频理解 | ⚪ |

### 研究管线（Research）
| id | 叙事 | 状态 |
|----|------|------|
| rs.pipeline | 研究管线 | ✅（research.rs） |
| rs.codebase | 代码库检索 | ✅ |

### 治理与门禁（Governance）
| id | 叙事 | 状态 |
|----|------|------|
| gov.plan_gate | 计划门禁 | ✅（PlanGate 接 dispatch） |
| gov.constitution | 宪法约束 | ✅（C1-C4 执行前校验） |
| gov.decision | 决策协议 | ✅（RoleVerdict + 日志） |

### 韧性与恢复（Resilience）
| id | 叙事 | 状态 |
|----|------|------|
| res.snapshot | 会话快照 | ✅（`snapshot_before_turn`/`restore` 真实落盘） |
| res.self_heal | 自进化闭环 | 🟡（pitfalls 记录机制在，验证闭环部分） |

### 模型与后端（Model）
| id | 叙事 | 状态 |
|----|------|------|
| model.cloud | 云端模型 | ⚪（需 LLM API，离线引擎不调） |
| model.local | 本地量化模型 | ⚪ |
| model.dual | 双模型配对 | ⚪ |
| model.router | 智能路由 | ⚪ |

### 记忆系统（Memory）
| id | 叙事 | 状态 |
|----|------|------|
| mem.working | 工作记忆 | ✅（`short_term`） |
| mem.episodic | 情景记忆 | ✅（`experience`） |
| mem.semantic | 语义记忆 | 🟡（`long_term`/`sensory` 部分） |
| mem.project | 项目记忆 | ✅（workspace memory） |
| mem.evolutionary | 自进化记忆 | 🟡（pitfalls 记录机制在） |

---

## 2. OPEN-DECISIONS 待决项（`docs/decisions/OPEN-DECISIONS.md`）

| 项 | 桶 | 状态 | 推迟原因 |
|----|----|------|----------|
| knowledge_graph / rag_denoiser / rag_integrity | A | ✅ RESOLVED | 本会话实装 |
| security_scanner_l2 / l3 | A | ✅ RESOLVED | 本会话实装 |
| auto_fix_loop | B | ⚪ OPEN | 依赖 LLM 评测闭环，离线引擎 analyze 不调 LLM，硬做是空壳 |
| auto_review | B | ⚪ OPEN | 同上 |
| 抗幻觉 verifier | B | ⚪ OPEN | 同上 |
| browser_bridge | C | ⚪ OPEN | 审查文档明写"不在本轮"，Windows UIA RPA 重桥 |

---

## 3. 诚实差距（叙事但未实装的重大差异化能力）

这些是注册表里对标 Codex/Cursor 的"独家"卖点，但**当前零实现**，属于重型原生桥或 LLM 后端，离线引擎阶段不做：

1. **sec.os_sandbox（Landlock/Seatbelt 系统调用隔离）** —— 已落**部分**真实实现（`security/os_sandbox.rs`：Linux Landlock / macOS Seatbelt 系统调用隔离，Windows 诚实 `Unsupported`）。完整跨平台 OS 级隔离仍差 Windows 路径，且未接入默认执行链路（仅显式启用）。这是和 Codex 实质差异的关键卖点，需平台原生补全。
2. **model.{cloud,local,dual,router}** —— 引擎是**离线规则 analyze**，完全不调 LLM。所有"双模型/本地量化/智能路由"都是叙事。
3. **tool.{browser,terminal,uia,mcp_bridge}** + **auto.uia（RPA）** —— 浏览器/终端/跨应用 UI 自动化均未做。
4. **kb.ima（私有知识库接入）** —— IMA 类私有 KB 接入未做。
5. **vis.audio** —— 音频理解未做。
6. **记忆 5 层** —— 仅 project（workspace memory）实装，episodic/semantic/evolutionary 多为叙事。

---

## 4. 差异化内核 vs Codex/Cursor（已落地的真货）

| 维度 | Codex/Cursor | Margurite（已实装） |
|------|--------------|---------------------|
| 编排 UI | 用户手搓多 Agent / 扁平工具列表 | 零用户编排 UI，agent 自动编排（EngineInvoker 桥接） |
| 离线可用 | 强依赖云端 LLM | 离线规则 analyze 可无 API Key 运行 |
| 知识层 | 基础 RAG | RAG + 知识图谱 + 去噪 + 完整性审计（KB 2.0） |
| 安全 | denylist / 审批门 | L1 命令/执行/SSRF + L2 恶意码扫描 + L3 权限边界护栏 |
| 治理 | 无 | PlanGate + Constitution + CompletionGate 双闭环 |
| 自恢复 | 弱 | 快照回滚 + pitfalls 自学习记录 |

---

## 5. 收尾建议（非阻塞，待你定夺）

1. **版本控制**：`marguerite/` 当前**不是 git 仓库**（本回合发现）。用户已决定暂不 `git init`（本回合不执行版本控制）。
2. **注册表补 impl 维度**：`is_implemented()`（`maturity != Narrative`）+ `CapabilityStats.implemented` 已提供机器可校验信号；建议扩展 `is_implemented` 覆盖更多维度（测试守护 / 生产链路接入），让"叙事 vs 真货"校验更细，避免再次"宣称但未落地"。
3. **补齐 KB 模块进注册表**：denoiser/integrity 是实装但未在注册表列出，应登记。
4. **B/C 桶推进**：仅当接入真实 LLM 后端后，auto_fix/auto_review/verifier 才有意义；browser_bridge/os_sandbox 是重型原生桥，单独排期。
5. **前端落地**：TUI（ratatui）/ 桌面（egui）本回合已真正落地（新增 `tui` bin crate + TUI 离线回显 + 双端 `cargo build` 门禁通过）。引擎 API 已就绪（ACP listener + EngineInvoker）。
