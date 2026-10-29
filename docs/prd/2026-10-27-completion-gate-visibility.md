# PRD（S 档）：完成门禁拦截必须在正文可见

- 日期：2026-10-27
- 触发：用户原话「虚假完成、长上下文就越答越差」；台账 Z-O2 与 `分析智能体能力缺陷问题.md` 均列此项未关闭
- 范围：`core/src/orchestration/mod.rs` 的 `finish_dispatch`（一处）
- 状态：待实施

## 问题（实测证据，非推测）

`finish_dispatch` 在完成门禁拦截时只发一条思考事件，产出照常返回：

```rust
let verdict = self.evaluate_completion(text, &lead_reply, &[], elapsed);
if !verdict.allowed {
    self.emit_thinking("> 完成门禁提示：...产出可能未经验证，请人工复核。");
}
Ok(OrchestrationOutcome { ... })          // ← lead_reply 原样返回
```

而 `emit_thinking` 发的 payload 是 `{ "kind": "narration" }`，前端渲染为 `ActivityNarration` **默认折叠块**。
另：`grep completion_verdict webui/src` = **0 处**，前端根本不消费这个裁决。

**净效果**：agent 声称「已创建 src/login.rs」但磁盘上没有 → 门禁判 `allowed=false` → 警告进折叠块 → 正文仍是「已完成」→ 用户信了。

**对照**：`plan/runner.rs:983` 对同一条门禁是**硬阻断**（`"The plan finished but did not pass the completion gate."`）。两条路径共用 `evaluate` 实现，但**处置强度不一致**。

## 目标

- G1：门禁拦截时，用户在**必然看到的位置**（最终回复正文）看到明确标注。
- G2：保留原有 narration 通道（活动叙述里仍有记录），不删既有行为。

## 非目标

- **NG1**：不改前端。`webui/src/components/chat/*` 与 `team/*` 正由用户修改，本批不碰。
- **NG2**：不把 Solo 路径改成硬阻断。Solo 常产出解释性回复，硬阻断会误伤；先做到「可见」，是否阻断留给后续批次评估。
- **NG3**：不新增事件类型（会牵动前端契约）。

## 验收口径

| # | 判据 | 验证方式 |
|---|---|---|
| A1 | 门禁 `!allowed` 时 `lead_reply` 末尾含固定标注串 | 单测：构造 blocked verdict → 断言 `contains("完成核验未通过")` |
| A2 | 门禁 `allowed` 时 `lead_reply` **一字不改** | 单测：断言输出与输入全等 |
| A3 | 原有 narration 仍发出 | 保留 `emit_thinking` 调用，不删 |
| A4 | `cargo test -p marguerite-core --lib orchestration::` 全绿 | 测试输出 |

## 影响面

- 消费方：Web UI / TUI / CLI 的最终回复文本（正文多一段 blockquote）。
- 风险：若某测试断言「回复正文长度/结尾格式」，可能失败 —— 需跑 `orchestration::` 与 `gateway::chat` 两组测试确认。

## 回滚

单处 `format!` 包裹，反向 patch 一行即可；无数据迁移、无契约变更。

## 改动契约

| 项 | 内容 |
|---|---|
| 触发 | 用户「虚假完成」痛点 + 台账 Z-O2 |
| 改动面 | `core/src/orchestration/mod.rs`：`finish_dispatch` 一处 + 单测 |
| 影响面 | 最终回复正文（多一段标注）；不影响事件总线契约、不影响前端 |
| 回归清单 | `cargo test -p marguerite-core --lib orchestration::`、`gateway::chat` |
| 回滚方式 | 反向 patch 一行 |
