# DeepSeek Harness × Marguerite 融合总结 v7（2026-08-21）

> 承接 v6 §4 剩余缺口：dsh-schedule 会话级提醒移植 + 真实接线。
> 依据：上游 `packages/schedule/schedule`（dsh-v0.1.1-rc.1 tag）domain 契约
> （after/at/every、`MIN_EVERY_INTERVAL_SECONDS=300`、错误码族、事件名）。

## 1. 核心模块 `core/src/schedule/`

| 上游概念 | Rust 对应 |
| --- | --- |
| `AfterScheduleRecord` / `AtScheduleRecord` / `EveryScheduleRecord` | `ReminderTrigger::{After{seconds}, At{ts_ms}, Every{interval_seconds}}` |
| `MIN_EVERY_INTERVAL_SECONDS = 300` | 同名常量；`every < 300` → `ScheduleError::FrequencyTooHigh` |
| `not_future` / `invalid_rule` / `time_out_of_range` | `ScheduleError` 同构错误码（`at` 必须在未来、`after`>0、消息非空、溢出校验） |
| 事件日志持有状态、定时器是可弃投影 | SQLite 台账（`schedule.db`，`MARGUERITE_SCHEDULE_DB` 测试覆盖）+ `due()` **纯查询**（扫描驱动可任意重建） |
| `EveryOccurrence` 决策（不枚举积压） | `due()` 中 every 一次性推进到最近一个未来锚点（错过不补跑），不置 fired |
| 一次性 fired | after/at 触发后置 `fired=true`，再次 due 不再触发 |

## 2. 接线

| 层 | 改动 |
| --- | --- |
| `gateway/chat.rs` | `emit_due_schedule_reminders`：三条对话路径（auto-decompose/Bloom/RealEngine）每轮结束后触发到期提醒，`schedule.due` Status 事件下发前端；`schedule_store` 进程级缓存，台账打开失败退内存模式（离线可运行） |
| `cli/dslash.rs` | 第 8 条斜杠命令 `/schedule list|after <sec> <msg>|at <iso> <msg>|every <sec> <msg>|cancel <id>`（解析面 + 5 类错误路径测试） |
| `plugins/manifests/dsh-schedule.plugin.json` | 0.1.1-mgr1：+list/cancel 能力、slash_command 声明、接线状态描述 |

## 3. 门禁（2026-08-21）

- `cargo test -p marguerite-core --lib`：**2035 passed + 14 ignored，0 failed**（基线 2026 → +9：schedule 台账 9 项）。
- `cargo check --workspace`：0 warning / 0 error。
- `cargo clippy --workspace --all-targets`：0 error，本轮新代码 0 告警。
- `python3 scripts/check-doc-cli-drift.py`：OK；`third_party/validate_integration.py`：PASS（1 条既有 warning）。

## 4. 剩余缺口（后续）

- workflow worker-thread 脚本引擎 + ralph 工具化。
- projection 消费方迁移（goal/todo 台账、context_status 用量视图）。
- `/schedule` 注册面执行端（当前斜杠命令解析后由前端/未来 Rust 处理器调用 `ScheduleStore::register`；store API 已就绪）。
- 主题/个性化资产包（webui 侧 skin/wallpaper 素材）。

## 5. 参考

- v3–v6 总结：`docs/dsh-fusion-v{3,4,5,6}-2026-08-21.md`
- 上游契约：`packages/schedule/schedule/src/domain.ts`（dsh-v0.1.1-rc.1 tag）
