# Marguerite 五项深度强化总结（v10，2026-08-21）

> 一次性交付：①PrefetchDedup 会话生命周期对齐；②路由模式深度优化；
> ③LLM/模型/智能体/工具联合调用；④SSH 与企业级远程协同；⑤内置编辑器与内置工具完善。

## ① 预填充去重窗口 → 会话生命周期（验收报告 §4 剩余边界）

| 改动 | 说明 |
| --- | --- |
| `PrefetchDedup::warmup_once_for_session(session, messages)` | 键 = `session\|前缀`；会话维度**无 TTL**——同会话稳定前缀全程只预热一次，服务端 KV 块在会话生命周期内持续复用（原固定 300s 窗口的剩余边界关闭） |
| `session_warmed` 集合 + `SESSION_DEDUP_CAP=4096` | 容量上限防无界膨胀（超限整体清空，进程级 LRU 近似） |
| `warmup_once` | 委托空会话 → 保留全局 TTL 窗口语义（向后兼容） |
| `Team::with_session` | 团队预填充去重按会话隔离（同会话恒去重、跨会话互不影响） |
| 单测 | 无视 TTL 恒去重 / 会话隔离 / 空会话回退 / 容量清空后可用 |

## ② 路由模式深度优化（子代理 A 交付 + 主代理修正平局测试）

- `IntentType` 增补 `WebSearch`/`Vision`/`LongContext` 变体 + 对应路由规则；
- `resolve_model` 平局确定性：同 priority 取规则表**最前者**（不再依赖迭代序）；
- `primary_with_thinking`/`primary_with_cap` 抽出共用（思考档/输出上限叠加语义正交）；
- 全档遍历测试：Simple/Medium/Complex × 全部意图必返回合法模型；未知规则回退默认模型。

## ③ 联合调用强化（子代理 B 交付）

- `multi_strategy::resolve_role_model(role, default)`：角色→模型表（architect/reviewer/researcher→pro、qa/coder→flash，归一化大小写/分隔符）；
- 新集成测试 `tests/joint_call.rs`：本地循环 mock LLM（首请求 tool_calls → 终答）+ RecordingTool——证明「模型路由 → 子代理 spawn → 工具执行 → 结果回收」闭环；审计记录「角色模型偏好未被消费」「带工具路径绕过意图路由」为后续接线点。

## ④ SSH 与企业级远程协同（子代理 C 交付）

- `snip_output`（头 4KB+标记+尾 4KB）、`is_idempotent_command`/`idempotency_warning`（非幂等重放审计）、`exec_idempotent`（trait 默认实现，向后兼容）、`exec_batch`（逐条保真、失败不吞）、退出码保真、`HostSpec::parse`（user@host:port）；14 项离线单测。
- 审计留痕：命令超时/ControlMaster 复用/真并发为后续项（未动真实进程路径）。

## ⑤ 内置编辑器与内置工具（子代理 D 交付）

- str_replace_editor 补 `insert`（行号语义）/`undo_edit`（最近编辑快照回退）与 CRLF·空文件·无尾换行·多次替换·空 old_str 等边界硬化 + 单测；路径/空串 panic 修复。

## 门禁（2026-08-21）

- `cargo test -p marguerite-core --lib`：**2083 passed + 14 ignored，0 failed**（基线 2055 → **+28**：路由/联合调用/SSH×14/编辑器/会话预填充）。
- `cargo check --workspace`：0 warning / 0 error。
- `cargo clippy --workspace --all-targets`：0 error，本轮新代码 0 告警。
- `python3 scripts/check-doc-cli-drift.py`：OK；`third_party/validate_integration.py`：PASS（1 条既有 warning）。
