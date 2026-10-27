# PRD：`wire_api` 每 provider 可选 + Responses API 适配

- 日期：2026-10-21
- 档位：M
- 触发：`deliverables/CODEX_CHATGPT_PARITY_MATRIX.md` §1 第 1 行（🔴 唯一协议级差距）
- 状态：**已实现（2026-10-21）**，六条 AC 全部有断言，见 §9 执行回执

## 1. 问题

Codex 在 `config.toml:113,119` 用 `wire_api = "responses"` 把请求打到 OpenAI **Responses API**。
Marguerite 只有一条路径：`build_chat_url()` 恒拼 `/chat/completions`（`core/src/model/llm.rs:520-532`），
`LLMConfig`（同文件 `:88-118`）也没有任何"协议选择"字段。

后果：

- 只提供 Responses 端点的服务商无法接入；用户只能改 base_url 硬碰；
- 反过来，把 `responses` 端点当 chat 打，会得到 404 或空回复——**错误信息还指向"模型名不对"**，极难排查；
- 与 Codex 的配置语义不兼容，用户迁移时要重写 provider 段。

## 2. 目标

- G1：`LLMConfig` / `ModelProfile` / `config.model` 支持 `wire_api ∈ {chat, responses}`，默认 `chat`（零迁移）。
- G2：Responses 路径支持非流式、流式、工具调用三种形态，与 chat 路径**行为等价**（同样的 `LLMResponse` / `LLMChunk` 出口）。
- G3：`base_url` 归一化对两种协议都正确（`/v1` 结尾不重复拼接）。
- G4：配置错误（如 responses + Anthropic）**响亮拒绝**，不静默降级。

## 3. 非目标（NG）

- NG1：不实现 OpenAI Assistants / Conversations 等有状态资源。
- NG2：不改现有 chat/completions 请求体字段语义（除必须提取的公共构造函数）。
- NG3：不做自动协议探测（探测会带来额外请求与延迟；由用户/provider 模板显式声明）。
- NG4：不引入新依赖（reqwest/serde 已足够）。
- NG5：不在本轮做 webui 协议选择 UI（先用配置 + 档案字段；UI 属下一批）。

## 4. 契约

### 4.1 配置

```toml
[model]
provider = "openai"
wire_api = "responses"   # 缺省 = "chat"
```

`ModelProfile` 增 `wire_api`（`#[serde(default)]`），旧 `models.json` 无缝加载。

### 4.2 URL

| wire_api | provider | 结果（base 无 `/v1` 后缀时） |
|---|---|---|
| chat | 非 anthropic | `{base}/v1/chat/completions` |
| chat | anthropic | `{base}/v1/messages` |
| responses | 任意 | `{base}/v1/responses` |

### 4.3 请求体（responses）

```json
{
  "model": "…",
  "input": [{"role": "user", "content": [{"type": "input_text", "text": "…"}]}],
  "instructions": "<system 合并文本>",
  "max_output_tokens": 4096,
  "temperature": 0.7,
  "stream": true,
  "tools": [{"type": "function", "name": "…", "description": "…", "parameters": {…}}],
  "reasoning": {"effort": "high"}
}
```

### 4.4 响应/事件映射

| Responses | 内部类型 |
|---|---|
| `response.output_text.delta` | `LLMChunk{delta}` |
| `response.output_item.added`（function_call） | `PartialToolCall` |
| `response.function_call_arguments.delta` | 累积到工具参数 |
| `response.completed`（含 usage） | `Done` + `TokenUsage` |
| `error` / `response.failed` | `Err`（保留原始 message） |

## 5. 验收（EARS）

- AC1：`wire_api` 缺省时，所有既有请求体与 URL **逐字节不变**（回归用捕获式 mock server 断言）。
- AC2：`wire_api=responses` 且非流式时，能解析 `output_text` 与 `function_call`。
- AC3：流式时按 4.4 映射，`usage` 只在 `response.completed` 记一次（沿用现有"一次记账"约束）。
- AC4：`responses` + `anthropic` 组合 → 启动即报错，错误消息含两个字段名。
- AC5：`base_url` 已含 `/v1`、`/api/v1`、末尾 `/` 三种写法 URL 都不重复。
- AC6：工具调用两轮（call → result → 再回答）在 responses 下可完成。

## 6. 影响面

`core/src/model/llm.rs`（6 条请求路径）、`core/src/model/registry.rs`（`ModelProfile`）、
`core/src/model/selector.rs`（`effective_model_fields` 不含协议，需确认是否透传）、
`core/src/persistence/config.rs`（`ModelConfig`）、`core/src/gateway/config.rs`（`config.get` 透出 `wire_api`）。

## 7. 风险与回滚

| 风险 | 缓解 |
|---|---|
| 6 条路径漏改 → 半修（chat 能跑、stream 挂） | 先抽 `request_url()` / `request_body()` 两个**唯一**构造点，再逐路径替换；用"路径覆盖表"逐条勾 |
| 响应字段名随 OpenAI 版本漂移 | 解析只认 `output_text` / `function_call` 两类稳定事件，未知事件忽略并计数 |
| 误改 chat 路径导致全量回归 | AC1 的"逐字节不变"断言是硬门禁 |
| 用户配错协议 | AC4 启动期校验，错误信息给修复示例 |

回滚：`wire_api` 全部 `#[serde(default)]`；删除响应分支即回到单选路径，配置无需回滚。

## 8. 执行顺序

1. 抽构造函数（不改行为）→ AC1 绿；
2. 加 `WireApi` 枚举 + 配置透传 → AC4/AC5 绿；
3. 非流式解析 → AC2 绿；
4. 流式解析 → AC3 绿；
5. 工具两轮 → AC6 绿。

---

## 9. 执行回执（2026-10-21）

### 9.1 实施与预期的一处偏差（如实记录）

PRD §1 写"6 条请求路径"，实际读代码后确认**只有 4 个收敛点**：
`chat_url()` / `build_body()` / `parse_response()` / 流式 `parse_stream_chunk()`。
4 条对外方法（`chat_complete` / `chat_complete_cached` / `chat_complete_with_tools` / `chat_stream_with_tools` 等）
都经由这 4 个点。因此改动面比预估小，但**风险点不变**：漏改任一点就会出现"非流式能用、流式静默坏"的半修。

### 9.2 落地清单

| 文件 | 改动 |
|---|---|
| `core/src/model/wire.rs`（新增） | `WireApi` 枚举、`as_key`/`from_key`（含别名 + 拒绝拼错）、`validate`、`responses_url`、5 个单测 |
| `core/src/model/mod.rs` | 注册 `pub mod wire` |
| `core/src/model/llm.rs` | `LLMConfig.wire_api`；`chat_url()` 按协议分派（Chat 仍委托原 `build_chat_url`）；`build_body` 增 Responses 分支；`messages_to_responses_input`；`parse_responses_response`；`parse_responses_stream_chunk`；`responses_usage_to_token_usage`；`parse_response` 与两处流式分派分支 |
| `core/src/model/registry.rs` | `ModelProfile.wire_api` + `to_llm_config` 传递 |
| `core/src/persistence/config.rs` | `ModelConfig.wire_api` + `Default` |
| `core/src/gateway/config.rs` | `config.get` 透出 `llm.wire_api` 与 `llm_profiles[*].wire_api` |
| `core/src/gateway/llm_profile.rs` | RPC 可传 `wire_api`：**显式优先、拼错报 `INVALID_WIRE_API`、未传沿用档案原值** |
| `core/src/model/selector.rs` | `profile_from_flags` 支持 `--wire-api` |
| `core/src/{gateway/fleet,gateway/vision,context/session,governance/verifier,orchestration/mod,tui/workbench/mod}.rs` | 补齐构造点：能取到配置的一律透传，本地模型显式写死 `Chat` |
| `core/src/model/wire_api_tests.rs`（新增） | 16 条契约测试，见 §9.3 |

> 关键实现决定：`LLMConfig` 加字段会让**所有**字面量构造编译失败。这不是麻烦，是**保护**——
> 编译器把 9 处生产代码 + 11 处测试代码全部点名，一处都不可能漏。反过来若用
> `#[serde(default)]` 之外的"隐式默认"（如全局变量），漏掉的点会静默走错协议。

### 9.3 AC 对账

| AC | 状态 | 证据（测试名） |
|---|---|---|
| AC1 缺省零回归 | 🟢 载荷级 + 线上级 | `ac1_default_wire_api_is_chat`（默认值 + 旧 JSON 缺字段）<br>`ac1_chat_url_equals_legacy_build_chat_url`（4 种 base_url 与 `build_chat_url` **逐字相等**）<br>`ac1_chat_body_keeps_legacy_shape`（`max_tokens`/嵌套 tools/无 `input`/system 留在 messages）<br>**线上**：`WireApi::Chat` 打真实端点 → `"收到"`，total=17 |
| AC2 非流式解析 | 🟢 载荷级 + 线上级 | `ac2_parses_output_text_and_function_call`、`ac2_reasoning_summary_becomes_thinking`、`ac2_refusal_is_surfaced_not_dropped`、`ac2_error_payload_becomes_err_with_original_message`<br>**线上**：Responses 非流式 → `"收到"` + usage 17（经兼容层修复后才通过）<br>`live_compat_layer_chat_shaped_body_is_still_parsed`（线上逐字响应回归） |
| AC3 流式映射 | 🟢 载荷级 + 线上级 | `ac3_stream_events_map_to_internal_chunks`、`ac3_unknown_events_are_ignored_not_fatal`、`ac3_stream_error_events_become_err`<br>**线上**：22 chunks → `"一、二、三、四、五"` + usage 38 |
| AC4 冲突拒绝 | 🟢 逻辑级 | `ac4_responses_with_anthropic_is_rejected_with_both_field_names`（消息同时含 `wire_api` 与 `anthropic` 并给修复示例） |
| AC5 URL 归一化 | 🟢 | `ac5_no_double_v1_for_any_base_url_shape`（5 种写法，断言不含 `/v1/v1`）+ `responses_url_normalises_like_chat_url` |
| AC6 工具两轮 | 🟡 仅请求体形状 | `ac6_tool_round_pairs_function_call_with_output`（call_id 成对）<br>`ac6_system_promotes_to_instructions`<br>`ac6_tools_use_flat_shape`<br>`ac6_stream_flag_is_carried_into_body`<br>**线上工具链路未验**（见 §9.8） |

### 9.4 测试量纲

- `cargo test -p marguerite-core --lib wire_api_tests` → **19 passed / 0 failed**（含 3 条线上实证回归）
- `cargo test -p marguerite-core --lib model::` → **320 passed / 0 failed / 1 ignored**
- `cargo check -p marguerite-core --lib` → 干净（0 error 0 warning）
- `cargo run --example verify_wire_live`（真实端点） → **3/3 PASS，0 失败**

### 9.5 线上级验证（2026-10-21，真实端点）

**端点**：`https://api.apigoto.com/v1`，模型 `gpt-6-astra`（即本机 Codex `config.toml` 里
声明 `wire_api="responses"` 的那一个）。密钥仅经环境变量传入，未写入仓库任何文件。

验证程序：`core/examples/verify_wire_live.rs`（用**生产同一份** `LLMClient` 代码发请求）。

| 场景 | 结果 | 正文 | usage |
|---|---|---|---|
| Responses 非流式 | 🟢 PASS | `"收到"` | prompt=12 completion=5 total=17 |
| Responses 流式 | 🟢 PASS | `"一、二、三、四、五"`（22 chunks） | prompt=25 completion=13 total=38 |
| 对照：`WireApi::Chat` | 🟢 PASS | `"收到"` | total=17 |

**结论：0 个失败。**

### 9.6 线上实证推翻了两个"载荷级"假设（这是本轮最大收获）

线上跑之前，16 条载荷级测试全绿。上线一跑，立刻暴露两个真实缺陷——**两者都不会报错，只会在用户那里表现为"模型没回答"**。

**缺陷 1（严重）：该端点的非流式 `/v1/responses` 返回的是 Chat Completions 格式。**

原始响应：`{"object":"chat.completion","choices":[{"message":{"content":"收到"}}],"usage":{"prompt_tokens":12,...}}`
—— 没有 `output[]`，是 `choices[]`。也就是说这个服务商**只实现了 Responses 的流式一半**，
非流式走了 chat 兼容层。

我的解析器当时只认 `output[]`，于是：
- `content` = `""`（空串）
- `usage` = 全零
- **不报错**

存量测试全绿却线上静默失败。**修复**：`parse_response` 改为按**响应体实际形态**分派
（有 `choices[]` → chat 解析器，否则 Responses 解析器），并加 `live_compat_layer_chat_shaped_body_is_still_parsed`
用**线上逐字响应**做回归。同样的探测也加到了流式解析入口。

> 判据表述：请求协议是我们发的，响应形态是上游决定的，两者**可能不一致**。
> 只按请求协议选解析器，等于相信对端一定守约。

**缺陷 2（中）：Responses 的 `output_index` 不连续。**

线上事件序列：`output_index:0` 是 reasoning item，正文在 `output_index:1`。
我直接用 `output_index` 当累积数组下标，于是第 0 槽是空洞。
下游用 `filter_map(|p| p.name.map(...))` 过滤空洞，所以**没有**产生假工具调用——
但这是"碰巧安全"：空洞会浪费槽位，且一旦下游改成 `map` 就会立刻出错。
已加 `live_output_index_has_holes_and_holes_never_become_tool_calls` 锁死这个不变量。

### 9.7 一次被证伪的怀疑（如实记录）

首次线上流式得到 `content = "三！"`（对提示"数到三：一、二、三"的回答），
我怀疑是解析截断。用同一提示词直接抓原始 SSE 对照：
原始 delta 完整为 `"一、二、三、四、五"`；再用客户端同一提示词跑，得到
`"一、二、三、四、五"`（22 chunks）。

**结论：不是解析问题，是模型对该提示的答法不同。怀疑被证伪，不做任何"修复"。**
（若当时直接去改解析器"修截断"，就会在没有缺陷的地方引入缺陷。）

### 9.8 仍未验证（诚实标注）

1. **未测工具调用（function_call）的线上链路**：该端点是纯文本代理，本轮没有构造需要
   工具调用的真实场景。AC6 仍停留在请求体形状断言。
2. **未测 Anthropic 混合场景**：唯一硬冲突（responses+anthropic）在启动期被 `validate` 拒绝，
   属逻辑验证，不需要线上。
3. **`/v1/responses` 非流式降级为 chat** 这件事本身值得反馈给服务商；本项目只能做兼容，
   不能改对端。

### 9.6 回滚

`wire_api` 全链路 `#[serde(default)]`，默认 `Chat`。回滚只需把 `chat_url()` / `build_body` /
`parse_response` / 流式分派里的 Responses 分支删掉即可，配置无需迁移、无需清档。
