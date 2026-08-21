# Marguerite goal — 目标循环（自治续轮 + blocked 门控）

> Track 1：把 DeepSeek Harness 的 goal 子系统（持久目标 + 自治续轮 + resume/pause/complete/blocked）
> 原生二次开发进 Marguerite Rust 核心。持久化于 `~/.marguerite/goal.db`（SQLite）。

## 语义

- 单一持久化「完成目标」：同一时刻至多一个 active/paused 目标。
- `activation`（armed/disarmed）为会话级状态，不落库：跨会话/进程恢复后目标自动 disarm，
  需 `resume` 重新 armed 后才继续自治续轮。
- `blocked` 需目标先经过 `MIN_BLOCKED_ROUNDS`（=3）轮续轮（roundsStarted 达标）才成立。

## 命令

```
marguerite goal create <目标描述>           登记唯一进行中的完成目标（armed）
marguerite goal get                        读取当前目标与激活态
marguerite goal update edit <新目标>        编辑目标描述（revision+1）
marguerite goal update pause               暂停
marguerite goal update resume              恢复并重新 armed
marguerite goal update complete            标记完成
marguerite goal update blocked <原因>       报告阻塞（连续 3 轮才置为 blocked）
```

## 示例

```
marguerite goal create 重构部署流水线
marguerite goal get --json
marguerite goal update complete
```

## 状态机

| 迁移 | 前置 | 结果 |
|------|------|------|
| create | 无 active/paused 目标 | active + armed（revision=1） |
| edit | 已有目标 | 目标描述更新（revision+1） |
| pause | active | paused |
| resume | 任意（含会话恢复后 disarm） | active + armed |
| complete | 任意 | complete + disarmed |
| blocked | roundsStarted >= 3 | blocked + disarmed |

## 续轮驱动

`GoalRegistry::begin_round()` 仅在「armed 且 active 且未达 max_goal_rounds」时放行一轮并 rounds_started+1；
其余情况返回 false。CLI 单次进程不自动续轮；驱动由 Agent 运行时/网关按需调用（后续 Track 接线）。
