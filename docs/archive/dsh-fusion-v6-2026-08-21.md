# DeepSeek Harness × Marguerite 融合总结 v6（2026-08-21）

> 承接 v2 映射表遗留缺口：goal-round-driver race-fenced 预留 + persona
> 组合渲染。依据：上游 goal 子系统（race-fenced 预留 roundsStarted+1，
> stale 不耗轮）与 persona 包（complete 阻断 + `{{model}}`/`{{cwd}}` 模板）。

## 1. goal-round-driver race-fenced 预留（v2 缺口收尾）

| 改动 | 说明 |
| --- | --- |
| `goal/mod.rs::begin_round_at(expected_revision)` | 乐观并发：持有修订号与当前一致才消耗轮次；**stale 修订号不耗轮**（上游「race-fenced 预留 roundsStarted+1，stale 不耗轮」语义） |
| 预留 = 状态变更 | 成功时 `rounds_started+1` **且 `revision+1`**——两个驱动以同一修订号并发预留时只有一个成功（不双耗） |
| `begin_round` | 委托 `begin_round_at(当前修订号)`，单驱动兼容入口（行为等价 + 修订号推进） |
| `revision()` / `is_stale(rev)` | 诊断接口：无目标时任何持有号都判过期 |

## 2. persona 组合渲染（v2 缺口收尾）

新模块 `core/src/persona/`：

| 上游概念 | Rust 对应 |
| --- | --- |
| 组合式部署段落 | `PersonaSection { id, content, complete }` 有序拼接 |
| `complete` 阻断后续 prompt 追加 | `render_sections`：遇 complete 段即停；段内 `<!--persona-complete-->` 独立标记行经 `strip_complete_marker` 剥除并置位 |
| `{{model}}` / `{{cwd}}` 模板解析 | `resolve_template`：命中变量表替换，未知变量替换为空串，未闭合 `{{` 原样保留 |

接线：

- `agent/store.rs::personality_for`：core/skills/tools 三段组合改为
  `PersonaSection` 渲染；core 段含 complete 标记时技能/工具段被阻断
  （默认无标记 → 输出与旧实现逐字节一致）。
- `gateway/chat.rs::agent_personality_hint(session, model, cwd)`：
  注入时解析 `{{model}}`（当前模型名）与 `{{cwd}}`（会话项目目录）；
  三条注入路径（auto-decompose / Bloom / RealEngine）全部接线。

## 3. 门禁（2026-08-21）

- `cargo test -p marguerite-core --lib`：**2026 passed + 14 ignored，0 failed**（基线 2015 → +11：persona 7 + store complete 1 + goal race 3）。
- `cargo check --workspace`：0 warning / 0 error。
- `cargo clippy --workspace --all-targets`：0 error，本轮新代码 0 告警（修掉 1 处自引入 map_or）。
- `python3 scripts/check-doc-cli-drift.py`：OK；`third_party/validate_integration.py`：PASS（1 条既有 warning）。

## 4. 剩余缺口（后续）

- schedule 会话级 after/at 语义（现为全局 cron）。
- workflow worker-thread 脚本引擎 + ralph 工具化。
- session-projection 消费方迁移（goal/todo 台账、context_status 用量视图）。
- 主题/个性化资产包（webui 侧 wallpaper/skin 素材）。

## 5. 参考

- v3/v4/v5 总结：`docs/dsh-fusion-v{3,4,5}-2026-08-21.md`
- 上游契约：goal-round-driver（`packages/goal/goal-round-driver`）、persona（`packages/persona`，dsh-v0.1.1-rc.1 tag）
