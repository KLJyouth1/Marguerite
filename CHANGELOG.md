# Marguerite CHANGELOG
Version v0.1.4 · Build 2026-08-20 whale-riding

[0.1.5-dev] - 2026-08-21 · DSH 0.1.1-rc.1 Fusion v8 (剩余四项一次性收尾)

Added
  workflow/mod.rs             → 声明式工作流引擎（WorkflowSpec 阶段内 fan-out + 阶段间屏障 +
                                单条失败→None 不拖垮整轮；fanout_plan 纯函数；离线单测）
  workflow/ralph.rs           → Ralph 全新子代理循环（next_prompt/parse_report 纯函数 +
                                run_with 注入式 spawn + 工作区记忆文件 + clamp[1,16]；8 项单测）
  gateway/chat.rs             → 会话投影消费方：每轮折叠 turnCount/hostAudit 进 session-projections.db，
                                projection.snapshot 事件下发（+1 单测）
  schedule/mod.rs             → execute_slash 执行面（/schedule list|after|at|every|cancel 落地台账，
                                +2 单测）；cli/repl.rs /schedule 分支 + /help 更新
  webui/public/wallpapers/    → 主题/个性化资产包：4 款 SVG（水墨/深空蓝橙/夜樱/鲸海）+
                                manifest.json 版本化清单 + README
  docs/dsh-fusion-v8-2026-08-21.md
Changed
  AGENTS.md §2 测试基线 2035 → 2050（+15 新单测）
Tests : 2050 passed / 0 failed / 14 ignored（NO_COLOR 已移除环境下）
WebUI : npm run typecheck 通过（check:architecture 10 项守卫 + vue-tsc）

[0.1.5-dev] - 2026-08-21 · DSH 0.1.1-rc.1 Fusion v7 (schedule 会话提醒)

Added
  schedule/mod.rs             → 上游 dsh-schedule 移植：ReminderTrigger{after/at/every} +
                                MIN_EVERY_INTERVAL_SECONDS=300 + ScheduleError 错误码族 +
                                SQLite 台账（schedule.db）+ due 纯查询 + every 锚点重挂载
                                （不枚举积压）+ cancel；9 项单测
  gateway/chat.rs             → emit_due_schedule_reminders：三条对话路径每轮后触发到期提醒，
                                schedule.due 事件下发；schedule_store 进程级缓存 + 失败退内存
  cli/dslash.rs               → 第 8 条斜杠命令 /schedule（list/after/at/every/cancel + 错误路径）
  plugins/manifests/dsh-schedule.plugin.json → 0.1.1-mgr1（+list/cancel 能力 + slash_command）
  docs/dsh-fusion-v7-2026-08-21.md
Changed
  AGENTS.md §2 测试基线 2026 → 2035（+9 新单测）
Tests : 2035 passed / 0 failed / 14 ignored（NO_COLOR 已移除环境下）

[0.1.5-dev] - 2026-08-21 · DSH 0.1.1-rc.1 Fusion v6 (goal race-fence + persona)

Added
  goal/mod.rs                 → begin_round_at（race-fenced 预留：stale 修订号不耗轮）+ revision()/is_stale()
                                （预留成功 rounds_started+1 且 revision+1，双驱动不双耗）
  persona/mod.rs              → PersonaSection 组合渲染 + complete 阻断后续段 + {{model}}/{{cwd}} 模板解析
  agent/store.rs              → personality_for 改段落化渲染（<!--persona-complete--> 标记剥除并阻断技能/工具段）
  gateway/chat.rs             → agent_personality_hint(session, model, cwd) 三路径接入模板解析
  docs/dsh-fusion-v6-2026-08-21.md
Changed
  AGENTS.md §2 测试基线 2015 → 2026（+11 新单测）
Tests : 2026 passed / 0 failed / 14 ignored（NO_COLOR 已移除环境下）

[0.1.5-dev] - 2026-08-21 · DSH 0.1.1-rc.1 Fusion v5 (凭证缝/投影接线)

Added
  credentials/mod.rs          → resolve_llm_key（provider 路由键 → 常见引用 → config 回退）
  model/router.rs             → ModelRouter::new_with_credentials（台账覆盖 api_key，全客户端继承）
  orchestration/mod.rs        → new_with_credentials / new_with_workspace_credentials（原构造器委托 None）
  tui/workbench RealEngine     → with_credentials + execute_turn/run_turn 透传
  gateway/chat.rs             → gateway_credentials（OnceLock 进程级缓存）+ Bloom/RealEngine 双路径接入
  projection/mod.rs           → TokenUsageUnit（上游 tokenUsage fold：同一步替换 addReplacing、跨步累加、totals 视图）
  harness/dsh_bridge.rs       → install_usage_hook_with_projection / install_dsh_bridges_with_projection
                                （usage 样本折叠进会话 tokenUsage 投影，best-effort 不阻断记账）
  docs/dsh-fusion-v5-2026-08-21.md
Changed
  AGENTS.md §2 测试基线 2007 → 2015（+8 新单测）
Tests : 2015 passed / 0 failed / 14 ignored（NO_COLOR 已移除环境下）

[0.1.5-dev] - 2026-08-21 · DSH 0.1.1-rc.1 Fusion v4 (P1 机制移植)

Added
  credentials/mod.rs          → CredentialKey `<scope>/<id>` + CredentialRef 双名义键空间；
                                 CredentialRecord（api-key | grant tagged）+ CredentialsStore
                                 （data_dir/credentials.json 原子落盘 + ref 解析 + 损坏回退）
  credentials/authorization.rs→ AuthorizationRegistry 授权流注册表（method/notice/prompt/status/
                                 settlement/outcome/entry wire-safe 类型 + begin/settle/last_settlement）
  projection/mod.rs           → ProjectionUnit 纯 init/apply fold + 可选 wire.view（host-only 无视图）+
                                 ProjectionStore（SQLite session_projection_state 版本化状态表、
                                 state_of/view_of/replay）+ TurnCounterUnit/HostAuditUnit 示例单元
  guard/gateway.rs            → ApprovalOrigin（default|selection|inferred）+ PendingApproval.origin +
                                 request_with_origin（request = Default，向后兼容）
  mode/mod.rs                 → ModeManager.spec_origins + spec_origin（内置=Default、目录覆盖=Selection）
  events/envelope.rs          → Envelope::mode_changed_with_origin（permission_origin 随 mode.changed 下发）
  docs/dsh-fusion-v4-2026-08-21.md
Changed
  AGENTS.md §2 测试基线 1985 → 2007（+22 新单测）
  skills/dsh-market-list.yaml → dsh-session-projection 行更新（fold 契约已移植）
Tests : 2007 passed / 0 failed / 14 ignored（NO_COLOR 已移除环境下）

[0.1.5-dev] - 2026-08-21 · DSH 0.1.1-rc.1 Fusion v3 (锚定接线收尾)

Added
  mode/anchoring.rs        → AnchorState 轮次计步 + deferred_allows（deferred_sources × deferred_grace_steps 语义）
  agent/runtime.rs         → bootstrap_max_tokens 一次性请求预算（首请求注入后剥离）+ defer_skill_catalog + mode_section 装配通道
  model/{llm,router}.rs    → with_max_tokens / chat_for_capped / chat_with_tools_capped（锚定 bootstrap 请求级上限）
  orchestration/mod.rs     → anchoring_spec/anchor/message_source 字段 + set_anchoring_spec/set_anchor/take_anchor/
                             set_message_source + anchor_for_turn/anchor_for_delegation；三处 dispatch 改走会话锚定态
  gateway/chat.rs          → SessionAnchors 会话级锚定台账（跨轮持久，晋升一次不重锚）+ 人设 deferred 门控
  tui/workbench RealEngine  → with_mode_section + 按模式自动注入 plan prompt_section（v2 §4 收尾）
  plugins/manifests/dsh-presets.plugin.json → 0.1.1-mgr1（session_persist/deferred_inject 能力 + 新 hook params）
  docs/dsh-fusion-v3-2026-08-21.md + docs/research/{dsh-rc8-to-011rc1,ecosystem-025-to-027}-delta.md
Changed
  AGENTS.md §2 测试基线 1976 → 1985（+9 新单测；NO_COLOR 环境注记）
  skills/dsh-market-list.yaml → dsh-presets/dsh-plan-mode/dsh-session-projection 行更新
Tests : 1985 passed / 0 failed / 14 ignored（NO_COLOR 已移除环境下）

[0.1.4] - 2026-08-20 · DSH 12 Plugin Deep Fusion

Added
  Capabilities SSOT unified registry  → 35 dsh.* capabilities  (12 manifests × 8 manifests SSOT)
  PetWhaleSlot V2 real integration    → WorkbenchState.pet_v2 = Some(PetV2::default())
  12 DSH plugins manifests + skills mappings aligned; 一键启动脚本
  Clippy Advisory-A (7/7) 全部修复
Changed
  core 0.1.3 → 0.1.4   |   tui 0.1.0 → 0.1.4   |   gui 0.1.3 → 0.1.4
  PetV2 6-frame ASCII sprite rendering 真读真写
Fixed
  Deprecated: Buffer::get → buf[(x,y)]
  Deprecated: NaiveDateTime::from_timestamp_opt → DateTime::<Utc>::from_timestamp().naive_utc()
  Unused: ratatui::text::Text import removal
Tests : 43 DSH modules → 43 passed / 0 failed / 0 ignored
Caps  : 35/35 (0 missing, 0 extra)  SSOT
