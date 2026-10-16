


# Marguerite M2 Agentic UX 验收报告

- **日期：** 2026-09-04
- **仓库：** `D:\Downloads\Marguerite`
- **分支：** master（最新提交 `126e7908` 之后的本轮工作区改动）
- **范围：** 按改进方案 D4 体验优先级 C1 → C5 → C6 实施，双端（WebUI + GUI）落地
- **实现原则：** core 真实可跑、禁 mock、逐项真实验证、门禁为准

---

## 一、总览

| 项 | 状态 | 说明 |
|----|------|------|
| C1 Intent Preview 动作确认卡（WebUI + GUI + 高风险 100% 单测） | ✅ 完成 | core 确认-拦截闭环 + 双端确认卡 |
| C5 Trust Calibration 三档信任（5 Mode 覆盖 + 落盘持久化 + 设置界面查看/撤销） | ✅ 完成 | core 三档接入 + RPC + 设置面板 |
| C6 持久任务队列（排队/暂停/恢复/取消/调序 + 跨会话落盘 + WebUI 队列管理视图 + core 单测） | ✅ 完成 | 本次会话补齐「调整顺序」缺口 |
| 验收门禁 `scripts/verify.sh --all` | ✅ 全绿 | 12 项通过，0 失败 |
| core 单元测试 | ✅ | 2471 passed / 0 failed / 17 ignored（基线 2425，净增 46） |
| vue-tsc 类型检查 | ✅ | 0 error（含架构/主题/i18n 等 10 项 guard） |
| 文档/代码 CLI 漂移 | ✅ | check-doc-cli-drift.py PASS（0 漂移） |

---

## 二、C1 Intent Preview 动作确认卡

### 实现
- **core 确认-拦截闭环**
  - `core/src/guard/tool_intent.rs`：`ToolIntention` 结构化意图分析（`IntentKind` 枚举 + `analyze` 纯函数 + 高风险 token 表 + fail-closed），判定删除/重置/覆盖/终止/批量/出网等高风险动作并给出副作用描述。单测 14 个，覆盖 delete / reset / terminate / exec / network / overwrite / read / unknown 等场景。
  - `core/src/guard/approval_registry.rs`：进程级全局裁决桥（`Arc<ApprovalGate>` 按 `approval_id` 索引），双端经 `approval_id` 定位 gate 裁决后 unregister。单测 6 个。
  - `core/src/chain/executor.rs` Confirm 分支：`tool_intent::analyze` 结构化意图 + 高风险判定命中则提级 `RiskLevel::High`，挂起审批同时注册进全局 registry，`Approval` 事件 payload 注入 `intent` 结构化字段（kind / high_risk / side_effects）。
  - 集成测试：`m2_c1_high_risk_approval_emits_intent_and_registers`、`m2_c1_high_risk_denied_stops_chain`（C1 测试经静态 Mutex 串行隔离全局 registry）。
- **WebUI 端**：`ApprovalCard.vue` + `useChatApprovals.ts`，`ChatView` 已挂载 `PendingQueue` + `useChatApprovals`，支持批准/拒绝/超时与结构化意图展示。
- **GUI 端**：`gui/src/components/ApprovalCard.tsx` + `gui/src/components/useApprovalBridge.ts`，已挂载于 `gui/src/App.tsx`。`gui/src/__tests__/approval.test.tsx` 共 **11 tests** 全过。

### 验证
- 单测实跑通过；门禁 clippy / typecheck / 单测全绿（见第五节）。

---

## 三、C5 Trust Calibration 三档信任

### 实现
- `core/src/guard/trust.rs`：`TrustLevel` 三档 `Low / Medium / High`，接入 ApprovalGate 影响确认频率；覆盖 5 个 Mode 的 gateway/mode 各分支。单测 12 个。
- **落盘持久化**：SQLite 落盘（默认 `~/.marguerite/trust.db`，`MARGUERITE_TRUST_DB` 环境变量可覆盖），Low/High 落盘、Medium 仅内存（按设计）。
- **RPC + 设置界面**
  - `core/src/gateway/trust_rpc.rs`：`trust.set` / `trust.list` / `trust.revoke`（list 查看、revoke 撤销），本次已将 `TrustLevel::from_str` 更名为 `from_key` 以消除 clippy `should_implement_trait` 告警。
  - WebUI `TrustCalibrationPanel.vue` 挂载于 `SettingsDialog`，支持查看与撤销已授权的记忆条目。

### 验证
- core lib 全量通过（含 trust 12 tests）；撤销能力经 RPC `trust.revoke` 真实验证。

---

## 四、C6 持久任务队列

### 实现
- `core/src/automation/task_queue.rs`：`QueueStatus` 六态（Queued / Running / Paused / Cancelled / Completed / Failed），`enqueue / start / pause / resume / cancel / complete / fail / move_step` 完备；SQLite 跨会话落盘（`MARGUERITE_QUEUE_DB` 可覆盖）；`global()` 单例 + `restore_global()` 恢复。专项单测 **8 个**（含本次新增 `move_step_reorders_queued_priority`）。
- **RPC**：`core/src/gateway/queue_rpc.rs` 提供 `queue.list / enqueue / start / pause / resume / cancel / move`；`core/src/gateway/methods.rs` 完成 `queue.move` 注册。
- **持久任务生命周期**：`core/src/persistence/long_running.rs` LongRunningScheduler（SQLite 持久化、跨会话恢复），单测 12 个。
- **WebUI 队列管理视图**：`TaskQueuePanel.vue` 覆盖查看/暂停/恢复/取消/执行/上移/下移四态操作。
- **本次会话补齐「调整顺序」缺口**
  - `task_queue.move_step(id, up)`：按调度序（优先级、入队时间）与相邻 Queued 任务交换优先级，1 个专项测试。
  - `queue_rpc.queue_move` + `methods.rs` 注册 `queue.move`。
  - `TaskQueuePanel.vue` 新增 ↑ / ↓ 按钮（queued 态）与 `moveStep` 调用。

### 验证
- 跨会话恢复由 `persists_and_restores_across_sessions`、`persists_running_and_completed` 等测试真实验证；调序由 `move_step_reorders_queued_priority` 验证（含最优先不可再上移、非 Queued 不可调序、不存在 id 返回 false）。

---

## 五、门禁执行结果（`scripts/verify.sh --all` 实跑）

闸门结果：**全绿（12 项通过）**。

| 检查项 | 结果 | 关键证据 |
|--------|------|---------|
| core 单元测试 (`cargo test -p marguerite-core --lib`) | PASS | `test result: ok. 2471 passed; 0 failed; 17 ignored` |
| core CLI 构建 | PASS | `cargo build -p marguerite-core --bin marguerite-core` |
| clippy L3 零警告 (`--workspace --all-targets`) | PASS | 0 warning / 0 error |
| 文档/代码 CLI 漂移 (`check-doc-cli-drift.py`) | PASS | 0 漂移 |
| 设计令牌三端同步 (`design/tokens.json`) | PASS | — |
| webui 类型+架构门禁 (`npm run typecheck`) | PASS | 10 项 guard（架构/安全/色板/动效/圆角/主题契约/作用域/cross-surface/i18n/channel 目录）+ vue-tsc，0 error |
| webui 单测 (`npm run test:unit`) | PASS | 305 个测试文件全过 |
| gui 类型门禁 (`npm run typecheck`) | PASS | tsc --noEmit 0 error |
| gui ESLint 门禁 (`npm run lint`) | PASS | 0 errors（9 warnings 均为既有风格告警） |
| gui 单测 (`npm run test:unit`) | PASS | 11 个测试文件 / 53 tests 全过（含 approval.test.tsx 11 tests） |
| gui 构建 (`npm run build`) | PASS | vite build 成功 |
| 全量构建门禁 (`cargo build --workspace`) | PASS | marguerite-core / gui / tui 编译成功 |

### 环境性说明（如实注明）
- `doctor::disk_free_over_threshold_ok` 为 C:\ 磁盘类环境性校验项；本次 verify 中 core lib 单测为 **0 failed**（该环境项未触发/按 ignored 处理），即使触发也属已知非回归项，非代码缺陷。

---

## 六、过程中修复的伴随问题
- **clippy L3 一击通过**：修复 `executor.rs`（C1 集成测试 `await_holding_lock`、`len_zero`）、`tool_intent.rs`（`manual_contains` / `collapsible_if` / `unnecessary_lazy_evaluations`）、`task_queue.rs` / `trust.rs` / `trust_rpc.rs` 的 `from_str` → `from_key` 改名（消除 `should_implement_trait`）。
- **WebUI TS2367**：`TaskQueuePanel.vue` 冗余分支比较（`status !== 'cancelled'` 与被收窄的 queued/paused 无重叠）已消除，`vue-tsc` 0 error。
- **m2_c1 测试锁毒化**：`move_step` 早期断言失败毒化共享 `QUEUE_TESTS_LOCK`，连锁影响 4 个队列测试；修复 `move_step` 按调度序（优先级、入队时间）排序与 `next_pending` 口径一致后全部恢复。

---

## 七、结论
C1 / C5 / C6 三项已全部真实施、真验证并达标：core 单测 2471 passed / 0 failed，clippy L3 零警告，vue-tsc 0 error，doc-drift 0，`verify.sh --all` 12 项全绿。无遗留阻碍。
*（内容由AI生成，仅供参考）*
*（内容由AI生成，仅供参考）*
