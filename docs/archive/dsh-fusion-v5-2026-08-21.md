# DeepSeek Harness × Marguerite 融合总结 v5（2026-08-21）

> 承接 v4 §3「接线点」：把凭证缝与投影 fold 契约从「独立缝」接进真实消费方。
> 依据：上游 rc.8 → 0.1.1-rc.1 的 authorization 缝与 session-projection 契约。

## 1. 凭证缝 → LLM 客户端接线（v4 接线点 1）

| 层 | 改动 | 说明 |
| --- | --- | --- |
| `credentials/mod.rs` | `resolve_llm_key(store, provider, fallback)` | 解析优先级：①`CredentialKey("llm", provider)`（LLM 适配器以 provider route key 寻址）→ ②常见 `CredentialRef`（MARGUERITE/DEEPSEEK/OPENAI/ANTHROPIC_API_KEY）→ ③config 直读回退；空密钥记录不覆盖回退（=「拥有者确认走自身环境发现」） |
| `model/router.rs` | `ModelRouter::new_with_credentials` | 台账命中即覆盖 `config.model.api_key` 后走原构造路径（默认客户端/故障转移链/懒创建客户端全部继承） |
| `orchestration/mod.rs` | `new_with_credentials` / `new_with_workspace_credentials` | 原构造器委托 None；模型路由器经凭证缝构造 |
| `tui/workbench RealEngine` | `with_credentials` + execute_turn/run_turn 透传 | 桌面/TUI 引擎同缝 |
| `gateway/chat.rs` | `gateway_credentials()`（OnceLock 进程级缓存）+ Bloom Orchestrator 与 RealEngine 双路径接入 | 网关是只读消费方，进程内只加载一次台账 |

## 2. 投影 fold → usage 钩子接线（v4 接线点 2）

| 层 | 改动 | 说明 |
| --- | --- | --- |
| `projection/mod.rs` | `TokenUsageUnit`（key=`tokenUsage`，state_version=1） | 移植上游 token-meter `tokenUsage` 单元语义：`totals{tokens_in,tokens_out,steps}` + 单槽 `last`；**同一步（`tool@ts`）重复样本替换旧值而非双计**（addReplacing），跨步累加；视图 = totals |
| `harness/dsh_bridge.rs` | `install_usage_hook_with_projection` + `install_dsh_bridges_with_projection` | usage 钩子每记一笔，同步把 `usage.accumulated` 样本折叠进会话投影库（best-effort，失败不阻断记账）；原函数委托 None 保持兼容 |

## 3. 门禁（2026-08-21）

- `cargo test -p marguerite-core --lib`：**2015 passed + 14 ignored，0 failed**（基线 2007 → +8：凭证解析 3 + tokenUsage fold 3 + 钩子→投影端到端 1 + 路由器凭证覆盖 1）。
- `cargo check --workspace`：0 warning / 0 error。
- `cargo clippy --workspace --all-targets`：0 error，本轮新代码 0 告警。
- `python3 scripts/check-doc-cli-drift.py`：OK；`third_party/validate_integration.py`：PASS（1 条既有 warning）。

## 4. 剩余接线点（后续）

- `usage` 模块主台账（usage.db 日聚合）与 `TokenUsageUnit` 会话投影并存，网关 `context_status` 可改为从投影库取会话级用量视图（对齐上游 client 侧 surface-projection 消费方式）。
- `goal`/`todo` 台账迁移到 `ProjectionStore`（fold 单元替换手工累计，`as_of_seq` 对齐事件日志做断点续折叠）。
- 交互式 OAuth/设备码授权流的 UI 往返（workbench/gateway 提示-通知通道）。

## 5. 参考

- v3/v4 总结：`docs/dsh-fusion-v3-2026-08-21.md`、`docs/dsh-fusion-v4-2026-08-21.md`
- 上游契约：`packages/credentials/*`、`packages/session/session-projection`、`packages/llm/token-meter/src/usage-projection.ts`（dsh-v0.1.1-rc.1 tag）
