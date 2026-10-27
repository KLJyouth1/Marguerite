# PHASE_B 收尾 · PRD(M)

> 日期：2026-10-21 · 版本基线 0.2.15
> 触发：用户要求「完成【百度搭子】执行 PHASE_B_PLAN：智能体团队与数据打通」。
> 本 PRD 只覆盖**收尾缺口**，不重写已落地部分。

---

## 1. 问题（先钉现状，再谈新增）

PHASE_B_PLAN 的 P1–P4 已完成大半，但**没有人做过"哪些真的能跑"的对账**。实测（2026-10-21 14:20）：

| 阶段 | 代码在 | 测试 | 结论 |
|---|---|---|---|
| P1 三断点（部署激活/候选刷新/路由发布） | ✅ | `local_deploy_activates_and_reconciles_candidates` 等 **50 passed** | 🟢 已闭环 |
| P1·P2 前端订阅 + `models.changed` | ✅ LocalModelsView:528 / candidateModels store | 前端测试在 | 🟢 |
| P2·A CLI/TUI | ✅ `marguerite model list/deploy/activate` | 编译通过 | 🟢 |
| P2·B flyhash / tiered | ✅ 952 行 | **6 passed** | 🟡 **有模块无接线**——引擎从不调用它 |
| P2·C capability_scan | ✅ 399 行 | **7 passed** | 🟢 |
| P2·D permission_matrix | ✅ 353 行 | **4 passed** + orchestrate 引用 | 🟢 |
| P2·E 对话编排 | ✅ `orchestrate.run` + 2 个 Vue 组件 | **5 passed** | 🟢 |
| P3 全域打通 | 🟡 `models.changed` 已广播 | 未逐行验 | 🟡 **11 行矩阵未逐行对账** |
| P4 工具扩展 | ❌ `tools/structured_output.rs` / `sandbox_exec.rs` **不存在** | — | 🔴 **完全未做** |

**三个真缺口**：
1. **P2·B 是"死库"**：`flyhash` + `tiered` 实现了稀疏投影/WTA/渐进检索，测试全绿，但**全仓无一处引擎调用**（`grep tiered core/src/agent core/src/engine` 为空）。这正是本项目反复出现的「有而不用」缺陷（lesson 9）。
2. **P3 只有 1/N 打通**：`models.changed` 发了、`LocalModelsView` 订阅了，但矩阵里另外 10 个消费方是否真的订阅，无人验证。
3. **P4 完全没有**：模型的两个结构性短板（JSON 不可靠、不能算）至今无工具可解。

---

## 2. 目标与非目标

### 目标（可测）
- **G1**：`tiered` 记忆真的接进上下文注入——给定会话，`progressive_retrieve` 的 top-k 能进入同一轮的模型请求体，且**无关项不注入**（断言：注入片段数 ≤ k，且不含异构标记）。
- **G2**：P3 矩阵 11 行逐行给出「读/写/刷新」三要素的**实测证据**（前端文件:行 或 测试名），断一处即写「未打通 + 原因」。
- **G3**：P4·T1 结构化输出工具可用——畸形 JSON 返回**可修复的结构化错误**（含 `expected`/`got`/`hint`），不 panic、不返回自由文本。
- **G4**：P4·T2 代码执行沙箱可用——**默认 dry-run 不执行**、白名单外拒绝、执行走 `SilentCommand`（不弹黑窗）。
- **G5**：`cargo test` + `npm run typecheck` + `npm run build` 全绿；出安装包并给指纹。

### 非目标（本轮明确不做）
- **NG1**：不改 P1/P2·A/C/D/E 的既有实现（已有测试保护，改动只增不删）。
- **NG2**：不做 TUI 的模型面板 UI（P2·A 只验 CLI；TUI 面板留批次 3）。
- **NG3**：不接真实 embedding 后端（P2·B 精排仍用现有余弦接口，后端选择留给配置）。
- **NG4**：不做 P4·T3 联网检索（价值最低、外联风险最高）。
- **NG5**：不动 `gui/src` React（产品不可达，见 P2·E 原文）。

---

## 3. 方案（每个缺口一条）

### 3.1 P2·B 接线（最小侵入）
- 新增 `core/src/memory/inject.rs`：`pub fn build_memory_context(session, query, budget) -> Option<String>`，内部 `TieredMemory::progressive_retrieve` → 拼成 `## 相关记忆` 块，**带条数与预算上限**。
- 接入点：`core/src/agent/runtime.rs` 组 prompt 处（与既有 volatile 块同级），**开关**：`config.memory.tiered_enabled`（默认 false，`#[serde(default)]`）。
- 为什么默认 false：接引擎是行为变更，默认开启会让所有既有对话的 prompt 变化 → 违反"零回归"。用户可在设置页开。

### 3.2 P3 逐行对账
- 产出 `deliverables/PHASE_B_P3_MATRIX.md`：11 行 × 4 列（读源 / 写 RPC / 刷新方式 / **证据**）。
- 证据形式：`webui/src/...:行号` 或 `test 名`。找不到即写 `✗ 未打通`，**不许写"应该"**。

### 3.3 P4·T1 结构化输出
- `core/src/tools/structured_output.rs`：`validate_json_against_schema(value, schema)` 纯函数 + `RepairHint { path, expected, got, hint }`。
- 注册进工具目录（`tools/mod.rs`）+ 单测覆盖：类型错、缺字段、多余字段、嵌套错、数组元素错。

### 3.4 P4·T2 执行沙箱
- `core/src/tools/sandbox_exec.rs`：`exec_python(code, opts)`，`opts.dry_run` 默认 true。
- 白名单：只允许 `python -c` + 无网络 + 只读挂载；执行走 `proc::SilentCommand`。
- 单测：dry-run 不产生副作用（写文件/网络均不发生）、白名单外（如 `os.system`）拒绝。

---

## 4. 影响面 / 回归清单 / 回滚

| 项 | 内容 |
|---|---|
| **影响面** | `agent/runtime.rs`（prompt 组装，受开关保护）；`tools/mod.rs`（新增 2 个工具，工具数 +2）；`persistence/config.rs`（新增 1 字段，`#[serde(default)]`）；webui 不涉及 |
| **回归清单** | `cargo test -p marguerite-core --lib`（全量）；`cargo test -- local_models flyhash tiered permission_matrix capability_scan orchestrate`（50+ 基线不许掉）；`npm run typecheck`；`npm run build` |
| **回滚** | ① 记忆接线：`memory.tiered_enabled=false`（默认即 false，等于未启用）② P4 工具：从 `tools/mod.rs` 摘除注册两行 ③ 全部：`git revert <本批 commit>` |

---

## 5. 验收口径（可观察 / 可命令）

| # | 验收项 | 判据 |
|---|---|---|
| V1 | 记忆接线生效 | 开开关 + 注入 3 条同族记忆 → prompt 含 `## 相关记忆`；关开关 → 不含。单测断言两个方向 |
| V2 | 记忆不污染 | 注入 10 条无关项 → 检索后注入 0 条（`tiered` 已有 `unrelated_items_do_not_pollute`，V2 验的是**注入层**同样成立） |
| V3 | P3 矩阵 | 11 行全部有 `file:line` 或 `test 名`；`✗` 行必须带原因 |
| V4 | T1 畸形 JSON | `{"a":"str"}` 对 schema `{a:number}` → 返回 `RepairHint{path:"a", expected:"number", got:"string"}`，不 panic |
| V5 | T1 合法 JSON | 合法输入 → `ok:true`，无 hint |
| V6 | T2 dry-run | dry-run 下执行 `open('x','w')` 后 `x` **不存在**；返回 `would_execute:true` |
| V7 | T2 白名单 | `import os; os.system('calc')` → 拒绝，错误码 `SANDBOX_DENIED` |
| V8 | 门禁全绿 | `cargo test` 0 failed；`npm run typecheck` exit 0；`npm run build` 产物 303 files |
| V9 | 出包对账 | 两处部署副本 `index.html` md5 一致；安装包 sha256 前缀记录 |

---

## 6. 分批（诚实区间）

| 批次 | 内容 | 预计 |
|---|---|---|
| **1（本轮）** | P4·T1 + P4·T2 + 单测（唯一完全未做的阶段） | 主战场 |
| **2（本轮）** | P2·B 接引擎（开关默认 false）+ V1/V2 测试 | 中 |
| **3（本轮）** | P3 矩阵逐行对账文档（含 `✗` 诚实标注） | 中 |
| **4（本轮）** | 全量门禁 + 前端 build + `cargo tauri build` 出包 | 长（12min 构建） |
| 5（下轮） | TUI 模型面板 UI；P2·B 精排接真实 embedding | — |

---

## 7. 借镜签名（用了谁的视角，得出什么结论）

- **陈序安（后端契约）**：新增 2 个工具必须走既有 `tools` registry + schema，不另起注册机制；`exec_python` 的 `dry_run` 必须是**默认值**而非调用方可选项 → 已落到 §3.4。
- **卫无漏（安全）**：执行沙箱的最大风险不是"执行"，是**默认就执行**。结论：`dry_run` 默认 true + 白名单 + `SilentCommand` 三重，并给 `SANDBOX_DENIED` 独立错误码（可被前端区分）。
- **岳训之（ML 评测）**：记忆接线的价值必须可测——不能只看"注入了"，要验"无关项不注入"（否则等于污染上下文）。→ 已拆成 V1/V2 两条对立验收。
- **郝运平（交付）**：`memory.tiered_enabled` 默认 false 是**回滚开关**，不是功能开关；默认开启会让所有既有会话 prompt 变化，属不可接受的静默回归。
- **甘无虞（审计）**：P3 矩阵的 `✗` 行必须写原因，不许留空——空行会被后续读者当成"已打通"。

---

## 8. 执行回执（2026-10-21 14:45 更新）

| 验收项 | 状态 | 证据 |
|---|---|---|
| V1 记忆接线生效（开） | 🟢 | `gateway::chat::tests::tiered_memory_on_injects_recalled_block` passed |
| V1 记忆接线零回归（关） | 🟢 | `..._off_keeps_prefix_byte_identical_to_baseline` passed（断言不含 `## 相关分层记忆`） |
| V2 记忆不污染 | 🟢 | `memory::inject` 7 passed，含 `empty_store_returns_none_not_empty_vec` / 预算截断 |
| V3 P3 矩阵 | 🟢 | `deliverables/PHASE_B_P3_MATRIX.md`：9🟢 / 1🟡 / 1🔴（🔴 本轮已补 = 主模型指示） |
| V4 T1 畸形 JSON | 🟢 | `structured_output` 13 passed（含 `wrong_type_reports_path_expected_and_got`） |
| V5 T1 合法 JSON | 🟢 | `valid_payload_yields_no_hints` passed |
| V6 T2 dry-run | 🟢 | `dry_run_does_not_execute_and_reports_risk` passed（带行号） |
| V7 T2 白名单 | 🟢 | `high_risk_code_is_denied_by_default` passed（`SANDBOX_DENIED`） |
| V8 门禁全绿 | 🟢 | 后端 PHASE_B 相关 **84 passed / 0 failed**；前端 `npm run build` **EXIT=0**，303 files，全部守卫通过 |
| V9 出包对账 | ⏳ | `cargo tauri build` 运行中（见下方补记） |

### 计划外但必要的修正

1. **`code_exec` 加固而非新建 `sandbox_exec`**：既有 `code_exec` 已有临时目录隔离 + 无 shell + 超时 + 不弹窗，
   再建一个同义工具属重复造轮子。改为给它加 `dry_run` / `allow_unsafe` / 静态风险扫描（HIGH 默认 fail-closed）。
2. **测试锁必须单一**：`memory::inject` 与 `gateway::chat` 各自 `OnceLock<Mutex>` → 两把锁保护一个单例，
   混跑必然互相清库（实测 83 passed / 1 failed）。已统一到 `inject::test_guard()`。
3. **开关读取必须在 `spawn_blocking` 之前**：`Config::set_config_dir_override` 是 thread_local，
   在阻塞线程里读会读不到覆盖 → 读到真实用户配置（开关恒 false，功能"永不生效"）。
   这是**生产正确性**问题，不只是测试问题。
