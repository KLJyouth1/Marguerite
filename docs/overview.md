# Marguerite · methods.rs 方法族分治收尾 + 编译修复

## 已完成（本轮核心交付）
修复了 `core/src/gateway/methods.rs` 在「方法族分治」重构中遗留的**文件损坏与编译断裂**，使 `marguerite-core` 恢复正常编译（零错误、零警告）。

### 关键改动（methods.rs + 关联模块）
- **修复文件尾部截断/碎片**：原文件末尾（`dispatch_sync` 闭合后）被错位拼接了 `skills_install`/`mcp_servers_probe`/`games_tests` 残片，导致孤立 `}` 与未闭合分隔符。已恢复 `skills_install`/`mcp_servers_probe` 规范 wrapper，并补回 `dispatch_sync` 缺失的闭合 `}`。
- **模块别名对齐**：`dispatch_rust` 中误用 `config::`/`mode::`/`memory::`，而 imports 是别名 `gateway_config`/`gateway_mode`/`gateway_memory` → 全部改为别名调用。
- **函数路径修正**：
  - 裸 helper 加模块前缀：`vision::modlens_analyze`、`collab::collab_ssh_probe/exec`、`team::team_run/_background`、`sandbox::sandbox_setup_status`、`artifacts::artifacts_list`。
  - `plan::plans_capabilities()` → `cache::plans_capabilities()`（实际定义在 cache.rs）。
  - `gateway_router::router_selflearning_status` → `cache::router_selflearning_status`（实际在 cache.rs）。
  - `config.effective` → 映射到 `g  - config::config_get`（config.rs 无 `config_effective`）。
  - `memory.cross_project.*` / `memory.be - .*` → 改用核心 `crate::memory::{cross_project, behavior}` 子模块（不是网关 memory 模块）。
- **结构体字段修正**（cache.rs / sandbox.rs / config.rs）：
  - `ctx.config.squilla_router.self_learning` → `ctx.config.squilla_router.enabled`。
  - `ctx.config.memory.dream` → `ctx.config.dream`。
  - `Config::data_dir` 关联函数（原误作字段 `.data_dir`）→ `Config::data_dir()`。
  - `DreamConfig` 补 `cadence_ms` 字段（JSON 契约依赖）。
- 清理未使用导入告警（PathBuf / PROTOCOL_VERSION / whitelist / `_ctx`）。

## 验证
- `cargo check -p marguerite-core --offline`：✅ 零错误、零警告。
- `cargo test -p marguerite-core --offline gateway`：**83 passed / 1 failed**。
  - 失败项 `test_complex_task_auto_decomposes_and_registers_long_running`：复杂任务能自动拆解并启动（`plan_revision`/`plan_run` 已发布），但**始终未发布终态 Done/Error** → `plan::runner::run_autonomous` 不退出。这正是用户反馈「任务执行效果垃圾」的根因候选：**子步骤 `orch.dispatch` 每步消耗过多 LLM round-trip，测试 mock 耗尽后挂起；真实任务同样存在无界调用风险**。

## 待办 / 后续（未完成，需确认范围）
1. **任务执行质量（#9）**：在 `plan::runner::run_aut"omous` 子步骤层加「每步 LLM 轮次硬上限」或约束 `Orchestrator.dispatch` 轮转次数，使复杂任务确定性终态。改 `core/src/plan/runner.rs` 与 orchestrator 时需谨慎。
2. **webui 繁杂重构（#10）**：190 组件 + App.vue 2033 行。需先与用户确认精简范围/优先级。
3. **网关问题修复（#11）**：server.rs / WS 事件 / 心跳 / 超时 / 缓存稳定性排查，需用户提供具体症状线索。
4. **上线前全检（#13）**：code review + security audit + QA，落盘 `deliverables/gstack/`。

## 下一步
建议先确认 2–4 的范围与优先级，再进入 Craft 模式执行。当前 methods.rs 修复已具备「可继续推进」状态。
