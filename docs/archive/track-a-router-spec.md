# Track A 设计规格：路由器与思维模式层（MVP 切片）

> 模块路径：`core/src/router/`（新建）
> 形态：规则版任务路由器 + 思维模式匹配（T0/T1 + P0/P1）+ 内置 persona 名册 + 决策日志
> 范围：最小可用闭环（MVP slice），非 ML 权重，非视觉取证，非工作台 UI
> 设据：已先读现有代码并核对行号（见第 7 节“确切集成点”）

---

## 0. 设计总览与目标

### 0.1 问题
Marguerite 引擎当前**没有**任务路由层与思维模式（thinking mode）匹配：
- `core/src/model/router.rs` 的 `ModelRouter` 只做**模型选择**（按 `IntentType`/`Complexity` 选模型），
  不产出“思考预算档位”或“system prompt 注入策略”，也无 persona 概念。
- `core/src/model/thinking.rs` 已有 `ThinkingEffort` 与 `ThinkingProfile`，但**只在工具路径**
  (`ModelRouter::chat_with_tools`) 经 `classify_task` 自适应注入；非工具对话路径
  (`chat_for` / `chat_stream`) **完全不调节思考预算**（始终 `ThinkingEffort::Off`）。
- 网关侧 `core/src/gateway/chat.rs` 已有 `EventType::RouterDecision` 与 `router_decision_payload`，
  但只承载 orchestrator 的 `analysis`，**没有 thinking_mode / prompt_policy / tier / confidence**。

### 0.2 本切片交付
一个纯规则、零 LLM 开销的 `classify()`，把一条用户输入映射为：

```
RouteDecision {
    tier:           PersonaTier,   // 路由到的 persona 档（opensquilla route_class）
    confidence:     f64,           // [0,1] 规则打分
    thinking_mode:  ThinkingMode,  // T0 (无思考) | T1 (开思考)
    prompt_policy:  PromptPolicy,  // P0 (不注入) | P1 (注入策略片段)
    source:         DecisionSource,
    fallback_reason:Option<String>,
}
```

并挂接到对话推理入口，使：
- `thinking_mode` 调节 provider 的**思考预算**（`LLMConfig.thinking` → 请求体 `reasoning_effort`）；
- `prompt_policy` 调节 **system prompt 注入**（追加一条策略 system 消息）。

### 0.3 非目标（本切片不做）
- 不引入 ML 权重 / 神经网络分类（Phase 2+）。
- 不做视觉取证协同（Track B）、不做 roster UI（Track D）。
- 不替换现有 `ModelRouter` 的**模型选择**职责，只在其之上叠加“思考预算 + prompt 策略 + persona 档”决策。
- 不替换 `crate::intent` 的本地意图分类器，而是**组合复用**其输出作为本分类器的输入信号之一。

---

## 1. 模块文件布局（`core/src/router/`）

依赖方向（单向，无环）：

```
core/src/router/
├── mod.rs            // 模块根：公共 API、RouteDecision、ThinkingMode、PromptPolicy、
│                    //        DecisionSource、PersonaTier、ClassifyInput、RouterError、
│                    //        classify() 入口、re-export 子模块
├── signals.rs        // ClassifyInput 构造与信号抽取（复用 crate::intent::Classifier 输出）
├── classify.rs       // 规则版 classify()：打分 → 选档 → 弱路由 → 降级（端口 opensquilla）
├── persona.rs        // Persona 结构 + PersonaCatalog 名册 + 默认 catalog 草例
├── decision_log.rs   // DecisionLog：JSONL 落盘、SCHEMA_VERSION、敏感字段仅存 hash
└── bindings.rs       // 与现有推理路径的适配：ThinkingMode::to_thinking_effort()、
                      //   PromptPolicy::system_fragment()、apply_to_runtime() 等桥接
```

依赖关系：
- `router::*` 依赖：`crate::model::thinking::ThinkingEffort`、`crate::cache::hash_text`、
  `crate::events::{Envelope, EventType, EventBus}`、`crate::intent::Classifier`（输入信号）、
  `crate::persistence::config::Config`（决策日志落盘目录）。
- 反向依赖（`router` 不反向依赖）：`agent::runtime`、`model::router`、`orchestration`、
  `gateway::chat`、`tui::workbench::RealEngine` 单向调用 `router::classify` / `router::bindings`。
- `lib.rs` 新增 `pub mod router;`（置于 `pub mod model;` 之后，`pub mod orchestration;` 之前，
  即当前 `core/src/lib.rs:29` 附近）。

---

## 2. 核心数据结构

> 标识符英文；注释精炼。以下为**类型定义（设计）**，非实现。

### 2.1 思维模式与 prompt 策略
```rust
/// 思考预算档位（opensquilla thinking_mode 的 Rust 对译）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThinkingMode { T0, T1 }

/// system prompt 注入策略（opensquilla prompt_policy 的 Rust 对译）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptPolicy { P0, P1 }
```

### 2.2 persona 档（route_class）
```rust
/// 路由档（opensquilla route_class）。与既有 IntentType/TaskKind 正交：
/// 它表达的是“应选用哪类 persona + 思考/策略组合”，而非“选哪个模型”。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PersonaTier {
    General,    // 通用闲聊 / 轻问答
    Coder,      // 代码生成 / 修复 / 重构
    Reasoner,   // 数学 / 逻辑 / 证明
    Tooler,     // 工具 / 终端 / 浏览器 / 桌面操控
    Explorer,   // 长文档 / 长任务（大上下文）
}
```

### 2.3 决策来源（可观测性 + 弱路由 / 降级标记）
```rust
/// 决策来源：用于决策日志与前端只读展示（对齐现有 router_decision_payload.source）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionSource {
    /// 规则版 classify 正常命中（opensquilla source="rule"）
    Rule,
    /// 弱路由：confidence < 阈值，按历史粘性落到最近 persona（opensquilla source="history"）
    History,
    /// 降级回退：分类器异常或 runtime 不可用 → 默认档（opensquilla v4_unavailable / fallback）
    Default,
    /// runtime 不可用降级（opensquilla 的 "v4_unavailable" 日志语义）
    RuntimeUnavailable,
}
```

### 2.4 单条路由决策（对外核心产出）
```rust
/// 单次 classify 的完整决策。chat.rs 的 router_decision_payload 直接消费本结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub tier: PersonaTier,
    pub confidence: f64,                // [0,1]
    pub thinking_mode: ThinkingMode,    // T0 | T1
    pub prompt_policy: PromptPolicy,    // P0 | P1
    pub source: DecisionSource,
    pub fallback_reason: Option<String>,
    /// 结构化可观测：本决策经历的每个管线步骤（opensquilla PipelineStepRecord）。
    pub steps: Vec<PipelineStepRecord>,
}

/// 管线步骤记录（opensquilla PipelineStepRecord 的对译）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStepRecord {
    pub step_name: String,              // 如 "classify" / "weak_route" / "fallback"
    pub applied: bool,                  // 该步骤是否实际生效
    pub routing_source: String,         // 步骤来源（"rule" / "history" / "v4_unavailable"）
    pub fallback_reason: Option<String>,
}
```

### 2.5 分类输入（opensquilla 端口：含 routing_history / prev_route_decisions / context_tokens_est）
```rust
/// classify 的输入。对齐 opensquilla 的 classify(input)：
/// 消息 + 路由历史 + 上一轮决策 + 上下文 token 估算 + runtime 可用性。
#[derive(Debug, Clone)]
pub struct ClassifyInput {
    pub message: String,
    /// 最近若干轮的 tier 序列（历史粘性用；空 = 无历史）。
    pub routing_history: Vec<PersonaTier>,
    /// 最近若干轮的完整决策（弱路由回退到最近一次生效决策）。
    pub prev_route_decisions: Vec<RouteDecision>,
    /// 当前上下文 token 估算（由调用方用 LLMClient::count_tokens 估算）。
    pub context_tokens_est: usize,
    /// runtime（opensquilla v4 运行时）是否可用；false → 降级默认档 + v4_unavailable 日志。
    pub runtime_available: bool,
}
```

### 2.6 错误类型
```rust
#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("决策日志写入失败: {0}")]
    LogWrite(String),
    #[error("persona 名册未命中 tier={0:?}")]
    UnknownTier(PersonaTier),
}
```

---

## 3. 分类算法（规则版，端口 opensquilla classify）

### 3.1 设计原则
- 纯函数、零 LLM 开销、可单测（与 `crate::model::thinking::TaskKind::classify` 同风格）。
- 复用 `crate::intent::Classifier` 的本地意图分类结果作为**输入信号之一**（不重复造轮子）。
- 每个 persona 带一组 `MatchRule`（关键词 / 正则 / 意图匹配），打分为命中加权。

### 3.2 伪代码
```
fn classify(input: &ClassifyInput) -> RouteDecision {
    // —— 步骤 1：runtime 可用性闸门（opensquilla v4_unavailable 语义）——
    if !input.runtime_available {
        let d = default_decision(
            source        = DecisionSource::RuntimeUnavailable,
            fallback_reason = Some("v4_unavailable".into()),
        );
        return d.with_step(PipelineStepRecord{
            step_name:"classify", applied:false,
            routing_source:"v4_unavailable", fallback_reason:Some("v4_unavailable")});
    }

    // —— 步骤 2：对名册每个 persona 规则打分 ——
    let mut scored: Vec<(PersonaTier, f64)> = CATALOG.personas.iter()
        .filter(|p| p.enabled)
        .map(|p| (p.tier, score_persona(p, &input)))
        .collect();

    // —— 步骤 3：选最高分档 ——
    scored.sort_by(|a,b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
    let (best_tier, best_conf) = scored.first().cloned().unwrap_or((PersonaTier::General, 0.0));

    // —— 步骤 4：弱路由（confidence_threshold = 0.5）——
    const CONFIDENCE_THRESHOLD: f64 = 0.5;
    if best_conf < CONFIDENCE_THRESHOLD {
        // 历史粘性：有上一轮决策则回退到最近一次生效档；否则回退默认 persona
        if let Some(prev) = input.prev_route_decisions.last() {
            return decision_for(prev.tier, best_conf, DecisionSource::History,
                                Some("weak_confidence_stick_prev"));
        }
        return default_decision(DecisionSource::History,
                                Some("weak_confidence_stick_default"));
    }

    // —— 步骤 5：由档派生 thinking_mode / prompt_policy ——
    let p = CATALOG.lookup(best_tier);
    RouteDecision {
        tier: best_tier,
        confidence: best_conf,
        thinking_mode: p.default_thinking_mode,   // T0 / T1
        prompt_policy: p.default_prompt_policy,   // P0 / P1
        source: DecisionSource::Rule,
        fallback_reason: None,
        steps: vec![PipelineStepRecord{
            step_name:"classify", applied:true,
            routing_source:"rule", fallback_reason:None}],
    }
}

// 打分：把消息 + intent 分类结果 + 上下文规模，对照 persona 的 match_rules 累加权重，
// 最后归一化到 [0,1]。
fn score_persona(p: &Persona, input: &ClassifyInput) -> f64 {
    let mut score = 0.0;
    for rule in &p.match_rules {
        if rule.matches(&input.message) { score += rule.weight; }
    }
    // 上下文规模信号：超大上下文更倾向 Explorer 档（在 classify 步骤 2 之后可二次加权，
    // 此处仅作基础分；Explorer 自身的 match_rule 含 "长任务/整个仓库" 关键词）。
    if input.context_tokens_est > 64_000 && p.tier == PersonaTier::Explorer {
        score += 0.3;
    }
    (score).clamp(0.0, 1.0)
}
```

> 说明：阈值 `0.5` 与 opensquilla `confidence_threshold` 一致；低于阈值走“历史粘性”，
> 与 opensquilla 弱路由起点（weak routing）同语义。

---

## 4. 弱路由 + 降级逻辑

### 4.1 弱路由（Weak Routing）
- 触发：`best_conf < 0.5`。
- 行为（历史粘性）：
  - 若 `prev_route_decisions` 非空 → 采用最近一次决策的 `tier`，`source = History`，
    `fallback_reason = "weak_confidence_stick_prev"`；
  - 否则 → 采用默认 persona（`PersonaTier::General`），`source = History`，
    `fallback_reason = "weak_confidence_stick_default"`。
- 产出仍带 `thinking_mode` / `prompt_policy`（取所选 tier 的默认值），保证弱路由下行为连续。

### 4.2 降级回退（Fallback）
- 触发 A：`runtime_available == false`（opensquilla 的 v4 运行时不可用） →
  `source = RuntimeUnavailable`，`fallback_reason = "v4_unavailable"`，tier = 默认。
- 触发 B：`classify` 内部任何异常（如名册为空、打分 panic 防护）→
  `source = Default`，`fallback_reason = "classify_error"`，tier = 默认。
- 降级**绝不阻断对话**：返回默认档决策，调用方照常继续推理，仅记日志。

---

## 5. persona 名册模型 + 默认 catalog 草例

### 5.1 结构与匹配规则
```rust
/// 单条匹配规则（规则版，复用 regex crate，与项目其余处一致）。
pub struct MatchRule {
    pub keywords: Vec<String>,   // 命中其一即 +weight（小写子串匹配）
    pub weight: f64,             // 累加权重，最终归一化到 [0,1]
}

/// 内置 persona（新建，不搬运任何 TS 代码）。
#[derive(Debug, Clone)]
pub struct Persona {
    pub id: String,
    pub tier: PersonaTier,
    pub label: String,
    /// 可选模型偏好（Phase 2 与 ModelRouter 联动；MVP 仅记录，不强制切换模型）。
    pub model_preference: Option<String>,
    pub default_thinking_mode: ThinkingMode,  // T0 / T1
    pub default_prompt_policy: PromptPolicy,  // P0 / P1
    /// persona 角色提示（始终注入的 system prompt 片段，与 P0/P1 策略片段分离）。
    pub system_prompt: String,
    pub match_rules: Vec<MatchRule>,
    pub enabled: bool,
}

/// persona 名册（进程单例 / 配置驱动，MVP 内置默认 + 允许配置覆盖）。
#[derive(Debug, Clone, Default)]
pub struct PersonaCatalog {
    pub personas: Vec<Persona>,
}
impl PersonaCatalog {
    pub fn lookup(&self, tier: PersonaTier) -> Option<&Persona> { /* first match by tier */ }
    pub fn default_catalog() -> Self { /* 见 5.2 */ }
}
```

### 5.2 默认 catalog 草例（MVP 内置）
| id | tier | think | policy | 关键词（草例） | system_prompt 主题 |
|----|------|-------|--------|----------------|--------------------|
| `general` | General | T0 | P0 | 你好 / 闲聊 / 翻译 / 总结 | 通用助手，简洁直接 |
| `coder` | Coder | T1 | P1 | `fn `/`def `/`impl `/`cargo`/代码/bug/修复/重构/test | 代码专家，先想后写、给可运行改动 |
| `reasoner` | Reasoner | T1 | P1 | proof/theorem/证明/推理/solve/数学/imo | 深度推理，拆解约束、举反例 |
| `tooler` | Tooler | T1 | P0 | tool/工具/终端/shell/浏览器/桌面/execute | 主动调用工具完成任务 |
| `explorer` | Explorer | T1 | P1 | 长任务/整个仓库/总结这/超长/entire document | 长上下文处理，渐进压缩 |

> 约束：每个 `tier` 在默认 catalog 中**唯一**；`default_catalog()` 保证非空（否则 classify 降级）。

---

## 6. decision_log 格式

### 6.1 写入时机与落盘
- 每轮对话在 `gateway::chat::run_turn` 调用 `classify` 后，调用
  `DecisionLog::record(&decision, session, turn_id)` 追加一行 JSONL。
- 落盘文件：`{Config::data_dir()}/router_decision_log.jsonl`（与 `sandbox/audit.rs`、`memory/behavior.rs`
  的 JSONL 实践一致，best-effort 追加写，失败仅 `tracing::warn` 不阻断）。

### 6.2 字段与 SCHEMA_VERSION
```json
{
  "schema_version": 1,
  "ts": "2026-08-04T12:00:00Z",
  "session": "ws.x7f3",
  "turn_id": "turn-9b2c",
  "tier": "coder",
  "thinking_mode": "T1",
  "prompt_policy": "P1",
  "confidence": 0.82,
  "source": "rule",
  "model_hash": "9f3a1c...",
  "fallback_reason": null,
  "steps": [
    { "step_name": "classify", "applied": true, "routing_source": "rule", "fallback_reason": null }
  ]
}
```
- `SCHEMA_VERSION`：在 `decision_log.rs` 定义 `pub const SCHEMA_VERSION: u64 = 1;`
  （对齐 `gateway/memory_import.rs:21` 的同名常量范式）。
- 前向兼容：消费方忽略未知字段（`serde` 默认）；新增字段必须带默认值；`schema_version` 自增时
  旧行仍可被新读取端忽略多余/缺失字段。

### 6.3 敏感字段仅存 hash（opensquilla 约束）
- **不**在日志中写原始用户输入、原始 API key、原始模型名。
- 需要可关联时只存 hash：如 `model_hash = format!("{:x}", crate::cache::hash_text(model_name))`
  （复用 `core/src/cache/mod.rs:23` 的 `hash_text`，与现有前缀哈希同源）。
- `message` 内容**完全不入日志**；需要审计时只记 `tier`/`confidence`/`source` 等决策元数据。

---

## 7. 确切集成点（先读后设计，行号已核对）

### 7.1 新增模块声明
- `core/src/lib.rs`（当前 `pub mod model;` 在第 29 行）→ 在其后加 `pub mod router;`。

### 7.2 thinking 预算调节点（thinking_mode T0/T1 → provider 思考预算）
现有链路：**思考预算**承载于 `LLMConfig.thinking: ThinkingEffort`
（`core/src/model/llm.rs:63`）；在请求体注入于 `LLMClient::build_body`
（`core/src/model/llm.rs:485-489`：`if let Some(effort) = self.config.thinking.reasoning_effort() { ... body["reasoning_effort"] = ... }`）。
但**非工具路径从未设置 `thinking`**（默认 `Off`）。

需新增/修改：
1. `core/src/model/llm.rs` 新增 builder（镜像 `with_adaptive_profile` @ `llm.rs:281`）：
   ```rust
   // 新增
   pub fn with_thinking(mut self, effort: crate::model::thinking::ThinkingEffort) -> Self {
       self.config.thinking = effort; self
   }
   ```
2. `core/src/model/router.rs` 在 `ModelRouter` 上新增带路由决策的调用（镜像 `chat_for` @ `router.rs:228`）：
   ```rust
   // 新增：把 RouteDecision.thinking_mode 映射为 ThinkingEffort 后作用于选中客户端
   pub async fn chat_for_route(
       &self,
       intent: &IntentType,
       complexity: Complexity,
       decision: &crate::router::RouteDecision,
       messages: &[Message],
   ) -> Result<LLMResponse> {
       let effort = decision.thinking_effort(); // T0->Off, T1->Deep（见 bindings.rs）
       let primary = self.select_model(intent, complexity).as_ref().clone().with_thinking(effort);
       let chain = self.candidate_chain(Arc::new(primary));
       try_failover_chain(&chain, |c| Box::pin(c.chat_complete_cached(messages))).await
   }
   // 同理新增 chat_stream_route / chat_with_tools_route（覆盖 adaptive 以 route 为准）
   ```

### 7.3 prompt 策略注入点（prompt_policy P0/P1 → system prompt 注入）
现有 system 消息装配在 `AgentRuntime` 三处（均为前置 `Message::System`）：
- `core/src/agent/runtime.rs:247-269`（`process_input`）
- `core/src/agent/runtime.rs:344-367`（`chat_stream`）
- `core/src/agent/runtime.rs:415-428`（`run_tool_loop`）+ `:527-540`（`run_tool_loop_stream`）

需新增（以 `process_input` 为例，另两处同样贴法）：
```rust
// 在 experience_hint 之后、history 之前插入：
if let Some(dec) = &self.route_decision {
    let frag = dec.prompt_policy.system_fragment(); // P0->""  P1->策略片段
    if !frag.is_empty() {
        model_messages.push(Message::System { content: frag, timestamp: Utc::now() });
    }
}
```
- `AgentRuntime` 新增字段 `route_decision: Option<RouteDecision>` 与
  `pub fn set_route_decision(&mut self, d: RouteDecision)`（镜像 `set_experience_hint` @ `runtime.rs:158`）。

### 7.4 收敛点：Orchestrator / RealEngine
- `core/src/orchestration/mod.rs:173` `Orchestrator` 持有 `lead: AgentRuntime`；
  新增 `pub fn set_route_decision(&mut self, d: RouteDecision)`，仅转发 `self.lead.set_route_decision(d)`。
  （镜像 `set_experience_hint` @ `mod.rs:427`、`set_stable_prefix` @ `mod.rs:433`。）
- `core/src/tui/workbench/mod.rs:118` `RealEngine` 新增字段 `route_decision: Option<RouteDecision>`
  与 builder `with_route_decision`（镜像 `with_stable_prefix` @ `mod.rs:159`）；
  在 `execute_turn`（`:188-222`）调用 `orch.set_route_decision(self.route_decision.clone())`
  （紧邻现有 `orch.set_stable_prefix` @ `:220`）。

### 7.5 对话推理入口（网关）
- `core/src/gateway/chat.rs:566` `run_turn` 是**唯一入口**。在 `should_auto_decompose` 分支
  （`:634`）**之前**插入：
  ```rust
  let input = crate::router::signals::classify_input_from_turn(&text, &session, &config);
  let decision = crate::router::classify(&input);
  crate::router::decision_log::DecisionLog::record(&decision, &session, &task_id);
  ```
- 简单路径（`:687-694` `RealEngine`）：`engine.with_route_decision(decision.clone())`。
- Bloom 并行路径（`:657-685`）：`orch.set_route_decision(decision.clone())`（在 `orch.set_mode` @ `:664` 附近）。
- 富路由决策事件（`:169-173` 已 `publish(EventType::RouterDecision, ...)`）：扩展
  `router_decision_payload`（`chat.rs:709`）使其携带 `thinking_mode` / `prompt_policy` /
  `tier` / `confidence` / `source` / `fallback_reason`（复用 `EventType::RouterDecision` @
  `core/src/events/envelope.rs:64`，无需新增事件类型）。

### 7.6 桥接映射（bindings.rs）
```rust
impl ThinkingMode {
    /// T0 -> Off（低延迟、无思考预算）；T1 -> Deep（开思考）。
    /// Bloom 模式下 T1 升级为 Max（见 11 节风险），此处给基础映射。
    pub fn to_thinking_effort(self) -> crate::model::thinking::ThinkingEffort {
        match self { ThinkingMode::T0 => ThinkingEffort::Off, ThinkingMode::T1 => ThinkingEffort::Deep }
    }
}
impl PromptPolicy {
    /// P0 -> 空（不额外注入）；P1 -> 稳定策略片段（思考/格式引导）。
    pub fn system_fragment(self) -> String { /* P0:""  P1:"你在作答前先完整推理…" */ }
}
impl RouteDecision {
    pub fn thinking_effort(&self) -> crate::model::thinking::ThinkingEffort {
        self.thinking_mode.to_thinking_effort()
    }
}
```

---

## 8. 公共 API（router 模块对外暴露）

```rust
// core/src/router/mod.rs
pub use classify::classify;
pub use decision_log::{DecisionLog, SCHEMA_VERSION};
pub use persona::{Persona, PersonaCatalog, MatchRule, default_catalog};
pub use signals::classify_input_from_turn;

pub fn classify(input: &ClassifyInput) -> RouteDecision;
pub struct ClassifyInput { /* 见 2.5 */ }
pub struct RouteDecision { /* 见 2.4 */ }
pub struct PipelineStepRecord { /* 见 2.4 */ }
pub enum ThinkingMode { T0, T1 }
pub enum PromptPolicy { P0, P1 }
pub enum PersonaTier { General, Coder, Reasoner, Tooler, Explorer }
pub enum DecisionSource { Rule, History, Default, RuntimeUnavailable }
pub enum RouterError { /* 见 2.6 */ }

// 调用方（gateway/agent/orchestration）侧签名
impl AgentRuntime     { pub fn set_route_decision(&mut self, d: RouteDecision); }
impl Orchestrator     { pub fn set_route_decision(&mut self, d: RouteDecision); }
impl RealEngine       { pub fn with_route_decision(self, d: RouteDecision) -> Self; }
impl ModelRouter {
    pub async fn chat_for_route(&self, intent:&IntentType, complexity:Complexity,
                                decision:&RouteDecision, messages:&[Message]) -> Result<LLMResponse>;
    pub async fn chat_stream_route(/* 同签名 */) -> Result<StreamReceiver>;
    pub async fn chat_with_tools_route(/* 同签名 + tools */) -> Result<LLMResponse>;
}
impl LLMClient { pub fn with_thinking(self, effort: ThinkingEffort) -> Self; }
```

---

## 9. 单元测试清单（覆盖 正常 / 弱路由 / 降级）

| # | 用例 | 覆盖点 | 期望 |
|---|------|--------|------|
| T1 | 代码提示 → coder 档 | 正常路径 | `tier==Coder`, `confidence>=0.5`, `source==Rule`, `thinking_mode==T1`, `prompt_policy==P1` |
| T2 | 数学证明提示 → reasoner 档 | 正常路径 | `tier==Reasoner`, `thinking_mode==T1`, `prompt_policy==P1` |
| T3 | 闲聊提示 → general 档 | 正常路径 | `tier==General`, `thinking_mode==T0`, `prompt_policy==P0` |
| T4 | 模糊提示（无强信号）→ 弱路由 | 弱路由 | `confidence<0.5`；无历史时 `source==History` 且 tier==默认 General |
| T5 | 弱路由 + 有 prev 决策 | 弱路由 | `confidence<0.5` 且有 `prev_route_decisions` → `source==History`，tier==prev.tier |
| T6 | runtime 不可用 | 降级 | `runtime_available==false` → `source==RuntimeUnavailable`，`fallback_reason=="v4_unavailable"`，tier==默认 |
| T7 | 名册空 → 降级 | 降级 | `default_catalog` 为空时 classify 返回 `source==Default`，不 panic |
| T8 | thinking 映射 | bindings | `T0.to_thinking_effort()==Off`, `T1.to_thinking_effort()==Deep` |
| T9 | prompt 映射 | bindings | `P0.system_fragment()==""`, `P1.system_fragment()!=""` |
| T10 | 注入生效（集成级，mock LLM） | 集成 | T1 时请求体含 `reasoning_effort`；P1 时 model_messages 含策略 system 消息 |
| T11 | 决策日志 | decision_log | 每轮追加 1 行；含 `schema_version`；`model_hash` 为 hash（无明文模型名）；行可独立 `serde_json::from_str` |
| T12 | catalog 不变量 | persona | `default_catalog()` 非空；每个 `tier` 唯一；`PersonaTier` 五档均被覆盖 |

---

## 10. 验收标准（EARS 格式，P0）

- **当** `classify` 接收高信号消息（命中某 persona 的 `match_rules`）**时**，系统**应**返回
  `confidence >= 0.5` 且 `source == Rule` 的 `RouteDecision`。
- **当** `best_conf < 0.5`**时**，系统**应**按历史粘性路由：若 `prev_route_decisions` 非空则采用
  最近一次决策的 `tier` 且 `source == History`，否则采用默认 persona 且 `source == History`。
- **当** `runtime_available == false`**时**，系统**应**返回默认 tier、`source == RuntimeUnavailable`、
  `fallback_reason == "v4_unavailable"`，且**不应**阻断对话。
- **当** 决策 `thinking_mode == T1`**时**，系统**应**令 provider 请求体注入 `reasoning_effort`
  （对 DeepSeek 系；`core/src/model/llm.rs:485`），即对应该模型的思考预算开启。
- **当** 决策 `prompt_policy == P1`**时**，系统**应**在该轮模型消息序列前置一条策略 `Message::System`
  （`core/src/agent/runtime.rs:247-269` 等装配点）。
- **每当** 一轮对话完成路由分类**时**，系统**应**向 `router_decision_log.jsonl` 追加恰好一行
  含 `schema_version` 且敏感字段仅存 hash 的 JSON。
- **若** `classify` 内部发生异常**则**系统**应**降级为默认档（`source == Default`）并继续对话，
  且**不应**向上抛出导致 `run_turn` 失败。
- **当** 前端订阅 `EventType::RouterDecision`**时**，系统**应**在每轮发布携带
  `thinking_mode`/`prompt_policy`/`tier`/`confidence`/`source` 的富事件
  （扩展 `core/src/gateway/chat.rs:709` 的 `router_decision_payload`）。

---

## 11. 已知坑 / 风险 + 后续 Phase 2+ 扩展点

### 11.1 风险与对策
- **R1 非工具路径此前不调思考预算**：`chat_for`/`chat_stream` 长期 `ThinkingEffort::Off`。
  本切片通过 `chat_for_route` / `with_thinking` 修正；需回归测试确认 T1 真正注入 `reasoning_effort`。
- **R2 与 `ModelRouter` 模型选择职责重叠**：本 router 选“思考档 + persona 档”，`ModelRouter` 选“模型”。
  二者 MVP 并存；Phase 2 可让 persona 的 `model_preference` 直接驱动 `ModelRouter.select_model`。
- **R3 Bloom 路径思考档冲突**：Bloom 经 `ModeSpec.bloom.reasoning_effort="max"` 强制 Max 思考。
  **对策**：Bloom 模式下 `T1` 映射升级为 `ThinkingEffort::Max`（而非 Deep），`T0` 仍 `Off`；
  在 `bindings.rs::to_thinking_effort` 增加 `is_bloom` 参数或在 `Orchestrator::set_route_decision` 内覆盖。
- **R4 决策日志 I/O 性能**：每轮追加写磁盘。对策：best-effort、无锁追加、失败仅 warn，
  与 `sandbox/audit.rs` 同策略；若需更高吞吐，Phase 2 改为异步批量刷盘。
- **R5 弱路由历史粘性可能“锁死”错误档**：长时间低置信对话会一直粘旧档。
  对策：`prev_route_decisions` 仅取最近 1 条，且一旦某轮 `confidence>=0.5` 即回到 Rule 路径。

### 11.2 Phase 2+ 扩展点
- **ML 权重注入**：`score_persona` 的规则权重改为可学习权重；`runtime_available` 为真时
  走 opensquilla v4 运行时（ML 分类），为假时退化本规则版（`v4_unavailable` 语义已预留）。
- **Track B（视觉取证）协同**：Explorer/视觉类消息经本 router 识别后，挂接
  `core/src/vision/` 的 `VisionCapability`（当前 `offline_default()` 为 Mock），
  在 `classify` 中增加“图像/视觉”匹配档，路由到视觉 persona。
- **Track D（roster UI）协同**：将 `PersonaCatalog` 暴露给 roster UI，支持用户手动 pin persona、
  覆盖 `default_thinking_mode`/`default_prompt_policy`；决策日志经 `EventType::RouterDecision`
  已对齐 Web UI `session.event.router_decision`，可直接驱动前端路由决策卡片。
- **决策日志分析**：积累 `router_decision_log.jsonl` 后，可离线分析 tier 分布 / 弱路由率，
  反哺 `match_rules` 与阈值调优。
- **与 `crate::intent` 深度整合**：将 `Classifier` 的结构化 `IntentType` 直接作为
  `score_persona` 的一路信号（当前仅用原始 message 文本，Phase 2 接入 `ClassificationResult`）。

---

## 附录 A：本设计未改动任何现有源文件
所有行号引用均为**只读核对**所得，用于指认挂接位置。本 Spec 为设计交付，不含实现代码；
落地时按第 7 节在指认位置新增/扩展方法，不修改既有函数签名语义（仅新增 `route` 变体方法）。
