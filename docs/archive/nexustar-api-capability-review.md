# nexustar-api 能力审查与吸收矩阵

> 审查对象：`D:\Downloads\GUI_Agent\nexustar-api`（Python 后端参考件，422 个 .py，冻结保留未删）
> 吸收目标：`D:\Downloads\Marguerite\marguerite`（Rust 单引擎本体）
> 原则：已有能力**不重复造**；仅吸收**真正差异化 + 可单测**的能力，据以落地 E 期剩余差异模块。
> 审查日期：2026-08-06

## 1. 已覆盖（Marguerite Rust 各期已落地，不重复造）

| nexustar-api 模块 | Marguerite 对应（已落地） |
|---|---|
| `agent_runtime.py` / `dual_model.py` / `max_orchestrator.py` / `expert_team_orchestrator.py` | `orchestration/`（双带队/专家团队/集群/spawn，C 期） |
| `auth.py` / `crypto.py` | `auth/`（本地优先账号下沉，**规划中**；截至 2026-08-07 审计 `core/src` 下无 `auth/` 目录，能力暂以 `auth.py`/`crypto.py` 为参照，待落地） |
| `backend_switcher.py` | `model/`（双栈路由） |
| `constitution.py` | `governance/constitution.rs`（E 期） |
| `decision_protocol.py` | `governance/decisions.rs`（sha256 链式，E 期） |
| `session_restorer.py` / `recovery/` | `recovery/`（命名检查点，E 期） |
| `sandbox_executor.py` / `ssrf_guard.py` | `security/`（命令护栏 + SSRF + OS 沙箱，D 期） |
| `kb_index.py` | `kb/`（BM25 RAG，D 期） |
| `tools/` / `codebase_search.py` | `tools/`（46 原生工具，D 期） |
| `context*.py` / `memory_*.py` | `context/` + `memory/`（原生上下文/记忆） |
| `plan_gate.py` / `guard.py` / `guard_gateway.py` | `governance/`（计划门接进 dispatch 前置裁决，E 期） |

## 2. 差异能力审查（本次吸收 / 落地点）

| nexustar-api 模块 | 差异化价值 | 吸收落点（本次） | 测试 |
|---|---|---|---|
| `acp/protocol.py` + `acp/session.py` | ACP(JSON-RPC 2.0)薄壳，回环优先 | **`serve/acp/`**（新模块） | 协议解析/路由全绿 |
| `research.py` + `research_pipeline.py` | 深度研究 pipeline（分解→检索→综合） | **`research/`**（新模块，复用 `kb`） | 端到端 mock LLM 全绿 |
| `vision.py` | 纯文本模型获得看图能力（零依赖元数据解析 + 多模态转发） | **`vision/`**（新模块，复用 `model::llm`） | 元数据解析各格式全绿 |
| `long_running_scheduler.py` | 7×24 长时任务：心跳/检查点/超时检测/自动续跑 | **`automation::long_running`**（扩展现有 automation） | 注册/心跳/续跑/超时全绿 |
| `antihallucination/verifier.py` + `completion_gate.py` | 原生抗幻觉门（Codex/Cursor 缺失） | 暂列待办（后续增量，需 LLM 评测闭环） | — |
| `knowledge_graph.py` | 关系型 KB（BM25 之外） | 暂列待办（KB 增强增量） | — |
| `rag_denoiser.py` / `rag_integrity.py` / `semantic_chunking.py` | KB 质量层 | 暂列待办（KB 增强增量） | — |
| `auto_fix_loop.py` / `auto_review.py` / `qa_remediation/` | 自愈循环 | 暂列待办（接 orchestration 后续） | — |
| `security_scanner_l2.py` / `security_scanner_l3.py` | 分层安全扫描 | 暂列待办（security 增强增量） | — |

## 3. 明确不吸收（与整合决策冲突或已废弃）

- `desktop/`(Tauri WebView) / `nexustar-ui` / `website` / `admin-backend` / `gwwc`：G 期已**可逆冻结**，能力已由原生 `tui/bloom` + `desktop`(egui) + `auth` 承接。
- `browser_bridge.py` / `browser_real_control.py` / `zweb_control.py`：RPA 浏览器控制属于重型原生桥（Windows UIA），列为后续增量，不在本轮。
- `worker_pool.py` / `task_queue.py` / `task_execution_chain.py`：执行层，与现有 `orchestration` 职责重叠，按需并入而非另起炉灶。

## 4. 结论

本次吸收 4 项高杠杆差异化能力（ACP / 研究 pipeline / 视觉桥 / 长时调度），均**离线可单测、零新增重依赖**，
据以完成 E 期剩余的 serve / research / vision / automation 四块。抗幻觉门、知识图谱、自愈循环、分层安全
作为后续增量，已在记忆中登记待办。
