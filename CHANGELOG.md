# Marguerite CHANGELOG
Version v0.1.4 · Build 2026-08-20 whale-riding

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
