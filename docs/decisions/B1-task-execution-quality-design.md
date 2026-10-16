# B1 设计文档 — Rust core 任务执行质量（plan::runner 子步骤 LLM 轮次）

> 状态：**草案，待项目总监 + 用户确认后执行**
> 日期：2026-08-31
> 技术线：Rust core（`core/src/plan/runner.rs` → `core/src/orchestration/mod.rs` → `core/src/agent/runtime.rs`）
> 用户指令：**「先做方案再执行」**

---

## 1. 背景与动机

`overview.md` 将 B1「任务执行质量」标注为人厌恶的真实痛点，关联根因描述为：
> 「子步骤每步消耗过多 LLM round-trip，mock 耗尽后挂起」。

即担忧 `plan::runner::run_autonomous` 的 **dispatch 次数无硬上限**，可能导致：
- LLM 调用无限轮次（烧 token／回不收敛）
- mock 数据耗尽后子步骤静默挂起，拖垮整个计划闭环

本方案采取 **先勘察后动手** 的纪律：**不基于假设加护栏**，而是先逐层读透现有防线，判断哪些缺口是真实存在的、哪些已被既有机制覆盖，避免「为加硬上限而加硬上限」的反模式。

---

## 2. 现有防线的完整勘察（逐层核实）

### 2.1 plan/runner.rs `run_autonomous`（第 156 行起）

| 防线 | 位置 | 现状 |
|------|------|------|
| `max_steps` 步骤数上限 | L245-248 | `if completed_steps > max_steps` → `stopped_reason = "max_steps"` + `plans_cancel` + `break`。**默认 12**（`persistence/config.rs:210` `default_plan_max_steps()`），可经 `config.plan.max_steps` 覆盖 |
| `STEP_TOTAL_TIMEOUT` 单步总墙钟预算 | L300-302 | 默认 **120s**（`STEP_TOTAL_TIMEOUT_DEFAULT=120`，L57），可经 `config.plan.step_timeout_secs` 覆盖。用**单调递减的剩余时间**包装单次 `dispatch` |
| `STEP_MAX_RETRIES` 瞬时失败重试 | L303-343 | 常量 **3**（L34），只对 `is_retryable_step_error`（网络／超时／5xx）退避重试；**治理门/宪法/反思门禁的确定性裁决绝不重试**（L291-292 注释明示） |
| `tokio::time::timeout(remaining, orch.dispatch(...))` | L311 | **每步墙钟超时硬中断**无论 dispatch 内部是否挂起 |
| `set_force_solo(true)` | L265 | 子步骤**强制 Solo**，不升级为多 Agent 策略（L841-844 注释：「原子任务不应被升级为多 Agent 策略（浪费额度且易失控）」） |
| `completed_steps` 检查点 | L355-372 | 每 `ckpt_every`（默认 1）步落检查点 + 心跳，支持续跑 |

**结论**：`run_autonomous` 的「dispatch 次数」已经通过 `max_steps`（步骤数硬上限）+ `STEP_TOTAL_TIMEOUT`（每步墙钟硬中断）双重收口。**dispatch 不再是无界的**。

### 2.2 orchestration/mod.rs `dispatch`（第 808 行起）

- `force_solo=true` 时走 `AgentMode::Solo`（L847-867）→ `self.lead.process_input(text)`（L865）。
- **Solo 路径是单次 LLM 调用**（见 2.3），不存在内部 round-trip 循环。

### 2.3 agent/runtime.rs `process_input`（第 393 行起）

- L442 构造模型消息 → L448-456 单次 `model_router.chat_for(...)` → L461 回写历史。
- **single-shot**：无 `for`/`while` 循环，天然不产生多轮 LLM round-trip。

### 2.4 agent/runtime.rs `run_tool_loop`（第 536 行起）

- L546 `let max_rounds: u32 = 20;` → L577 `for _ in 0..max_rounds`。
- **已硬上限 20 轮**，超限即 `Err`（L631-634）。

### 2.5 汇总：真实防御矩阵

| 路径 | 轮次上限 | 状态 |
|------|----------|------|
| Solo `process_input`（plan 子步骤实际路径）| **1 次**（single-shot）| 已天然有界 |
| `run_tool_loop` | **20 次**（硬编码）| 已硬上限 |
| `run_tool_loop_stream`（真流式）| `max_rounds` 参数，`stream_loop.rs:239` 有界 | 已硬上限 |
| `run_bloom_auto` | `auto_max_iterations`（`current_bloom_spec`）| 已硬上限 |
| 外层 `run_autonomous` dispatch 循环 | `max_steps`（默认 12）| 已硬上限 |
| 外层每步 | `STEP_TOTAL_TIMEOUT`（默认 120s）| 已墙钟硬中断 |

**核心发现**：`overview.md` 所述「子步骤每步消耗过多 LLM round-trip」在 **当前代码中已经不存在**——plan 子步骤走 Solo（单次调用），且外层 dispatch 已由 `max_steps` + `STEP_TOTAL_TIMEOUT` 收口。**无需再加一道冗余硬上限**（那会违反「不过度设计/不为覆盖率而覆盖率」的评审护栏）。

---

## 3. 识别出的真实缺口（非冗余、非假设）

既然「无界挂起」已被既有机制覆盖，B1 真正值得投入的改进是**任务执行质量的可见性 + 预算可调性**，而非再加一道上限：

### 缺口 A：单步 LLM 成本/轮次不可观测
- 现状：plan 子步骤每步调 1 次 LLM，但 **没有 telemetry** 报告「本计划共 N 次 LLM 调用、总耗时、每步耗时」。
- 影响：任务执行后无从判断「效果垃圾」到底是模型质量、越权、还是步骤拆解不当。与「验证不采信自述、要用机械证据」的 R9 纪律相悖。

### 缺口 B：`max_steps=12` / `step_timeout_secs=120` 为隐式默认，不可按任务类型调
- 现状：两个有界参数硬编码默认，仅在 `config.plan` 全局覆盖。
- 影响：无法对「一个复杂任务」与「一个简单拆分」差异化地分配轮次预算。

### 缺口 C：子步骤失败缺少「为何失败」的归因钩子（可观测性边角）
- 现状：失败时仅 `stopped_reason="failed"` + 错误消息（L388-415）。
- 影响：区分「治理门拒绝 / 墙钟超时 / 重试耗尽 / LLM 报错」四类终态需要人工 grep 日志。

---

## 4. 方案对比

| 方案 | 描述 | 优点 | 缺点 | 建议 |
|------|------|------|------|------|
| **C1（推荐）**：成本/轮次遥测 | 在 `run_autonomous` 聚合 `OrchestrationOutcome` 时，记录每步 LLM call 数/耗时/模型，汇入 `OrchestrationOutcome` + 事件总线回流前端 | 直接回应「执行质量不可见」；机械证据，符合 R9；零新增上限，不改行为 | 需在 `run_single`/`process_input` 冒泡耗时；侵入少量调用链 | ✅ 采纳 |
| **C2**：预算按任务类型可调 | `config.plan` 增加 `max_steps` 的按 intent/complexity 默认值映射 | 灵活 | 改动 over-engineering，当前无需 | ❌ 暂缓（OPEN-DECISIONS） |
| **C3**：再加一道 dispatch 硬上限 | 给 `dispatch` 循环再加 `MAX_DISPATCH` 常量 | 表面保险 | **冗余**——已被 `max_steps`+`STEP_TOTAL_TIMEOUT` 双重覆盖；违反「不为覆盖率加码」护栏 | ❌ 不采纳 |

---

## 5. 推荐执行范围（C1 最小闭环）

**目标**：为「任务执行质量」注入机械证据，让每次 plan 运行结束后可回答「这轮烧了多少 LLM、每步花多久」。

### 改动点
1. **`core/src/plan/runner.rs`**
   - 在子步骤 `dispatch` 调用处（L311 起）包裹耗时计时（`Instant::now()` 起始，`outcome` 返回时记录 `elapsed_ms`）。
   - 在 `OrchestrationOutcome`（或 `step` 局部）累计 `llm_call_count` / `total_elapsed_ms` / 每步明细。
   - 收尾（L420 起）把聚合指标写入事件总线（`EventType` 沿用，或新增字段），并附到 `lead_reply` 末尾的可选摘要。

2. **`core/src/orchestration/mod.rs`**
   - 在 `run_single`（Solo 路径）返回时，把内部 LLM 调用数/耗时冒泡到 `OrchestrationOutcome`（新增可选字段，缺省不破坏既有契约）。

3. **测试（`core/src/plan/runner.rs` 或 `tests/`）**
   - 新增：断言聚合指标在成功/失败两条路径均被记录（`llm_call_count ≥ 1`、`total_elapsed_ms ≥ 0`）。
   - 保持既有 `run_autonomous_rejects_simple_task`（L960）等零回归。

### 范围门禁
- **不新增** `MAX_DISPATCH` 类常量（避免冗余）。
- **不修改** `process_input`/`run_tool_loop` 的循环边界（已各自有界）。
- **不新增** 配置项（先出证据，再决定要不要可调）。

---

## 6. 验收标准（EARS）

| 编号 | 验收标准 |
|------|----------|
| AC-B1-01 | While 一个 plan 运行结束，系统**必须**在聚合结果中记录 `llm_call_count` ≥ 1 |
| AC-B1-02 | While 一个 plan 运行结束，系统**必须**记录每步 `elapsed_ms` 并汇总为 `total_elapsed_ms` |
| AC-B1-03 | If 子步骤失败，系统**必须**仍能回传已记录的指标（不因失败丢弃可观测数据） |
| AC-B1-04 | If 既有 `run_autonomous_rejects_simple_task` 等测试运行，系统**必须**零回归 |

---

## 7. 暂缓项（登记 OPEN-DECISIONS）

- **预算按任务类型可调（C2）**：待 C1 遥测跑出真实分布后，由数据驱动决定是否需分档。先出证据再定 —— `design-decision-to-evaluate`。

---

## 8. 决策摘要（以供用户拍板）

- **「无界挂起」已被既有 `max_steps` + `STEP_TOTAL_TIMEOUT` + Soloada` 覆盖**，不采纳再加硬上限（避免冗余/过度设计）。
- **推荐 C1**：为 task execution quality 注入遥测（每步 LLM 调用数 + 耗时 + 聚合），用机械证据替代「凭感觉说质量垃圾」。
- 范围最小、不改行为、不新增配置；改 2 个文件 + 补测试。
