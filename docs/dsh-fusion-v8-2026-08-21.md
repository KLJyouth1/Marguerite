# DeepSeek Harness × Marguerite 融合总结 v8（2026-08-21）

> 一次性收尾 v7 §4 全部剩余项：workflow/ralph 引擎、projection 消费方接线、
> /schedule 注册面执行端、主题/个性化壁纸资产包。
> 依据：上游 `dsh-tool-workflow` + `dsh-workflow-worker-thread`（无阶段屏障 pipeline
> / 阶段屏障 parallel / phase 进度）+ `dsh-tool-ralph`（每轮全新子代理、工作区为
> 共享记忆、有界结构化报告）契约。

## 1. workflow 声明式脚本引擎 + ralph 循环（`core/src/workflow/`）

| 上游概念 | Rust 对应 |
| --- | --- |
| workflow spec（phases + 扇出 items） | `WorkflowSpec`/`WorkflowPhase`（serde JSON，name/phases/title 校验） |
| pipeline 无阶段屏障、parallel 屏障 | `WorkflowEngine`：阶段内 fan-out（依赖 SubAgentManager 并发上限）、阶段间屏障；单条 item 失败 → `None`（不拖垮整轮，对齐「普通 stage throw 只 drop 该 item」） |
| 结构化结果 | `WorkflowReport`/`PhaseResult`（与 spec 一一对应的 item_results） |
| ralph 每轮全新子代理 + 工作区共享记忆 | `RalphRunner`：`next_prompt` 纯函数、`parse_report`（completed/blocker:/继续）、`run_with` 注入式 spawn（同步可测）、memory 文件 `workspace/ralph/<name>.json`（写失败不中断循环）、`RalphOutcome{Completed,Blocked,RoundLimit}` |
| 离线可运行 | 引擎不依赖 LLM；spawn 注入假回复即可完整测试三条结局路径 |

## 2. projection 消费方接线（`gateway/chat.rs`）

- `gateway_projection_store`：`<data_dir>/session-projections.db`（失败退内存）。
- 每轮对话结束后（三条路径）折叠 `turnCount` + `hostAudit` 投影，
  发布 `projection.snapshot` 事件（`{"turns":…,"hostAudit":…}`）——前端可渲染
  会话级投影视图，对齐上游 client 侧 surface-projection 消费。

## 3. /schedule 注册面执行端（`schedule::execute_slash` + REPL）

- `ScheduleStore` 落地执行：`/schedule list|after|at|every|cancel` 解析 →
  注册/列出/取消（at 走 RFC3339 解析，every 下限校验）→ 展示文本。
- `cli/repl.rs::handle_slash` 新增 `/schedule` 分支（REPL 真实执行）；
  网关前端可复用同一 `execute_slash` 入口。

## 4. 主题/个性化壁纸资产包（`webui/public/wallpapers/`）

- 4 款 SVG（水墨 ink-wash / 深空蓝橙 deep-space / 夜樱 sakura / 鲸海 whale-ocean）
  + `manifest.json`（版本化资产清单：id/name/description/file/palette）
  + README（新增约定：SVG + manifest version+1；palette 只放语义令牌）。
- 静态托管于 `/wallpapers/`，皮肤中心按 manifest 渲染面板。

## 5. 门禁（2026-08-21）

- `cargo test -p marguerite-core --lib`：**2050 passed + 14 ignored，0 failed**（基线 2035 → +15：workflow 4 + ralph 8 + 投影快照 1 + schedule 执行面 2）。
- `cargo check --workspace`：0 warning / 0 error；clippy 新代码 0 告警。
- `python3 scripts/check-doc-cli-drift.py`：OK；`third_party/validate_integration.py`：PASS（1 条既有 warning）。
- webui：`npm run typecheck` 通过（check:architecture 10 项守卫 + vue-tsc，exit 0）。

## 6. 至此 v2 映射表全部缺口收尾

锚定族（v3）· plan prompt_section（v3）· 凭证缝/投影 fold/权限溯源（v4）·
凭证/投影接线（v5）· goal race-fence + persona（v6）· schedule（v7）·
workflow/ralph + 投影消费 + schedule 执行面 + 主题资产（v8）。

## 7. 参考

- v3–v7 总结：`docs/dsh-fusion-v{3,4,5,6,7}-2026-08-21.md`
- 上游契约：`packages/workflow/*`、`packages/tool-ralph`（dsh-v0.1.1-rc.1 tag）
