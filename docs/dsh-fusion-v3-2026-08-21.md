# DeepSeek Harness × Marguerite 融合总结 v3（2026-08-21）

> 依据：上游 `deepseek-ai/deepseek-harness` rc.8 → **0.1.1-rc.1**（HEAD = `dsh-v0.1.1-rc.1`，npm latest 同为 0.1.1-rc.1）差异分析；
> 生态 `@linxin666/dsh-liangshen` 与 `@linxin666/dsh-web-ui-all` 0.2.5/0.2.6 → **0.2.7** 差异分析；
> 本仓完成 v2 §4/§5 遗留接线收尾（锚定规格 → 编排器/运行时全链路落地）。
> 分析报告：`docs/research/dsh-rc8-to-011rc1-delta.md`、`docs/research/ecosystem-025-to-027-delta.md`。

## 1. 版本事实

| 项 | 值 |
| --- | --- |
| 上游最新 tag | `dsh-v0.1.1-rc.1`（= origin/master HEAD，SHA 528c682e） |
| npm `@deepseek-ai/dsh` | latest = next = 0.1.1-rc.1 |
| 本机安装面 | 0.1.0-rc.8（node_modules checkout，只读参照） |
| liangshen / web-ui-all | 本机 0.2.6 → npm latest **0.2.7**（2026-08-21 发布） |
| rc.8 → 0.1.1-rc.1 | 13100 提交、2368 文件；**功能源码改动仅 88 文件**，其余为文档/版本号 |

## 2. 上游 delta 要点（rc.8 → 0.1.1-rc.1，逐项映射）

| # | 上游变化 | 本仓对应 | 状态 |
| --- | --- | --- | --- |
| 1 | **新包 `credentials/authorization`**：通用 `ctx.authorization` 凭证流注册表（OAuth/device-code/api-key）+ `CredentialKey`/`CredentialRecord`（api-key\|grant）+ 本地持久化（PR #2509） | `model/llm.rs` 目前 API Key 直连，无凭证流抽象 | 缺口 → P1（见 §4） |
| 2 | **session-projection API 拆分**：`ProjectionDefinition` 增 `stateSchema`（持久化 fold 状态）与可选 `wire.viewSchema`（客户端视图）；新增 `SessionProjectionStateMap` 表；host-only 单元；`stateOf()`（PR #2730/#2702） | `events/bus.rs` 有界订阅 + `gateway context_status` 台账；`goal/mod.rs` 有事件溯源表 | 部分 → P1：移植「纯 init/apply fold + 版本化状态 + 可选视图」契约 |
| 3 | `IndexInjection` 类型化启动 HTML 行抽象（PR #2725） | `gateway/server.rs` 回环托管 + `webui/` | 缺口（低优先级，浏览器壳细节） |
| 4 | **权限溯源**：`permission/preset` 事件带 `origin: default\|selection\|inferred`；`refreshDefaultForReuse()`（PR #2608） | `guard/gateway.rs` 审批事件无 origin 字段 | 缺口 → P1：事件加 `origin` 枚举 |
| 5 | agent presets 四档**零变化**（preset.yml + agent.cordis.yml 哈希相同） | v2 已融合 | ✅ 无需再动 |
| 6 | bwrap `--unshare-pid` 加固；`withFileLock(waitMs)`；turn-error 不再被 retry 链抑制；cache-hit 显示小数 | `sandbox/`、`automation/`、`model/prefetch.rs` | 参考；turn-error 语义对齐现有「不静默失败」原则 ✅ |
| 7 | 新视觉模型 `deepseek-v4-flash-vision-exp`（text+image） | `is_vision_model` 启发式关键词 | 缺口（小改：关键词加 flash-vision） |
| 8 | **无 anchoring / theming 上游增量** | `mode/anchoring.rs` Bloom 锚定为本仓自持能力 | ✅ 锚定=自有资产，上游不提供 |

## 3. 生态 delta 要点（0.2.5/0.2.6 → 0.2.7）

- **liangshen 锚定引擎零变化**：`preset.yml`/`agent.cordis.yml`/`tool-bootstrap.mjs`/`custom-bash.mjs`/`cordis.patch.yml` 三个版本字节一致 → **本仓 BloomAnchoringSpec 已 0.2.7 同级，无新旋钮可移植**。
- 唯一语义变化：`announceToAgent` 默认 true → false（issue #839，插件自述段默认不再注入）→ 本仓无此自述段，无需移植。
- 新增 live-settings seam（`installSettingsSection`/`settingsNamespace`，devDep `@deepseek-ai/dsh-settings ^0.1.1-rc.1`）→ 参考模式：运行时旋钮（enabled/announceToAgent）可热改并 disposer 清理重注册。
- web-ui-all 聚合层 0.2.5→0.2.7 **字节一致**，仅 17 个子包版本锁步抬升；`dsh-skins` 11 款皮肤字节一致 → 主题侧无增量，v1 已融合的水墨 skin + 蓝橙语义令牌保持现状。

## 4. 本仓本轮落地（v2 §4/§5 接线收尾，全部完成）

### 4.1 锚定规格 → 编排器全链路（v2 §5「接线点已锁定」→ 已接线）

| 接线点（v2 文档锁定） | 落地 |
| --- | --- |
| `orchestration/mod.rs` 三处 `AnchorState::new()` | 新增 `anchoring_spec`/`anchor`/`message_source` 字段 + `set_anchoring_spec`/`set_anchor`/`take_anchor`/`set_message_source` + `bloom_anchoring_spec()`（显式 > 内置 bloom.yaml > 默认）+ `anchor_for_turn`/`anchor_for_delegation`；`dispatch_bloom`/`dispatch_bloom_swarm`/`dispatch_bloom_auto` 三处全部改走会话锚定态 |
| 首请求预算 `bootstrap_max_tokens` | `AgentRuntime` 新增一次性 `Mutex<Option<u32>>` 旋钮；`process_input`/`run_tool_loop` 首个 LLM 请求经 `ModelRouter::chat_for_capped`/`chat_with_tools_capped` + `LLMClient::with_max_tokens` 注入后立即剥离（晋升后绝不残留） |
| 消息源白名单 `message_sources` | `anchor_for_turn` 内 `source_allowed` 门控：非白名单来源（如 auto-round）**不进首块观察、不触发晋升分支**（梁神 issue #578 语义） |
| 延迟注入 `deferred_sources` + `deferred_grace_steps` | `AnchorState` 新增 `turns`/`promoted_at_turn` 计步 + `deferred_allows(source)`（晋升后越过 grace 步数才放行）；`agent-instructions` 侧在 `gateway/chat.rs` Bloom 路径门控人设注入；`skill-catalog` 侧在 `AgentRuntime` 工具回路扣住技能/工具指引段（`set_defer_skill_catalog`） |
| turn/end 释放 + 跨轮持久 | `gateway/chat.rs` 新增 `SessionAnchors` 会话级台账（bounded 1024）：每轮取出注入编排器 → 本轮结束后 `take_anchor` 回收推进后的状态。**晋升一次后不再重锚**（对齐梁神「首轮结束即解除门控会话」） |
| `promote_after_first_response` | 已有（`note_no_tool_round` 参数化），本轮无需改动 |

### 4.2 plan 模式 prompt_section 装配（v2 §4「待注入」→ 已注入）

- `AgentRuntime` 新增 `mode_section` 通道：`build_model_messages` 装配序 = 稳定前缀 → **模式提示段** → 工具指引 → 经验提示 → 路由片段 → 历史（仅追加，红线 #1）。
- `Orchestrator::set_mode_section` 透传；`RealEngine` 新增 `with_mode_section`（显式优先）+ `execute_turn` 按 `runtime_mode()` 自动取 `ModeManager` 规格 `prompt_section`（Plan 模式即注入 plan.yaml 的 DSH 规则段）。

### 4.3 有意分歧（维持 v2 结论）

- 不移植 liangshen「压缩后回退受控阶段（compactionTools）」（红线 #2：工具面单调扩展）。
- 锚定首块门控输入为**用户文本**而非模型首块流（编排器单轮闭环、无流钩子）；已在注释记录为已知简化。
- `bootstrap_max_tokens` 内置默认 `~`（None 不设限）；梁神 1024 作为 manifest 参考参数登记，用户可在 `~/.marguerite/modes/bloom.yaml` 显式开启。

### 4.4 数据层同步

- `plugins/manifests/dsh-presets.plugin.json` → `0.1.1-mgr1`：新增能力 `dsh.anchor.session_persist`/`dsh.anchor.deferred_inject`，hook params 补 `promote_after_first_response`/`deferred_sources`。
- `skills/dsh-market-list.yaml`：dsh-presets 版本与说明更新；dsh-plan-mode / dsh-session-projection 行注明接线与上游待移植状态。

## 5. 门禁（2026-08-21）

- `cargo test -p marguerite-core --lib`：**1985 passed + 14 ignored，0 failed**（基线 1976 → +9 新单测：anchoring deferred/计步 4 + 编排器接线 5）。
- 环境注：宿主 shell 带 `NO_COLOR=1` 会使 2 个既有主题测试走单色回退而失败——门禁运行前 `Remove-Item Env:\NO_COLOR`（非代码缺陷，已复核）。
- `cargo check --workspace`：0 warning / 0 error。
- `cargo clippy --workspace --all-targets`：0 error（style 类既有 warning 数量与 v2 基线一致，本轮未新增）。
- `python3 scripts/check-doc-cli-drift.py`：OK（18 路由全对齐）。
- `python3 third_party/validate_integration.py`：PASS（1 条既有 warning）。

## 6. 下一步（Next Actions，按优先级）

1. **P1 移植 `ctx.authorization` 凭证流缝**：`model/` 新增 `CredentialRegistry`（api-key/grant 两种记录 + 本地持久化），LLM 客户端从注册表解析密钥而非直读 config——对齐 PR #2509 的「凭证流注册表 + 持久化」结构。
2. **P1 移植 projection fold 契约**：`events/bus.rs` 的投影单元改为「纯 `init`/`apply` + 版本化 stateSchema + 可选 wire.viewSchema」，`goal`/`todo`/`plan` 台账迁移到 `SessionProjectionStateMap` 式持久化表。
3. **P1 权限溯源**：`guard/gateway.rs` 审批事件补 `origin: default|selection|inferred` 枚举（对齐 PR #2608）。
4. **小改**：`is_vision_model` 关键词加 `flash-vision`（对齐 PR #2726 新视觉模型目录）。
5. **P2 参考**：`IndexInjection` 启动 HTML 行抽象、live-settings 热改模式（liangshen 0.2.7 settings seam）、bwrap `--unshare-pid`（Windows 侧无对应物，记录即可）。

## 7. 参考

- 上游 delta 逐包证据：`docs/research/dsh-rc8-to-011rc1-delta.md`
- 生态 delta 逐包证据：`docs/research/ecosystem-025-to-027-delta.md`
- v1/v2 融合：`docs/dsh-integration-v1.md`、`docs/dsh-fusion-v2-2026-08-21.md`
- 上游仓库：https://github.com/deepseek-ai/deepseek-harness（tag dsh-v0.1.1-rc.1）
- 梁神预设参照：`C:\Users\Administrator\.dsh\profiles\web\node_modules\@linxin666\dsh-liangshen`（0.2.6，与 0.2.7 锚定引擎字节一致）
