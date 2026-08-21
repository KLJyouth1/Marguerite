# DeepSeek Harness × Marguerite 融合总结 v4（2026-08-21）

> 承接 v3 §6「下一步」：P1 三项全部落地。依据：上游 rc.8 → 0.1.1-rc.1 的
> 三个机制型增量（authorization 凭证流缝 / session-projection fold 拆分 /
> permission 溯源），逐一对译到 Rust 并过全门禁。

## 1. 落地清单

### ① 凭证流缝（credentials/authorization 移植，上游 PR #2509）

新模块 `core/src/credentials/`：

| 上游概念 | Rust 对应 | 说明 |
| --- | --- | --- |
| `CredentialKey`（`<scope>/<id>` 名义地址） | `credentials::CredentialKey` | scope = 拥有记录的插件注册名（payload 按拥有者格式书写）；与 ref 键空间互不碰撞 |
| `CredentialRef`（POSIX 风格环境变量名） | `credentials::CredentialRef` | 引用而非地址；`set_reference`/`unset_reference`/`resolve_reference` |
| `CredentialRecord`（api-key \| grant tagged） | `credentials::CredentialRecord`（serde tag="kind"） | api-key 可缺省（=「拥有者确认走自身环境发现」，区别于「无记录」）；grant payload 对缝不透明（JSON roundtrip 唯一约束） |
| 本地持久化 | `CredentialsStore`（`<data_dir>/credentials.json` 原子写） | 内存态优先、落盘失败不丢内存态；损坏文件回退空台账并告警 |
| `AuthorizationMethod` / `Notice` / `Prompt` / `Status` / `Settlement` / `Outcome` / `Entry` | `credentials::authorization` 同构纯数据类型 | wire-safe（无运行时引用），secret/text/select 提示、notice 绝不携带密钥 |
| 授权流注册表 | `AuthorizationRegistry` | `register`/`entries`/`begin`（in_flight 置位 → 提交记录 → 清位 → settlement=authorized）/`settle`/`last_settlement`（`failed` 只存在于事件流视角） |

### ② session-projection fold 契约（上游 PR #2730/#2702）

新模块 `core/src/projection/`：

| 上游概念 | Rust 对应 | 说明 |
| --- | --- | --- |
| `ProjectionDefinition{key,stateVersion,stateSchema,init,apply,wire:{viewSchema,view}}` | `ProjectionUnit` trait（`key`/`state_version`/`init`/`apply`/可选 `view`） | `view → None` 即 host-only 单元；fold 是纯函数（确定性、无副作用），重放同序必同态 |
| `SessionProjectionStateMap` 持久化表 | `ProjectionStore`（SQLite `session_projection_state`：`(projection_key,session_id) → (state_version,as_of_seq,state_json)`） | 版本不一致时弃旧重建（stateVersion 迁移语义） |
| `stateOf()` | `ProjectionStore::state_of` / `view_of` / `state_version_of` | 只读查询 |
| 重放/重建 | `ProjectionStore::replay`（纯 fold，不读库不落盘） | 事件日志持有状态、投影是可弃物的本地形态 |
| 示例单元 | `TurnCounterUnit`（视图 `{"turns":n}`）+ `HostAuditUnit`（host-only，无视图） | 覆盖 view/host-only 两种形态 |

### ③ 权限溯源（上游 PR #2608 `permission/preset` origin）

| 上游概念 | Rust 对应 | 说明 |
| --- | --- | --- |
| `origin: default \| selection \| inferred` | `guard::gateway::ApprovalOrigin`（serde lowercase + `as_str`） | — |
| 审批请求携带来源 | `PendingApproval.origin` + `ApprovalGate::request_with_origin`（`request` = Default） | 向后兼容 |
| 预设来源追踪 | `ModeManager.spec_origins` + `spec_origin(mode)`：内置 = Default；`from_dir` 覆盖（含 legacy yolo.yaml）= Selection | 用户显式提供的规格可溯源 |
| 事件载荷 | `Envelope::mode_changed_with_origin`（`permission_origin` 字段随 `mode.changed` 下发） | 不新增 EventType（事件类型表穷举匹配成本高），以载荷字段对齐溯源语义 |

## 2. 门禁（2026-08-21）

- `cargo test -p marguerite-core --lib`：**2007 passed + 14 ignored，0 failed**（基线 1985 → +22：credentials 7 + authorization 5 + projection 5 + guard 3 + mode 2）。
- `cargo check --workspace`：0 warning / 0 error。
- `cargo clippy --workspace --all-targets`：0 error，本轮零新增告警。
- `python3 scripts/check-doc-cli-drift.py`：OK；`third_party/validate_integration.py`：PASS（1 条既有 warning）。

## 3. 接线点（后续）

- `CredentialsStore` 与 `AuthorizationRegistry` 目前是独立缝；接入点 = `model/llm.rs` 客户端构造时经 `resolve_reference` 取密钥（api-key 直读 config 的现有路径保留为回退），交互式 OAuth/设备码流的 UI 往返留给 workbench/gateway 层。
- `ProjectionStore` 的消费方 = `goal`/`todo` 台账迁移与 `gateway context_status`（fold 单元替换手工累计）；`as_of_seq` 对齐会话事件日志序号后可用于断点续折叠。
- `spec_origins` 已随 `mode.switch` 下发 `permission_origin`，前端可按 origin 区分预设来源展示（对齐 web-ui permission 面板）。

## 4. 参考

- v3 总结与上游 delta 证据：`docs/dsh-fusion-v3-2026-08-21.md`、`docs/research/dsh-rc8-to-011rc1-delta.md`
- 上游契约出处：`packages/credentials/{credentials,credentials-local,authorization}`、`packages/session/session-projection`（dsh-v0.1.1-rc.1 tag）
