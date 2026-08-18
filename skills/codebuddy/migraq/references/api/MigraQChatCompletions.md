# MigraQChatCompletions — 迁移专家全局对话

腾讯云迁移服务专家对话接口（SSE 流式输出），支持云资源扫描、迁移方案规划、目标云选型推荐、TCO 分析、迁移工具选择等全流程迁移问答。

## 参数

| 参数 | 必选 | 类型 | 描述 |
|------|------|------|------|
| Input | 是 | String | 用户问题，如 `阿里云50台ECS如何迁移到腾讯云` |
| Stream | 是 | Boolean | 固定值 `true`（SSE 流式输出） |
| SessionKey | 是 | String | 会话 ID（UUID v4）。首次调用由脚本自动生成；多轮对话时传入上次返回的 `session_id`，服务端据此维护独立的对话上下文 |

## 鉴权

使用腾讯云 AK/SK 鉴权：`Authorization: Bearer <TENCENTCLOUD_SECRET_KEY>`

## 调用示例

```bash
python3 {baseDir}/scripts/migrateq_sse_api.py '阿里云50台ECS如何迁移？'
python3 {baseDir}/scripts/migrateq_sse_api.py '详细说说 go2tencentcloud 步骤' '550e8400-e29b-41d4-a716-446655440000'
```

## 返回格式（脚本输出）

脚本自动解析 SSE 流并汇总为统一 JSON：

```json
{
  "success": true,
  "action": "MigraQChatCompletions",
  "data": {
    "content": "## 迁移方案概述\n\n对于阿里云50台ECS迁移到腾讯云...",
    "is_final": true,
    "session_id": "550e8400-e29b-41d4-a716-446655440000",
    "usage": {
      "prompt_tokens": 13080,
      "completion_tokens": 512,
      "total_tokens": 13592
    }
  },
  "requestId": "resp_84ced3ce-1234-5678-abcd-ef0123456789"
}
```

## 返回字段说明

| 字段 | 类型 | 说明 |
|------|------|------|
| `content` | String | Markdown 格式回答，直接展示给用户 |
| `is_final` | Boolean | 是否为最终结果（固定 true） |
| `session_id` | String | 调用方传入的 SessionID，原样返回 |
| `usage.prompt_tokens` | Integer | 输入 Token 数 |
| `usage.completion_tokens` | Integer | 输出 Token 数 |
| `usage.total_tokens` | Integer | 总 Token 数 |
| `requestId` | String | Gateway 请求 ID，用于问题排查 |

## 原始 SSE 流格式

```
event: run.started
data: {"type":"run.started","session_id":"550e8400-e29b-41d4-a716-446655440000"}

event: run.progress
data: {"type":"run.progress","stage":"preparing","summary":"Preparing request"}

event: message.delta
data: {"type":"message.delta","delta":"## 迁移"}

event: message.delta
data: {"type":"message.delta","delta":"方案"}

event: message.completed
data: {"type":"message.completed","reply":"## 迁移方案\n...","usage":{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150}}
```

| SSE 事件 | 含义 |
|---------|------|
| `run.started` | 会话已建立，服务端确认收到 `SessionKey` |
| `run.progress` | 处理进度提示（可忽略） |
| `message.delta` | 流式文本增量，取 `delta` 字段实时拼接 |
| `message.completed` | 流结束，取 `reply`（完整回复）和 `usage` |

## Session 管理

**服务端按 `SessionKey` 隔离对话上下文，当前对话中 SessionKey 必须保持不变。**

| 场景 | 处理方式 |
|------|---------|
| 首次对话 | 不传 session_id，脚本自动生成 UUID v4 并通过 `SessionKey` 发送 |
| 同一对话追问 | **必须**沿用上次返回的 session_id，通过 `SessionKey` 发送 |
| 用户要求新对话 | 不传 session_id，脚本重新生成新 UUID，服务端创建独立上下文 |

> `SessionKey` 通过请求体传入 Gateway，服务端按此字段维护独立的多轮对话上下文。
> `session_id` 完全由客户端生成和维护，返回结果中的 `session_id` 就是客户端传入的值。

## 清除会话

```bash
python3 {baseDir}/scripts/migrateq_sse_api.py --clear-session
```

## 默认调用规则

当用户问题**没有明确匹配到其他触发词**时，**默认使用 MigraQChatCompletions**。包括但不限于：

- 跨云迁移方案咨询
- 云资源扫描和清单生成
- 目标云选型和规格对标
- TCO 成本分析
- 迁移风险评估
- 迁移工具和步骤指引
- 用户问题含义模糊无法确定具体操作时

## 展示规则

- `content` 为 Markdown 格式，可直接展示给用户
- 若 `success: false`，向用户说明服务暂时不可用，并提示检查环境配置

## 常见错误码

| 错误码 | 含义 | 处理方式 |
|--------|------|---------|
| `NetworkError` | 无法连接 Gateway | 检查 Gateway 地址和网络 |
| `HTTPError` | Gateway 返回 HTTP 错误 | 检查 TENCENTCLOUD_SECRET_KEY 和 Gateway 状态 |
| `MissingParameter` | 脚本调用缺少参数 | 检查调用方式 |
