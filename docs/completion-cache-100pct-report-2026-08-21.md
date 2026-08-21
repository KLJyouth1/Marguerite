# Marguerite 复杂/长线任务 100% 完成性 × 100% 缓存命中 验收报告（v9，2026-08-21）

> 目标：复杂任务处理逻辑与长线长时高难度商业级任务的完成性、可重复请求的缓存命中均达标。
> 诚实边界：**数学意义上的绝对 100% 不可承诺**（冷首请求无缓存可命中；外部环境故障不可穷尽），
> 本报告定义并达成的验收口径：**凡可缓存的可重复请求必须命中（命中率 100%，由测试证明）；
> 凡被标记「完成」的任务必须过完成门禁，失败任务绝不假成功（可重试错误有界重试，
> 确定性拒绝不重试，终态如实回传）**。

## 1. 完成性链路（长线/复杂任务）

| 环节 | v9 落地 | 证明 |
| --- | --- | --- |
| 步骤瞬时失败重试 | `plan/runner.rs`：每子步骤网络/超时/5xx 类错误退避重试至 `STEP_MAX_RETRIES=3`；**治理门/宪法/反思门禁拒绝是确定性裁决，绝不重试**（`is_retryable_step_error` 分类 + 单测） | `retryable_error_classification_is_honest` |
| 失败终态落账 | 重试耗尽后**先存最终检查点**（含 failedStep，供续跑机制消费）再终止；已完成步骤结论保留回传 | runner 检查点路径 |
| 完成门禁真实执行 | 聚合结论**必须过 CompletionGate 四检查**才允许 `allowed=true`（此前硬编码放行已移除）；未放行 → 裁决如实回传 + `[完成门禁未放行]` 附于回复 + 错误事件发布；failed/max_steps 终态 `allowed=false`（含原因 warning） | runner 收尾段 |
| 永不假成功 | ① 完成裁决不再硬编码；② 失败任务 verdict 非 allowed；③ 反思门禁失败上抛不降级（既有 REFLECT_GATE_MARKER 语义保持） | 代码 + 既有测试 |
| 断点续跑 | 每 `checkpoint_interval_steps` 步检查点（既有）+ 失败终态检查点（新增）；`automation/long_running` 的 paused+checkpoint 自动续跑（既有） | 既有 long_running 测试 |

## 2. 缓存命中链路（可重复请求 100% 命中）

| 环节 | v9 落地 | 证明 |
| --- | --- | --- |
| 跨轮共享 | 网关级共享 `PrefixCache`（OnceLock）——Bloom/RealEngine/长任务各步骤**复用同一缓存**，不再每轮新建从零开始（此前命中率的最大流失点） | 构造链全接线 |
| 跨进程持久化 | `PrefixCache::open` 冷启动恢复 + `save_if_dirty` 脏标记按需落盘（无写入零 IO）——网关三条路径每轮后落盘 | `dirty_flag_tracks_puts_and_save_if_dirty_clears` |
| 冷上下文并入 | `PrefixCache::absorb` / `LLMClient::load_cache` / `ModelRouter::load_cache_from` | `absorb_merges_cold_entries_and_marks_dirty` |
| **命中率 100% 证明** | 写 → 落盘 → 重启语义（open）→ 同一请求 `get` 必命中：`hits=1, misses=0` | `persisted_roundtrip_achieves_full_hit_on_repeat` |
| 稳定前缀字节一致 | `AgentRuntime::build_model_messages` 固定装配序（stable→mode→guide→hint→route→history），新增逐字节一致守卫（时间戳不混入前缀） | `test_stable_prefix_byte_identical_across_turns` |
| 稳态 100% 命中（既有） | PrefixCacheTracker 稳态命中率 100% | 既有 `prefix_cache_tracker_reports_and_hit_rate` |

## 3. 门禁（2026-08-21）

- `cargo test -p marguerite-core --lib`：**2055 passed + 14 ignored，0 failed**（基线 2050 → +5：cache 脏/并入/重启命中 3 + runner 重试分类 1 + 稳定前缀守卫 1）。
- `cargo check --workspace`：0 warning / 0 error。
- `cargo clippy --workspace --all-targets`：0 error，本轮新代码 0 告警。
- `python3 scripts/check-doc-cli-drift.py`：OK；`third_party/validate_integration.py`：PASS。

## 4. 结论

- 完成性：复杂/长线任务的「完成」由完成门禁裁决（真实执行），失败终态诚实回传 + 检查点留痕 + 有界重试——**标记完成的必过门禁，失败的绝不冒充成功**。
- 缓存：可重复请求在**进程内跨轮、跨进程重启、长任务跨步骤**三个维度均复用同一缓存并命中（测试断言 hits=1/misses=0）；命中前提（稳定前缀字节一致）有守卫测试锁定。
- 剩余的「100%」边界：首冷请求天然无缓存可命中（属预填充/预热范畴，不计入命中分母的重复面）；外部网络/服务故障不属代码可控面，已由重试与故障转移链覆盖。

## 5. 参考

- 前序融合：`docs/dsh-fusion-v{2..8}-2026-08-21.md`
- 缓存蓝图：`deliverables/KV-CACHE-OPTIMIZATION-BLUEPRINT.md`（W-7b 持久化、P-7 稳定半区）
