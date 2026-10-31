# 下一代工作台 · M1 上下文层实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把研究方案 §7「上下文用量展示与三层压缩」在 Marguerite 上**抄改完整**落地 —— 让已上线却从不渲染的上下文压力芯片真的亮起来，把「工具输出进历史前截断」这一层补上，并把压缩后的交接信息结构化成可导出的 `CompactState`。

**Architecture:** 全部落在既有内核上，不新增子系统：`core/src/context/` 新增三个**聚焦模块**（`pressure.rs` 压力档位 + 跨实现载荷；`spool.rs` 工具输出磁盘留底；`compact_state.rs` 结构化交接状态），`context/session.rs` 只加字段（全部 `#[serde(default)]`，零迁移）。网关不新增 RPC 方法 —— 一切经**已有** `usage.status` / `context.status` 的载荷**附加键**下发，避开三处方法表同步与 6 份 locale 的连带改动。前端复用已上线的 `useChatUsageWidget` + `ChatContextPressureChip`，只在输入区脚注加一块用量表。

**Tech Stack:** Rust 2021 / tokio / rusqlite / serde_json（`core/`）；Vue 3 Composition API + Pinia + vue-i18n + Vitest（`webui/`）；跨实现守卫用 `include_str!` 解析 Python 源码（仓内既有先例）。

**Spec:** `D:\Downloads\Marguerite\artifacts\next-gen-architecture-research-2026-10-29.md` §7（本计划实现它，并显式记录其中**三条过期前提**）

---

## 0. 实测校准：spec 说的和仓库现状不一样的地方

计划以**仓库现状**为准。下面每条都是本轮亲自核对的结果，后续任务据此立论。

| spec 的说法 | 实测 | 本计划的处理 |
|---|---|---|
| §7.1「用户看不到当前会话 token 用量」 | **半过期**。前端整条链路早已上线：`webui/src/composables/chat/useChatUsageWidget.ts:110-118` 算 `contextWarning`，`webui/src/components/chat/ChatContextPressureChip.vue`（`data-testid="context-pressure-chip"`）已接进 `ChatView.vue`，i18n 键 `chat.contextPressure` 六语齐备（`webui/src/locales/en.json:4889`） | 真缺口是**纯 Rust 部署里这颗芯片永远不亮**，而且是**两层叠因**（施工前核对前端才暴露，原计划只看到第一层）：① `contextStatus` 只有侧车发（`pyengine/src/opensquilla/gateway/rpc_usage.py:285`；`core/src/usage/mod.rs` 全文 0 命中）；② 更根本的是 `sessions` 这个键**两侧类型不同** —— 侧车回**行数组**（`rpc_usage.py:701`），Rust 回**会话计数（数字）**，而消费者是 `usage?.sessions \|\| []` 后 `.find(...)` ⇒ 对数字调 `find` 抛 TypeError、被外层空 `catch {}` 吞掉。⇒ **只补载荷不补类型，芯片仍然不亮、脚注 token 仍然不刷新** ⇒ Task 1 |
| §7.1「压缩策略只有侧车 compact.rs」 | **过期**。Rust 侧 `core/src/context/session.rs`（2470 行）是真正的压缩引擎：`COMPACT_RATIO=0.85`:30、`WARN_RATIO=0.80`:32、`KEEP_RECENT_TURNS=8`:34、`compact()`:583（L4 快照→L1 `snip_long`:709→L2 `select_summary`:611→L3 verbatim 尾）。回合后自动压缩已在跑（`chat.rs:2635`、`:2745`、`:2830`、`plan/runner.rs:1633`） | 「三层压缩」的 L1/L2 其实存在；缺的是 spec §7.2 **第 0 层（入口截流）** 与 **结构化 CompactState**。⇒ Task 3、Task 4 |
| §7.2「PTY/工具输出进历史前 middle truncation，中间写临时文件只给路径」 | **缺**。源头封顶有（`security/exec.rs:40` `DEFAULT_MAX_OUTPUT = 256KB`、`tools/terminal.rs:917 MAX_TOOL_READ_CHARS`），但一旦进入历史就是**全文**；契约测试把它钉死了：`agent/stream_loop.rs:1352` `assert_eq!(tool_out.len(), 40_000)` | 新增入口截流 + 磁盘留底。⇒ Task 3 |
| §7.3「新增 `rpc_compact.rs`（`compact.status`/`compact.force`）」 | **不该照抄**。`context.status`（`gateway/methods.rs:431`→`:2414`）与 `context.compact`（`:248`→`:820`）已存在且语义正是这两条 | 用**附加键**扩展既有载荷，不新增方法（新增方法要同步 `protocol.rs SUPPORTED_METHODS` + `webui/scripts/check-rpc-contract.mjs` + 6 locale，纯增加漂移面） |
| §7.2「存结构化 CompactState」 | **缺**。`memory/compact.rs`（586 行）里 `compact_messages`:178 / `token_economics`:259 / `render_timeline`:281 / `hybrid_retrieve`:323 **无任何生产调用者**（孤儿），只有 `as_stable_prefix`:302 活着（`federated.rs:295`、`cross_project.rs:581`） | 不激活那堆孤儿函数；新写一个**只填真能导出的字段**的 `CompactState`，导不出的显式 `null`。⇒ Task 4 |
| （顺带发现的假承诺）`ToolCompressor` 的压缩标记写着 `retrieve full via tool_compress.retrieve(<fp>)` | **该工具不存在**。`retrieve_full`:198 只有单测调用者；全仓 `tool_compress` 命中只有 `agent/tool_cycle.rs`（且只挂在 cached agent 路径 `:290/:476/:542/:560`） | 属于 AGENTS.md R54 同族缺陷（文档承诺了、代码没兑现）。Task 3 把「取回」做成**真能取回**：磁盘留底 + 注册一个真工具 |

**明确推迟（写清楚，不要静默不做）**

- **发送前压缩钩子**：spec 的「在 prompt cache 形成之前截断」由 Task 3 的**入口截流**满足（一次成型、此后不再改历史 → 前缀稳定）。真正的 pre-send 重写历史会把 `chat.rs` 的回合后压缩语义搬到回合前，与 Bloom 红线 #1（AGENTS.md §3.1「会话内只允许增量追加」）有张力，需要单独对齐 → 不在 M1。
- **精确 BPE 计数**：`model/tokenizer.rs:94 count_exact` 依赖 `VOCAB_FILE "deepseek-v3.json"`:36，该文件不随仓库发布 ⇒ 现状只能用 `estimate_tokens`（CJK +1.6 / ws +0.3 / else +0.25）。本计划沿用估算口径并**在载荷里如实标注 `tokenSource`**，不假装精确。

## Global Constraints

每个任务都隐含遵守本节：

1. **只多不少**：不得删减任何现有功能、现有载荷键、现有断言的强度。契约测试可以**改写**（换更准的断言），不得**删弱**。
2. **默认回环**：网关 / ACP / 控制台只监听 `127.0.0.1`，本计划不得引入任何公网监听或对外请求。
3. **探针永不碰真实 `~/.marguerite`**：任何手工验证都用 `MARGUERITE_CONFIG_DIR` 指到临时目录（AGENTS.md §2 已实测 `USERPROFILE`/`HOME` 在 Windows 上无效，必须用这个变量）。
4. **不提交、不推送**，除非用户明示。每个任务的收尾产物是**新鲜的命令输出**，不是 commit。
5. **完成声明的证据标准**（AGENTS.md §8）：`bash scripts/verify.sh --clean --all` 每条 `[PASS]` + exit 0；`[SKIP]` 永远不算通过。跑闸门前先停 daemon 与孤儿侧车（`tasklist | grep -iE "marguerite|python"`；本轮实测 `target/debug/resources/python/python.exe` 残留进程会让 `gui/src-tauri/build.rs` 报 `os error 32`）。
6. **并发纪律**：同一工作区不同时跑两个 cargo；不同时跑两个 `verify.sh --all`（`check_webui_e2e` 固定占 `18877` 并按端口 `taskkill /T`）。
7. **CLI/UI 文本契约**：不用 emoji 图标，文本标记用 ASCII `[OK]/[FAIL]`；尊重 `NO_COLOR`；门禁前 `env -u NO_COLOR`（否则 2 条既有主题测试走单色回退假红）。
8. **前端守卫**：改 `webui/` 必须过 `npm run check:architecture`（13 项）；新文案必须进 6 份 locale（`de/es/en/fr/ja/zh-Hans.json`）。

---

## 文件结构（本计划落地后的形态）

| 文件 | 职责（单一） | 任务 |
|---|---|---|
| `core/src/context/pressure.rs` | **新增**：把台账读数折算成跨实现一致的 `contextStatus` 载荷 + 三档压力阶梯（60/75/85）；唯一导出 `contextStatus` 的地方 | T1 T2 |
| `core/src/context/spool.rs` | **新增**：工具输出原文的磁盘留底（内容指纹 → 路径 → 取回），带总量上限与淘汰 | T3 |
| `core/src/tools/builtin/fs.rs`（或 `tools/builtin.rs` 注册表） | 注册**一个**真工具 `tool_output.retrieve`，兑现压缩标记里的取回承诺 | T3 |
| `core/src/context/compact_state.rs` | **新增**：结构化交接状态；每个字段声明「数据从哪个真实模块取」以及「取不到时是 `null`」 | T4 |
| `core/src/context/session.rs` | **改**：`SessionContext` 加 3 个 `#[serde(default)]` 字段（`context_window` / `tool_output_tokens` / `compact_state`）；`context_status()` 附加键 | T1 T2 T3 T4 |
| `core/src/persistence/config.rs` | **改**：新增 `ContextConfig`（`context.*`），全 `#[serde(default)]` ⇒ 旧配置文件零迁移 | T2 |
| `core/src/usage/mod.rs` | **改**：`usage.status` 的 `sessions` 从「会话计数（数字）」改为**侧车形状的行数组**，`contextStatus` 落在**每行内**（消费者只读 `sessions[i].contextStatus`） | T1 |
| `core/src/agent/stream_loop.rs` / `runtime.rs` | **改**：工具输出写进历史的**那一行**前过入口截流；重写 E12 契约为三向分裂断言 | T3 |
| `webui/src/components/chat/ChatContextMeter.vue` | **新增**：输入区脚注用量表（已用/窗口/分项/阶梯） | T5 |
| `webui/src/components/chat/ChatComposer.vue` | **改**：`.chat-input-footer`（:130）挂载用量表；无 `contextStatus` 时**整块不渲染**（不是渲染 0%） | T5 |

---

## Task 1：让纯 Rust 部署的上下文压力芯片真的亮起来

**Files:**
- Create: `core/src/context/pressure.rs`
- Modify: `core/src/context/mod.rs`（`pub mod pressure;`）
- Modify: `core/src/context/session.rs`（`SessionContext` + `set_budget` + `context_status`）
- Modify: `core/src/usage/mod.rs:984`（`pub fn status(params: &Value) -> Value`）
- Test: `core/src/context/pressure.rs` 内 `#[cfg(test)]`；跨实现守卫同文件

**Interfaces:**
- Consumes: `crate::context::session::context_status(&str) -> Value`（已有，键：`used_tokens`/`overhead_tokens`/`budget`/`ratio`/`alert`/`compaction_count`）
- Produces:
  - `pub fn context_status_payload(session: &str) -> Option<Value>` —— 形状与 pyengine `_context_status` 的返回**逐键一致**
  - `SessionContext.context_window: u64`（新字段，`#[serde(default)]`，0 = 未知）
  - `context_status()` 输出新增 `"context_window": u64`

- [x] **Step 1：写失败的跨实现守卫（先证明「缺」）** —— 2026-10-29 落地为 `rust_context_status_covers_every_pyengine_key`

在 `core/src/context/pressure.rs` 末尾写入测试模块。守卫直接读 Python 源码，把 `_context_status` 函数体里返回字典的顶层键抠出来，与 Rust 载荷的键集比对 —— 这个先例是 `test_control_ui_security_headers_match_pyengine_middleware`（AGENTS.md R53）。

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// pyengine 的 `_context_status` 返回了哪些顶层键 —— 从**真实源码**解析，
    /// 不抄清单（抄来的清单会和代码一起过期，那是「恒绿守卫」的配方）。
    fn pyengine_context_status_keys() -> std::collections::BTreeSet<String> {
        const SRC: &str =
            include_str!("../../../pyengine/src/opensquilla/gateway/rpc_usage.py");
        let start = SRC
            .find("async def _context_status(")
            .expect("pyengine 里找不到 _context_status");
        // 只看到下一个 `def` 为止 —— 否则会把别的字典键混进来。
        let body = &SRC[start..{
            SRC[start + 1..]
                .find("\nasync def ")
                .map(|o| start + 1 + o)
                .unwrap_or(SRC.len())
        }];
        let ret = body
            .find("return {")
            .expect("_context_status 应返回字典");
        body[ret..]
            .lines()
            .filter_map(|l| {
                let t = l.trim();
                t.strip_prefix('"')
                    .and_then(|s| s.split_once("\":"))
                    .map(|(k, _)| k.to_string())
            })
            .collect()
    }

    #[test]
    fn rust_context_status_covers_every_pyengine_key() {
        let ours = context_status_from_readings(
            "unit-t1",
            /* used_tokens */ 1000,
            /* overhead_tokens */ 0,
            /* budget */ 4000,
            /* context_window */ 8000,
            /* compaction_count */ 0,
        );
        let ours = ours.expect("窗口与预算都给值时必须产出载荷");
        let keys = pyengine_context_status_keys();
        // 自证：解析器**真的**解析出了东西。空集合会让上面那个循环恒绿 ——
        // 那正是 AGENTS.md 里「单点采样 + 形式错误的模型 = 恒绿的守卫」的同型事故。
        assert!(
            keys.len() >= 8,
            "从 pyengine 源码解析出的键太少（{keys:?}），解析器可能已随重构失效"
        );
        for k in &keys {
            assert!(
                ours.as_object().unwrap().contains_key(k),
                "pyengine 有 `{k}`，Rust 载荷没有 —— 芯片在纯 Rust 部署会永远不亮"
            );
        }
    }
}
```

- [x] **Step 2：跑它，确认失败** —— 红灯实测（2026-10-29 04:17）：
  `rust_context_status_covers_every_pyengine_key ... FAILED`，报
  「pyengine 有 `compactionCount`，Rust 载荷没有 —— 芯片在纯 Rust 部署会永远不亮」（0 passed / 1 failed）。
  守卫先证明「缺」，再实现 ⇒ 它不是恒绿。

Run: `env -u NO_COLOR cargo test -p marguerite-core --lib context::pressure`
Expected: FAIL —— `panicked at 'pyengine 有 \`contextTokens\`，Rust 载荷没有'`（此刻 `context_status_from_readings` 还不存在，先按 Step 3 的最小桩跑起来；若以「编译不过」失败也算确认）。

- [x] **Step 3：加字段（全部 `#[serde(default)]`，零迁移）** —— 落地并随 Step 8 变异实测复验

`core/src/context/session.rs` 的 `SessionContext`（:57）内，`overhead_tokens` 之后追加：

```rust
    /// 当前生效模型的**原始**上下文窗口（token）。0 = 未设置。
    ///
    /// 为什么要单独存：`budget` 是 `window*3/4` 再 `clamp(32K, 1.5M)`
    /// （见 [`budget_for_window`]），**不可逆** —— 反推出来的「窗口」是假的。
    /// 而芯片要显示的是 `已用 / 窗口`，必须用真值。
    #[serde(default)]
    pub context_window: u64,
```

`SessionContext::new`（:104 起）的初始化里补 `context_window: 0,`。

`set_budget`（:1497）改为同时记窗口：

```rust
pub fn set_budget(session: &str, context_window: u64) -> u64 {
    let budget = budget_for_window(context_window);
    if context_window == 0 {
        return budget;
    }
    {
        let mut led = ledger().lock().unwrap_or_else(|e| e.into_inner());
        let ctx = led.status(session);
        ctx.context_window = context_window;
    }
    // ……（以下沿用现有实现，不要改动 budget 的写法与持久化）
```

> ⚠️ 上面最后一段是**示意**：`set_budget` 现有实现已在写台账并落盘。改动只是「顺带把 `context_window` 记进同一个 `SessionContext`」，**不要**新起一次锁或第二次写盘 —— 按现有代码结构在**同一个** `ctx` 上多赋一个字段即可。

- [x] **Step 4：写 `pressure.rs` 的载荷构造（逐键对齐 pyengine，含 snake_case 别名）** —— 键集合由跨实现守卫逐字比对侧车源码

```rust
//! 上下文压力：把台账读数折算成**跨实现一致**的 `contextStatus` 载荷。
//!
//! 存在理由（实测）：`ChatContextPressureChip` 早已上线，但它只认 `usage.status`
//! 里的 `contextStatus`，而这个键此前**只有侧车**发（`rpc_usage.py:285`）。
//! 纯 Rust 部署 —— 也就是安装包降级后的常见形态 —— 芯片永不出现：用户看不到
//! 「即将压缩」，直到撞上 provider 400。这里把缺的那一半补上。

use serde_json::{json, Value};

/// 与 pyengine `_CONTEXT_WARNING_RATIO`（rpc_usage.py:25）同值。
pub const WARNING_RATIO: f64 = 0.85;

/// 由台账读数产出 `contextStatus`；`used == 0 && window == 0` 时返回 `None`
/// —— 宁可不显示，也不显示一个假的 0%。
pub fn context_status_from_readings(
    session: &str,
    used_tokens: u64,
    overhead_tokens: u64,
    budget: u64,
    context_window: u64,
    compaction_count: u32,
) -> Option<Value> {
    // 窗口口径与 pyengine 一致：优先真窗口；只有台账从没记过窗口时，
    // 才退到 budget 并**如实改名** windowSource —— 退化的来源必须是可见的。
    let (window, window_source) = if context_window > 0 {
        (context_window, "model_window")
    } else if budget > 0 {
        (budget, "budget_fallback")
    } else {
        return None;
    };
    let threshold = ((window as f64) * WARNING_RATIO) as u64;
    // `pressure` 只算**一次**并先取整，两种拼写复用同一个变量：
    // `json!` 里同键写两遍会静默保留后者、clippy 不报 —— 那是「看着有别名、
    // 实际少一个键」的假绿。Step 5 的自测专门钉「同值两种拼写」。
    let pressure = if window > 0 {
        (used_tokens as f64 / window as f64).min(1.0)
    } else {
        0.0
    };
    let pressure = (pressure * 1e6).round() / 1e6;
    let out = json!({
        "contextTokens": used_tokens,
        "contextWindowTokens": window,
        "thresholdTokens": threshold,
        "pressure": pressure,
        "warningRatio": WARNING_RATIO,
        "compactionCount": compaction_count,
        "tokenSource": "session_context_tokens",
        "windowSource": window_source,
        "context_tokens": used_tokens,
        "context_window_tokens": window,
        "threshold_tokens": threshold,
        "pressure": pressure,
        "warning_ratio": WARNING_RATIO,
        "compaction_count": compaction_count,
        "token_source": "session_context_tokens",
        "window_source": window_source,
        // 以下三键是 Rust 侧**附加**的（pyengine 没有），前端按可选处理：
        // 分项来自台账已有的两个真实计数，不是猜出来的。
        "chatTokens": used_tokens.saturating_sub(overhead_tokens),
        "toolTokens": overhead_tokens,
        "session": session,
    });
    Some(out)
}

/// 读活台账 → 载荷。会话不存在 = `None`（绝不凭空造一个 0 值会话）。
pub fn context_status_payload(session: &str) -> Option<Value> {
    let st = crate::context::session::context_status(session);
    let used = st.get("used_tokens")?.as_u64()?;
    let overhead = st.get("overhead_tokens").and_then(Value::as_u64).unwrap_or(0);
    let budget = st.get("budget")?.as_u64()?;
    let window = st.get("context_window").and_then(Value::as_u64).unwrap_or(0);
    let count = st.get("compaction_count").and_then(Value::as_u64).unwrap_or(0) as u32;
    context_status_from_readings(session, used, overhead, budget, window, count)
}
```

> ⚠️ 载荷的**键集**由 Step 1 的跨实现守卫钉住，不是由这段代码自证。以后 pyengine 往 `_context_status` 里加一个键，Rust 这边没跟上 —— 那个守卫必须在 CI 里变红（先例：`test_control_ui_security_headers_match_pyengine_middleware`，AGENTS.md R53）。

- [x] **Step 5：补三条自测（含变异面）**

```rust
    #[test]
    fn camel_and_snake_aliases_carry_the_same_value() {
        let v = context_status_from_readings("s", 6000, 1000, 8000, 10_000, 2).unwrap();
        for (a, b) in [
            ("contextTokens", "context_tokens"),
            ("contextWindowTokens", "context_window_tokens"),
            ("thresholdTokens", "threshold_tokens"),
            ("pressure", "pressure"),
            ("warningRatio", "warning_ratio"),
            ("compactionCount", "compaction_count"),
        ] {
            assert_eq!(v[a], v[b], "{a} 与 {b} 必须同值（前端两种拼写都在读）");
        }
    }

    #[test]
    fn window_source_names_the_degradation_honestly() {
        let real = context_status_from_readings("s", 100, 0, 4000, 8000, 0).unwrap();
        assert_eq!(real["windowSource"], "model_window");
        assert_eq!(real["contextWindowTokens"], 8000);
        let fallback = context_status_from_readings("s", 100, 0, 4000, 0, 0).unwrap();
        assert_eq!(fallback["windowSource"], "budget_fallback");
        assert_eq!(fallback["contextWindowTokens"], 4000);
    }

    #[test]
    fn no_window_known_at_all_yields_no_payload() {
        // 0 窗口 + 0 预算 ⇒ 无法给出百分比。返回 None，而不是 0% 的假绿。
        assert!(context_status_from_readings("s", 100, 0, 0, 0, 0).is_none());
    }
```

Run: `env -u NO_COLOR cargo test -p marguerite-core --lib context::pressure`
Expected: PASS。实测（2026-10-29 还原变异后）：**7 passed / 0 failed** —— 计划三条之外，
实现时又补了 `chat_and_tool_split_is_a_real_partition_not_a_guess`（分项之和必须等于总量）、
`empty_session_yields_no_payload_like_pyengine`、`window_source_names_the_degradation_honestly`。

- [x] **Step 6：`usage.status` 回吐侧车形状的 `sessions` **数组**，`contextStatus` 落在**行内****

> ⚠️ **本步是原计划的修正处**（施工前核对前端才暴露）。原写法把 `contextStatus` 挂在 `usage.status`
> 的**顶层**，而唯一的消费者 `webui/src/composables/chat/useChatUsageWidget.ts:161-209` 读的是
> **`sessions.find(...).contextStatus`** —— 顶层挂载的载荷前端**永远取不到**，计划里那几条测试却照样全绿。
> 更根本的是：`sessions` 这个键在两侧**类型不同** —— 侧车回**行数组**（`rpc_usage.py:701`），
> Rust 回**会话计数（数字）**（`core/src/usage/mod.rs`）。前端对数字调 `.find()` ⇒ TypeError 被外层
> 空 `catch {}` 吞掉 ⇒ 纯 Rust 部署里脚注 token 永不刷新、芯片永不出现。
> ⇒ 只补载荷不补类型，产品仍是坏的。所以这一步**照侧车契约把 `sessions` 做成行**。

`core/src/usage/mod.rs`：新增 `Ledger::session_rows(session: Option<&str>)`（按 `session, model`
`GROUP BY` 聚合 `usage_events`，锁内折进 `BTreeMap`，**释放连接锁后**再调 `context_status_payload`），
和唯一的构造点 `session_row_json(key, agg)`；顶层 `pub fn status(params)` 保持两个既有的
零值/台账分支**逐字不动**，只在其后插入 `map.insert("sessions", Value::Array(rows))`。

```rust
// 行内键（camel + snake 双拼，与侧车 `_usage_row` 对齐）：
//   session / sessionKey / key、model（按 token 取主导模型，同数取字典序第一）、
//   inputTokens / outputTokens / totalTokens / cacheReadTokens / cacheWriteTokens、
//   costUsd + costSource、createdAt / updatedAt、
//   contextStatus / context_status —— **永远存在**，无上下文时是 null（照侧车口径）。
```

两处**如实**与侧车不同，不许伪齐：① Rust 台账没有真实账单列，成本是 `price_for` 估算，
所以 `costSource` 如实写 `"Marguerite_estimate"`（与 `query_contract`/`task_cost_view` 同一张表）；
② Rust 只有 `created_ms/max`，没有 `started_at/ended_at` 概念 ⇒ **不编造**这两个键。
`budget_for_window` 会夹紧（2M 与 3M 窗口都映射到 1.5M 预算），所以 `set_budget`
**必须同时比较 `budget` 与 `context_window`**，否则只改窗口时不会落盘。

- [x] **Step 7：加端到端载荷测试（断言**位置**，不只是键存在）** —— 实测 `usage::` 41 passed / 0 failed（含这三条）

```rust
// core/src/usage/mod.rs 的 #[cfg(test)] mod 内
#[test]
fn usage_status_sessions_are_rows_the_chip_can_actually_find() {
    // ① sessions 必须是数组（不是数字）② 按会话键真的 find 得到那一行
    // ③ contextStatus 在**行内**且 contextWindowTokens == 10_000（不是 budget_fallback 的 7500）
    // ④ 既有的会话计数仍在 total_sessions/totalSessions 里存活
    // ⑤ 否定面：不存在的会话键 ⇒ **空数组**（不是别的会话的行）
}
#[test]
fn usage_status_backfills_a_row_for_pressure_without_usage_events() {
    // 有上下文压力、还没有 usage_events ⇒ totalTokens == 0 但 contextStatus 是对象
}
#[test]
fn usage_status_rows_never_include_the_empty_session_bucket() {
    // R64 那条「session='' 垃圾桶」不得回流成一行
}
// ↓ 施工时追加的第四条：函数级绿灯证不了「路由真的挂在会产出这个形状的函数上」
#[test]
fn dispatch_rust_usage_status_answers_with_rows_the_console_can_find() { /* gateway/methods.rs */ }
```

> 断言键集合的测试抓不到「挂错位置」；所以第 ② ③ 条走的是前端的真实查找路径（先 `find` 再读行内键）。

- [x] **Step 8：变异实测（守卫必须有牙）** —— 2026-10-29 实测完成，**四处**变异都有牙，逐字节还原后
  `usage::` 41 passed / `context::pressure` 7 passed / 路线级 1 passed，且**全量 lib 4305 passed / 0 failed / 24 ignored**（61.47s）

分别临时改坏，确认对应测试判红并打印预期信息，然后**逐字节还原**：
1. `set_budget` 不写 `context_window` ⇒ `usage_status_sessions_are_rows_the_chip_can_actually_find` 判红。
   **实测比预期更狠**：回落值不是「预算反推的 7500」而是 **32000** —— `budget_for_window` 的
   `clamp(32_000, …)` 下限被触发（10K 窗口的 3/4 是 7500，夹到 32000）。于是小窗口的会话
   压力被**系统性低报到 2%（实测）而不是 6.4%**，芯片会在用户真的快撑爆时才亮。
   这条就是「必须存原始窗口、不能从预算反推」的最硬证据。
2. 去掉行的附加（`context_status_payload` 不再进 `session_row_json`）⇒ **两条**同时红：
   正常行 `contextStatus:null`、补行同样为 `null`（断言把整行 JSON 打出来，键形状一眼可核）。
3. `sessions` 改回计数（数字）⇒ **三条全红**，包括否定面那条（它也要 `Array` 才能查）。
4. **路线级盲区实证**：`dispatch_rust` 的臂改成 `"usage.status" => Value::Null`（即路由没接上）
   ⇒ 新用例 `dispatch_rust_usage_status_answers_with_rows_the_console_can_find` 判红，
   而同一时刻**函数级 41 条一字未改、全部照绿**。这就是「只测函数证不了路由」的现场证据，
   也是这一条用例存在的唯一理由（R54/R56/R61 同族：契约在两处不一致）。

Run（`--lib` 只接**一个**过滤串，得分两次跑）：
`env -u NO_COLOR cargo test -p marguerite-core --lib usage::` 与
`env -u NO_COLOR cargo test -p marguerite-core --lib context::pressure`
Expected: 还原后全 PASS。

---

## Task 2：60/75/85 三档压力阶梯 + `context.*` 配置（additive）

**Files:**
- Modify: `core/src/context/pressure.rs`（新增 `Tier` + 阶梯与 `attach_tier`）
- ~~Modify: `core/src/context/session.rs`（`context_status()` 加 `"tier"`）~~
  **实际未改**（Step 5 的偏离：那里保持无配置依赖，档位由 RPC 分发层挂载）
- Modify: `core/src/usage/mod.rs`（一次取阶梯，摊到整批行上）
- Modify: `core/src/gateway/methods.rs`（`context.status` 臂挂 `tier` + 路线级用例）
- Modify: `core/src/persistence/config.rs`（新增 `ContextConfig`，`Config` 加 `context` 字段）
- Test: 三处 `#[cfg(test)]`（`pressure.rs`、`persistence/config.rs`、`gateway/methods.rs`）

**Interfaces:**
- Consumes: Task 1 的 `context_status_from_readings`
- Produces:
  - `pub enum Tier { Normal, Watch, Advise, Compact }`（`as_str()` → `"normal"|"watch"|"advise"|"compact"`）
  - `pub fn tier_for(pressure: f64, ladder: [f64; 3]) -> Tier`
  - `crate::persistence::config::ContextConfig { ladder: [f64;3], entry_truncation: bool }`，`Config.context`

**为什么加一档 0.60 而不是改 0.80：** 现有 `alert` 只有 `normal|warning|compact_required`，且**四个生产调用点**在用它（`chat.rs:2631/2745/2830`、`plan/runner.rs:1633`）。改 `alert` 的语义 = 同时改这四处行为，属回归面。**新增 `tier` 键**，`alert` 一个字节不动 ⇒ 「只多不少」。

- [x] **Step 1：写失败的阶梯测试** —— 2026-10-29 落地为 `pressure.rs` 的两条用例

```rust
// core/src/context/pressure.rs 的 tests 模块内
#[test]
fn ladder_has_four_bands_and_uses_the_configured_thresholds() {
    let l = [0.60, 0.75, 0.85];
    assert_eq!(tier_for(0.59, l).as_str(), "normal");
    assert_eq!(tier_for(0.60, l).as_str(), "watch");   // 边界含下界
    assert_eq!(tier_for(0.74, l).as_str(), "watch");
    assert_eq!(tier_for(0.75, l).as_str(), "advise");
    assert_eq!(tier_for(0.84, l).as_str(), "advise");
    assert_eq!(tier_for(0.85, l).as_str(), "compact");
    assert_eq!(tier_for(1.00, l).as_str(), "compact");
}

#[test]
fn a_misconfigured_ladder_falls_back_instead_of_silently_lying() {
    // 降序 / 越界 / 非有限值都是「配置写坏了」，不是「压力算坏了」。
    // 回落默认阶梯，并让调用方拿到的档位仍是单调的。
    for bad in [[0.9, 0.5, 0.3], [0.6, 0.6, 0.6], [f64::NAN, 0.7, 0.8]] {
        let t = tier_for(0.7, sanitize_ladder(bad));
        assert!(matches!(t, Tier::Watch | Tier::Advise), "坏阶梯应回落默认：{t:?}");
    }
}
```

Run: `env -u NO_COLOR cargo test -p marguerite-core --lib context::pressure`
Expected: FAIL（`tier_for` 未定义）。

红灯实测（2026-10-29 13:2x）：11 条编译错误 —— `cannot find function \`tier_for\``、
`cannot find function \`sanitize_ladder\``、`cannot find type \`Tier\``，正是预期的「缺实现」。
同一次输出顺带暴露一条**命名冲突**（写进 Step 2 的处理，别当成我的错）：
`crate::gateway::local_models::tier_for(ram_gb, has_gpu) -> &'static str` 早已存在（私有，
含义是「按内存/显卡选模型档位」）。保留计划里的名字，用**模块路径**区分而不是改名 ——
`local_models` 那个是「选哪个模型」，这里是「上下文有多挤」，两个语义不该合并成一个函数。

- [x] **Step 2：实现阶梯** —— 落地为 `Tier` / `DEFAULT_LADDER` / `sanitize_ladder` /
      `tier_for` / `window_pressure` / `attach_tier`，载荷补 `"tier"` + `"ladder"`，
      12 个调用点全部补参（`grep` 复核：6 参调用残留 **0**）。
      额外两条自测（计划没要求，但它们是 Step 5 那处偏离的地基）：
      `tier_and_ladder_travel_inside_the_payload`（档位与阶梯必须在**同一份载荷**里出去，
      且坏阶梯进、默认阶梯出）与 `attach_tier_uses_the_window_denominator_not_the_budget_ratio`
      （钉死分母，见 Step 5 的 ⚠️）。
      全量 core 单测实测（2026-10-29，`env -u NO_COLOR cargo test -p marguerite-core --lib`）：
      **4316 passed / 0 failed / 24 ignored**（161.06s）。

```rust
/// 上下文压力档位。`Watch`/`Advise` 是**新增的可见提示层**，
/// 不改变 `Compact` 对应的既有自动压缩行为（那由 `alert` 驱动）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Normal,
    Watch,
    Advise,
    Compact,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Normal => "normal",
            Tier::Watch => "watch",
            Tier::Advise => "advise",
            Tier::Compact => "compact",
        }
    }
}

pub const DEFAULT_LADDER: [f64; 3] = [0.60, 0.75, 0.85];

/// 拒绝坏配置：必须**严格递增**且都落在 `(0, 1]`，否则回落默认。
pub fn sanitize_ladder(l: [f64; 3]) -> [f64; 3] {
    let ok = l.iter().all(|v| v.is_finite() && *v > 0.0 && *v <= 1.0)
        && l[0] < l[1]
        && l[1] < l[2];
    if ok {
        l
    } else {
        DEFAULT_LADDER
    }
}

pub fn tier_for(pressure: f64, ladder: [f64; 3]) -> Tier {
    let [watch, advise, compact] = sanitize_ladder(ladder);
    if pressure >= compact {
        Tier::Compact
    } else if pressure >= advise {
        Tier::Advise
    } else if pressure >= watch {
        Tier::Watch
    } else {
        Tier::Normal
    }
}
```

`context_status_from_readings` 签名加一个参数 `ladder: [f64; 3]`，载荷里补：

```rust
        "tier": tier_for(pressure, ladder).as_str(),
        "ladder": ladder.to_vec(),
```

> 同任务内所有调用点（含 Task 1 的测试）跟着补参；Task 1 测试里传 `DEFAULT_LADDER`。

- [x] **Step 3：`context.*` 配置（照 `MemoryConfig` 的既有范式）** —— `ContextConfig`
      落在 `MemoryConfig` 之后、`Config.context` 字段紧随 `Config.memory`。
      动手前先核对了两件事：`Config` 的**每一个**字段都带 `#[serde(default)]`（所以
      Step 4 的「半份 JSON」才可能解析成功），且本文件里**没有** `deny_unknown_fields`
      / 精确键集合断言（所以新增一节不会打断既有 round-trip 用例）。

`core/src/persistence/config.rs` 内，仿 `MemoryConfig` 加：

```rust
/// 上下文压力与入口截流配置（JSON 键 `context.*`）。
///
/// 整块**新增** ⇒ 缺字段的旧配置走 `ContextConfig::default()`，零迁移
/// （同 `MemoryConfig.layers` 的处理方式）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    /// 三档压力阶梯 `[watch, advise, compact]`，必须严格递增且落在 (0,1]。
    /// 坏值由 `crate::context::pressure::sanitize_ladder` 回落默认，不静默生效。
    #[serde(default = "default_context_ladder")]
    pub ladder: [f64; 3],
    /// 工具输出进历史前是否做入口截流（M1 Task 3）。
    /// 默认 **false**：它会改变模型看到的文本，属「静默改 prompt 前缀」那一类，
    /// 必须用户显式打开（同 `memory.tiered_enabled` 默认 false 的理由）。
    #[serde(default)]
    pub entry_truncation: bool,
}

fn default_context_ladder() -> [f64; 3] {
    crate::context::pressure::DEFAULT_LADDER
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self { ladder: default_context_ladder(), entry_truncation: false }
    }
}
```

`Config` 结构体（:511）内追加：

```rust
    /// 上下文压力与入口截流配置（`context.*`）
    #[serde(default)]
    pub context: ContextConfig,
```

- [x] **Step 4：旧配置零迁移测试** —— 两条都落地并实测：`persistence::config` 层
      **18 passed / 0 failed**（2026-10-29，此前 16 条）。`-- --list` 复核确认两条新用例
      **真的在跑**（`an_old_config_file_without_the_context_section_still_loads`、
      `a_ladder_written_backwards_does_not_take_effect`），而不是被过滤串静默排除。

```rust
// core/src/persistence/config.rs 的测试模块内
#[test]
fn an_old_config_file_without_the_context_section_still_loads() {
    // 这就是「零迁移」的可执行证明：删掉整节也必须能解析。
    let cfg: Config = serde_json::from_str(r#"{"model":{"provider":"openai"}}"#)
        .expect("缺 context 节的旧配置必须解析成功");
    assert_eq!(cfg.context.ladder, crate::context::pressure::DEFAULT_LADDER);
    assert!(!cfg.context.entry_truncation);
}

#[test]
fn a_ladder_written_backwards_does_not_take_effect() {
    let cfg: Config =
        serde_json::from_str(r#"{"context":{"ladder":[0.9,0.5,0.3]}}"#).unwrap();
    assert_eq!(
        crate::context::pressure::sanitize_ladder(cfg.context.ladder),
        crate::context::pressure::DEFAULT_LADDER
    );
}
```

- [x] **Step 5：`context.status` 补 `tier`（同一份载荷，别处不重算）**

      原文写法（保留在这里，作为下面这条偏离的对照）是在 `context/session.rs` 的
      `context_status()` 里、`"alert"` 那行**之后**插入：

      ```rust
              "tier": crate::context::pressure::tier_for(
                  ratio,
                  crate::persistence::config::Config::load()
                      .map(|c| c.context.ladder)
                      .unwrap_or(crate::context::pressure::DEFAULT_LADDER),
              ).as_str(),
      ```

      原文自带的 ⚠️：「`context_status()` 目前不读配置。若 `Config::load()` 在这里会造成
      锁重入或循环依赖（`context → persistence::config → ?`），**改为**由调用方传阶梯：
      `context_status()` 保持无配置依赖……以实测编译结果为准，两条路都走通即可，
      **不要**为了少改一行引入新的依赖边。」 —— 走的就是这条，但偏离得比原文更多一点；
      两个理由都是量出来的，不是审美：

      **① 字面照抄会发出两个档位（同一个会话）。** 上面那段用的是 `ratio`，而
      `ratio` 的分母是**预算**；芯片的 `pressure` 分母是**窗口**。`budget_for_window`
      带 `clamp(32_000, …)` 下限，于是小窗口的预算口径**系统性偏低**：实测 10K 窗口 /
      6000 token 时 `ratio = 0.1875`（`normal`）而真值 `pressure = 0.60`（`watch`）。
      两条 RPC 就会对同一个会话显示两档。所以档位只由**一个实现**
      （`pressure::window_pressure`）算出：载荷内部用它，`context.status` 那条臂用
      `attach_tier()` 走同一条路。分母之差被专门一条用例钉住
      （`attach_tier_uses_the_window_denominator_not_the_budget_ratio`），Step 6 的
      M4 变异（改回 `ratio`）实测把它打红。

      **② `Config::load()` 放进逐行路径 = 每行一次读盘，还可能写用户主目录。**
      `Config::load()` 无缓存（每次 `read_to_string` + YAML 解析），且**配置文件不存在时
      会 `save()` 一份默认配置**。而 `usage.status` 会为每个会话各产一份载荷 —— 把它接进
      `context_status()`，等于「列一次会话」触发 N 次解析，也让单测有可能写
      `~/.marguerite/config.yaml`（与 AGENTS.md R64「单测曾把夹具写进真实的
      `~/.marguerite/usage.db`」同一类缺陷）。落地做法：`context_status_payload()`
      **不读配置**、阶梯由参数给定；`configured_ladder()` 只在 RPC 分发层取一次
      （`methods.rs` 的 `context.status` 臂、`usage::status()` 顶部），再往下传给
      `session_rows` / `session_row_json` / 补行分支；`configured_ladder()` 在
      `cfg!(test)` 下直接回 `DEFAULT_LADDER`（**不碰磁盘**）。阶梯与配置的连线由
      Step 4 那两条纯解析用例负责，它们不落盘。

      **③ 补了一条路线级用例**（Task 1 的教训：键集合测试证明不了挂载位置）：
      `dispatch_rust_context_status_carries_the_same_tier_the_chip_reads` 经真实的
      `dispatch_rust` 断言 `tier` / `ladder` 挂在 `context.status` 上，且与
      `usage.status` 行内那份**完全相等**。

      实测（2026-10-29，分两次跑）：`context::` **87 passed / 0 failed**、
      `persistence::config` **18 passed / 0 failed**、`dispatch_rust` **2 passed / 0 failed**。

Run（一个 `--lib` 过滤串只能给一个前缀，分两次）：
`env -u NO_COLOR cargo test -p marguerite-core --lib context::` 与
`env -u NO_COLOR cargo test -p marguerite-core --lib persistence::config`
Expected: PASS。

- [x] **Step 6：变异实测**

把 `tier_for` 的 `>=` 改成 `>` ⇒ `ladder_has_four_bands…` 的 `tier_for(0.60) == "watch"` 红。把 `sanitize_ladder` 改成恒等（不校验）⇒ `a_misconfigured_ladder…` 红。还原后复跑 PASS。

> **2026-10-29 实测完成：四**处**变异都有牙，逐字节还原（每条都打印 `还原校验: sha256 一致 ✓`），无 `.t2bak` 残留。**
> 脚本 `.tmp/t2_mutations.sh`（`apply()` 用 Python 精确子串替换，命中数 ≠ 1 直接 `exit 3`，绝不"顺手改到别处"）。
>
> | # | 变异 | 变红的用例（实测原文） |
> |---|---|---|
> | M1 | `tier_for` 的 `>=` → `>` | `context::pressure::tests::ladder_has_four_bands_and_uses_the_configured_thresholds` — `0 passed; 1 failed` |
> | M2 | `sanitize_ladder` → 恒等（`let _ = ok; l`） | `context::pressure::tests::a_misconfigured_ladder_falls_back_instead_of_silently_lying` — `0 passed; 1 failed` |
> | M3 | 摘掉 `context.status` 臂上的 `attach_tier` | `gateway::methods::local_hub_contract_tests::dispatch_rust_context_status_carries_the_same_tier_the_chip_reads` — `1 passed; 1 failed`（**这条就是 Task 1 那条教训要的东西**：函数级用例证明不了路线，摘掉分发层的挂载才红） |
> | M4 | `attach_tier` 改用预算口径的 `ratio` | `context::pressure::tests::attach_tier_uses_the_window_denominator_not_the_budget_ratio` — `0 passed; 1 failed`；同轮 `dispatch_rust` 仍 `2 passed`（见下面的坑） |
>
> ⚠️ **踩到一个会骗人的测量坑，值得固化**：还原用 `mv "$file.t2bak" "$file"` 会把源文件的 mtime
> **拨回备份发生的那一刻**（比变异版更早），而 cargo 判陈旧只看 mtime —— 于是「全部变异已还原 ⇒ 复跑绿」
> 那一步**复用的还是 M4 的变异二进制**，报出 `context::` **86 passed / 1 failed**。
> **它不是代码缺陷，是我的验证方法缺陷**：① 原样复跑**精确复现**同一条红、且红的正是 M4 的牺牲者
> `attach_tier_uses_the_window_denominator_not_the_budget_ratio`（磁盘上的源码此时已经是对的）；
> ② 只 `touch` 两个源文件、**先证 sha256 逐一不变**（`touch 未改内容 ✓`）；③ 复跑即
> `context::` **87 passed / 0 failed**、`persistence::config` **18 passed / 0 failed**、`dispatch_rust` **2 passed / 0 failed**。
> **判据（可复用）**：任何用 `cp`/`mv`/`git checkout` 还原源文件再做变异实测的脚本，**还原后必须 `touch`**，
> 否则「还原后复跑」测的是上一个变异的二进制 —— 这与 §4「stale core」「增量缓存遮蔽」是同一族。

---

## Task 3：第 0 层入口截流 + 把「可取回」的假承诺做成真的

**Files:**
- Create: `core/src/context/spool.rs`
- Modify: `core/src/context/mod.rs`
- Modify: `core/src/agent/stream_loop.rs`（`Message::Tool` 入历史处，约 :717）
- Modify: `core/src/agent/runtime.rs`（同一点，约 :1297）
- Modify: `core/src/tools/builtin.rs` / `builtin/fs.rs`（注册 `tool_output.retrieve`）
- Test: `core/src/context/spool.rs` 内；改写 `stream_loop.rs` 的 E12 契约测试

**Interfaces:**
- Consumes: `crate::context::session::{estimate_tokens, record_tool_output_with_call_id}`、Task 2 的 `Config.context.entry_truncation`
- Produces:
  - `pub fn spool_and_truncate(tool: &str, output: &str, budget_chars: usize) -> Truncated`，`Truncated { text: String, fingerprint: Option<String>, dropped_chars: usize }`
  - `pub fn retrieve(fingerprint: &str) -> Option<String>`
  - 新工具名 `"tool_output.retrieve"`

- [x] **Step 1：写 spool 的失败测试（含磁盘与安全边界）**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_truncation_keeps_head_and_tail_and_labels_the_gap() {
        let raw = format!("{}{}{}", "H".repeat(5_000), "M".repeat(30_000), "T".repeat(5_000));
        let t = spool_and_truncate("big_out", &raw, 4_000);
        assert!(t.text.starts_with('H'));
        assert!(t.text.ends_with('T'));
        assert!(t.dropped_chars > 20_000);
        assert!(t.text.contains("[已截断"), "必须如实标注，不能静默丢弃：{}", &t.text[3800..]);
        assert!(t.text.chars().count() <= 4_000 + 300);
    }

    #[test]
    fn under_budget_is_byte_identical() {
        let raw = "一行普通输出".to_string();
        let t = spool_and_truncate("read", &raw, 4_000);
        assert_eq!(t.text, raw, "未超预算零改动（同 ToolCompressor 的保真纪律）");
        assert_eq!(t.dropped_chars, 0);
        assert!(t.fingerprint.is_none(), "不该为小输出留底");
    }

    #[test]
    fn fingerprint_roundtrips_the_exact_original_from_disk() {
        let raw = "x".repeat(60_000);
        let t = spool_and_truncate("term", &raw, 4_000);
        let fp = t.fingerprint.expect("超预算必须留底并给出指纹");
        assert_eq!(retrieve(&fp).as_deref(), Some(raw.as_str()), "取回必须逐字节等于原文");
    }

    #[test]
    fn a_fingerprint_can_not_escape_the_spool_directory() {
        // 工具参数由模型给 ⇒ 路径遍历必须挡住。
        assert!(retrieve("../..%2fetc/passwd").is_none());
        assert!(retrieve("").is_none());
        assert!(retrieve(&"a".repeat(65)).is_none());
    }

    #[test]
    fn spool_stays_within_its_total_cap_by_evicting_oldest() {
        for i in 0..(MAX_FILES + 5) {
            spool_and_truncate("t", &format!("{i}{}", "z".repeat(10_000)), 1_000);
        }
        let n = list_spooled().len();
        assert!(n <= MAX_FILES, "留底数量必须封顶，否则长任务把盘写满：{n}");
    }
}
```

Run: `env -u NO_COLOR cargo test -p marguerite-core --lib context::spool`
Expected: FAIL（模块不存在）。

- [x] **Step 2：实现 spool**

```rust
//! 工具输出的**磁盘留底**：入口截流丢掉的那段中间内容进这里，模型凭指纹取回。
//!
//! 为什么需要它：`ToolCompressor` 的压缩标记写着
//! `retrieve full via tool_compress.retrieve(<fp>)`，但 `retrieve_full`
//! (agent/tool_compress.rs:198) **没有任何生产调用者**，注册表里也没有那个工具
//! —— 那是「文档承诺了、代码没兑现」（AGENTS.md R54 同族）。
//! 本模块把承诺做实：留底落盘（进程重启也取得回）+ 一个真注册的取回工具。

use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// 单个留底文件的字节上限（防一条超大输出把留目录撑爆）。
pub const MAX_SPOOL_BYTES: usize = 8 * 1024 * 1024;
/// 留底条数上限（超出按 mtime 淘汰最旧）。
pub const MAX_FILES: usize = 512;
/// 头/尾各保留的比例（middle truncation）。
const KEEP_RATIO: f64 = 0.45;

pub fn spool_dir() -> PathBuf {
    crate::persistence::config::Config::data_dir().join("tool_spool")
}

/// 截流结果。`fingerprint == None` 表示未超预算、原文一字未动。
pub struct Truncated {
    pub text: String,
    pub fingerprint: Option<String>,
    pub dropped_chars: usize,
}

/// 保留头 `budget*0.45`、尾 `budget*0.45`，中间丢掉的部分写入留底目录。
///
/// 口径要点：**一次成型**。此后这条历史消息不再被改动 —— 这正是
/// 「在 prompt 前缀缓存形成之前截断」的意义：事后回头改旧消息，
/// 会让改动点之后的服务端缓存全废。
pub fn spool_and_truncate(tool: &str, output: &str, budget_chars: usize) -> Truncated {
    let chars: Vec<char> = output.chars().collect();
    if chars.len() <= budget_chars {
        return Truncated { text: output.to_string(), fingerprint: None, dropped_chars: 0 };
    }
    let keep = ((budget_chars as f64) * KEEP_RATIO) as usize;
    let keep = keep.max(1);
    let head: String = chars[..keep].iter().collect();
    let tail: String = chars[chars.len() - keep..].iter().collect();
    let dropped = chars.len() - head.chars().count() - tail.chars().count();

    let fp = fingerprint(tool, output);
    let written = write_spool(&fp, output);
    let (text, fingerprint) = if written {
        (
            format!(
                "{head}\n[已截断 {dropped} 个字符；原文留底 tool_output.retrieve(\"{fp}\")]\n{tail}"
            ),
            Some(fp),
        )
    } else {
        // 写盘失败也要如实标注，只是不给一个取不回的信封。
        (
            format!("{head}\n[已截断 {dropped} 个字符；原文留底不可用]\n{tail}"),
            None,
        )
    };
    Truncated { text, fingerprint, dropped_chars: dropped }
}

fn fingerprint(tool: &str, output: &str) -> String {
    let mut h = Sha256::new();
    h.update(tool.as_bytes());
    h.update([0u8]);
    h.update(output.as_bytes());
    let digest = h.finalize();
    let mut s = String::with_capacity(32);
    for b in &digest[..16] {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// 指纹当文件名，且**只允许** 32 位小写十六进制 —— 其它一律拒（防路径遍历：
/// 这个参数最终由模型给）。
fn spool_path(fp: &str) -> Option<PathBuf> {
    if fp.len() != 32 || !fp.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        return None;
    }
    Some(spool_dir().join(format!("{fp}.txt")))
}

fn write_spool(fp: &str, output: &str) -> bool {
    let Some(path) = spool_path(fp) else { return false };
    if output.len() > MAX_SPOOL_BYTES {
        return false; // 超上限的原文不落盘：宁可不给取回，也不写半截冒充全文
    }
    if std::fs::create_dir_all(&spool_dir).is_err() {
        return false;
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, output).is_err() {
        return false;
    }
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    evict_oldest();
    true
}

/// 模型侧的取回入口。非法指纹 / 已被淘汰 / 读失败 ⇒ `None`（绝不回一个空的 `Some`）。
pub fn retrieve(fp: &str) -> Option<String> {
    let path = spool_path(fp)?;
    let bytes = std::fs::read(&path).ok()?;
    String::from_utf8(bytes).ok()
}

pub fn list_spooled() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(spool_dir()) {
        for e in rd.flatten() {
            if e.path().extension().and_then(|s| s.to_str()) == Some("txt") {
                out.push(e.path());
            }
        }
    }
    out
}

fn evict_oldest() {
    let mut files = list_spooled();
    if files.len() <= MAX_FILES {
        return;
    }
    files.sort_by_key(|p| {
        p.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    for victim in files[..files.len() - MAX_FILES].to_vec() {
        let _ = std::fs::remove_file(victim);
    }
}
```

> ⚠️ `spool_and_truncate` 里 `if std::fs::create_dir_all(&spool_dir)` 是笔误示意 —— 实际必须是 `spool_dir()`（函数调用）。写完立刻 `cargo check` 会抓到。
> ⚠️ `sha2::Sha256` 已是 core 依赖（`guard/trust.rs:173 hash_fingerprint` 在用）—— **先确认再 import**，不要新增依赖。

- [x] **Step 3：跑 spool 五条测试**

Run: `env -u NO_COLOR cargo test -p marguerite-core --lib context::spool`
Expected: PASS 5 条。

- [x] **Step 4：改写 E12 契约测试为「三向分裂」（不许削弱）**

现有断言（`stream_loop.rs:1352`）是「历史里就是全文 40 000 字符」。入口截流后这条不再成立；**但不能因此把断言删掉** —— 把它换成更强的三向断言：事件流保全文（前端与审计要全文）、留底可逐字节取回原文、历史里的截断段**不计**进台账 `overhead_tokens`（那才是模型真正看到的）。

```rust
    /// E12 回归（M1 Task 3 改写）：工具输出的三个去向**各有各的正确口径**。
    ///
    /// 旧断言只有一条「历史里是全文」，它把三个本应不同的口径压成了同一个：
    /// - **事件流**：给前端与审计看，必须全文（截了用户就看不到日志）；
    /// - **模型历史**：受入口截流约束，只带头尾 + 可取回的指针；
    /// - **台账 `overhead_tokens`**：按「真正进入模型上下文」计量（AGENTS.md 里
    ///   `overhead_tokens` 的口径注释就是这个意思）—— 所以截流后必须按**截断后**计。
    /// 三者分不开，用量表和压缩阈值就会一起骗人。
    #[tokio::test]
    async fn stream_loop_splits_tool_output_across_events_history_and_ledger() {
        // ……夹具同现有用例（BigOutputTool 返回 "x".repeat(40_000)）……
        // 打开入口截流（默认关闭 ⇒ 本用例显式打开，见 Step 5 的 config 开关）
        let tool_out = history
            .iter()
            .find_map(|m| matches!(m, Message::Tool { output, .. } then Some(output.clone())))
            .expect("回路应追加 Message::Tool");

        // ① 历史：短于原文，且**如实标注**被截 + 给出可取回的指纹
        assert!(tool_out.len() < 40_000, "入口截流应生效：{}", tool_out.len());
        assert!(tool_out.contains("[已截断"), "不得静默丢弃：{tool_out}");
        let fp = tool_out
            .split("tool_output.retrieve(\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("截断标注必须给出取回指纹")
            .to_string();

        // ② 取回：逐字节等于原文（假承诺 → 真兑现的唯一证明）
        let full = crate::context::spool::retrieve(&fp).expect("指纹必须取得回");
        assert_eq!(full.len(), 40_000, "取回必须逐字节等于原文");
        assert_eq!(full, "x".repeat(40_000));

        // ③ 事件流：仍是全文
        let events = drain(rx).await;
        assert!(
            events.iter().any(|e| matches!(
                e, AgentEvent::ToolResult { output, .. } if output.len() == 40_000)),
            "ToolResult 事件应携带完整输出（前端/审计不因截流而瞎）"
        );

        // ④ 台账：按进入模型的截断后文本计量，而不是按原文
        let status = crate::context::session::context_status(&session);
        let expected = crate::context::session::estimate_tokens(&tool_out);
        assert_eq!(status["overhead_tokens"].as_u64(), Some(expected),
            "台账要等于模型实际看到的：{status}");
        assert!(
            crate::context::session::estimate_tokens(&full) > expected * 3,
            "自证：原文口径确实更大，否则上面这条断言是空断言"
        );
    }
```

- [x] **Step 5：接线（开关默认关 ⇒ 现网行为零变化）**

在 `stream_loop.rs` 与 `runtime.rs` 各**一处**「把 `ToolResult` 落成 `Message::Tool` 并记台账」的现场，把写入文本改为：

```rust
let history_output = if entry_truncation_enabled() {
    crate::context::spool::spool_and_truncate(&tool_name, &raw_output, TOOL_HISTORY_BUDGET_CHARS).text
} else {
    raw_output.clone()
};
// Message::Tool { output: history_output, .. } —— 台账也用 history_output 记
```

`Config::load()` 返回的是 **`Result<Self>`**（`persistence/config.rs:669`），所以开关读取必须照仓内既有写法收口成一个纯函数：

```rust
/// 读配置时按**关闭**处理：入口截流会改变模型看到的文本，属「静默改 prompt 前缀」
/// 那一类 —— 磁盘抖动时绝不能 fail-open 把它悄悄打开。
/// （对照：`memory.auto_capture_enabled` 相反地 fail-open，因为记忆是增强不是门禁。）
fn entry_truncation_enabled() -> bool {
    crate::persistence::config::Config::load()
        .map(|c| c.context.entry_truncation)
        .unwrap_or(false)
}
```

`TOOL_HISTORY_BUDGET_CHARS` 定在 `core/src/context/spool.rs`：`pub const TOOL_HISTORY_BUDGET_CHARS: usize = 6_000;`（约 3.6k token，与 `ToolCompressor` 的 2000 字符同一量级、但作用于**所有**历史写入点而非仅 cached agent 路径）。

> ⚠️ 事件发射点**不动** —— 它继续用 `raw_output`。这正是 Step 4 断言 ① 与 ③ 分叉的地方。

Run: `env -u NO_COLOR cargo test -p marguerite-core --lib agent::stream_loop`
Expected: 新用例 PASS；**其余既有用例一条不少一条不多**（默认关闭 ⇒ 旧行为原样）。

- [x] **Step 6：注册真取回工具**

照 `core/src/tools/builtin/patch.rs:276` 的 `boxed(name, description, schema, handler)` 范式写构造函数（**不要**新增 `impl ToolExecutor`，仓内 builtin 全走 `boxed`）：

```rust
pub(crate) fn tool_output_retrieve_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "tool_output.retrieve",
        "取回**已被入口截流**的工具输出原文全文。指纹取自历史里的 \
         `[已截断 N 个字符；原文留底 tool_output.retrieve(\"<fp>\")]` 标注。 \
         参数: fingerprint(必填，32 位十六进制)",
        json!({"type":"object","properties":{"fingerprint":{"type":"string"}},"required":["fingerprint"]}),
        Arc::new(|args| {
            Box::pin(async move {
                let fp = args
                    .get("fingerprint")
                    .ok_or_else(|| anyhow!("missing 'fingerprint'"))?;
                match crate::context::spool::retrieve(fp) {
                    // 取不到必须是**明确的失败**：回 `ok("")` 会被模型读成「原文就是空的」。
                    Some(text) => ok(text),
                    None => Err(anyhow!(
                        "原文不可用：指纹非法、留底已被淘汰，或该输出从未被截断"
                    )),
                }
            })
        }),
    )
}
```

注册进 `core/src/tools/builtin.rs:127` `register_builtin_tools_with_roots` 的 `tools` 数组（`read_file_tool(...)` 同侧）。

> ⚠️ **不注册到 `core/src/chain/builtin.rs:17`**：链式执行器只有 5 个自带工具（Read/Edit/Search/RunTests/Debug），且**本就不做入口截流** —— 那边没有截断，也就没有可取回的东西。把它塞进链路面只会多一个恒定失败的入口。这条排除要在代码注释里写明理由。

- [x] **Step 7：取回工具的用例 + 变异实测**

```rust
#[test]
fn retrieve_tool_reports_failure_rather_than_empty_string() {
    // 指纹不存在时必须失败，绝不能回 Some("") —— 那会被模型读成「原文是空的」。
    let mut reg = ToolRegistry::new();
    crate::tools::builtin::register_builtin_tools_with_roots(
        &mut reg,
        &[std::env::temp_dir()],
        None,
        None,
        None,
    )
    .expect("注册内置工具应成功");
    let tool = reg
        .get("tool_output.retrieve")
        .expect("取回工具必须已注册进模型可见的工具面");
    let mut args = HashMap::new();
    args.insert("fingerprint".to_string(), "0".repeat(32));
    let r = futures::executor::block_on(tool.execute(&args));
    assert!(matches!(r, ToolResult::Error { .. }), "{r:?}");
}
```

> `ToolResult` 定义在 `core/src/agent/tool_cycle.rs:19`（`Success{data,metadata}` / `Error{message,code}` / `Timeout{tool_name,duration}`）。异步调用方式按该测试模块里**既有**的 `#[tokio::test]` 写法来（本仓多用 `#[tokio::test]` + `.await`，上例的 `block_on` 只是示意 —— 直接写成 async 用例）。

变异：把 Step 5 的 `history_output` 换回 `raw_output` ⇒ E12 用例的 ①（`[已截断`）红；把 `retrieve` 改成 `Some(String::new())` ⇒ Step 7 红。还原后复跑 PASS。

**落地偏差（2026-10-30 回填，以代码为准）** —— 四条，都是被现网事实逼出来的，不是偷懒：

1. **截流入口不是新建的，是既有的**。本仓在 Task 3 开工前**已经有** D11 中间截断：
   `context::middle_truncate::truncate_for_context(tool, output)`，三条工具回路
   （`stream_loop` / `runtime` / `multi_strategy`）共用。所以没有再引入
   `spool_and_truncate(...)` 第二份策略，而是把留底**接在这个唯一入口内部**：
   超预算 → 先 `spool(output)` 拿指纹 → 再 `middle_truncate_with_hint(..., hint)`
   把「真能取回」的指纹写进标注。改一处，三条回路同时生效（这正是 D11 当初拆出
   统一入口的理由 —— 若这里再分叉就把它白建了）。
2. **开关默认从 `false` 翻成 `true`**。原计划写「默认关 ⇒ 现网行为零变化」，前提是
   「本 Task 新引入截流」。前提不成立：D11 现网**已经在无条件截断**，默认关等于
   悄悄关掉一条已上线的行为（且 `entry_truncation=false` 会让历史里的 `[中间省略]`
   标注消失，模型与用户同时失去线索）。因此 `default_entry_truncation() -> true`，
   Task 2 的零迁移测试同步改为 `assert!(cfg.context.entry_truncation)` 并注明理由。
   磁盘抖动仍按 fail-open（`unwrap_or(true)`）：截断是**保护**上下文窗口不炸的那一侧。
3. **工具名不能是点号名**。原计划的 `tool_output.retrieve` 会被 `register_local` 的
   `^[a-zA-Z0-9_-]+$` 清洗成 `legacy_alias`，而 P1-E 回归
   `all_builtin_tool_names_are_legal_at_registration` 断言别名表**必须为空** ——
   用点号名要么红那条守卫，要么模型看到的面包屑（`tool_output_retrieve("…")`）和
   工具真实名不一致，等于给了个取不回的承诺。实际落地 `tool_output_retrieve`，
   截断标注、E12 断言、文档注释同步。
4. **取回工具只注册在 `tools/builtin.rs`，不进 `chain/builtin.rs`**（计划本就如此要求，
   代码注释已写明理由：链式执行器不做截流，没有可取回的东西）。

**证据状态**：spool 五条 + `middle_truncate` 两条 + `builtin/tests.rs` 两条 +
`stream_loop` 的 E12 三向分裂用例已写；权威全绿随 Task 6 的统一门禁给出（本 Task
未单独跑门禁，遵用户「先把所有改动任务完成再跑门禁」）。

---

## Task 4：结构化 `CompactState`（只填真能导出的，导不出的显式 `null`）

**Files:**
- Create: `core/src/context/compact_state.rs`
- Modify: `core/src/context/session.rs`（`SessionContext.compact_state`、`context_status()` 输出）
- Modify: `core/src/gateway/methods.rs`（`context.status` 已转发，载荷多一键即可，**不新增方法**）
- Test: `compact_state.rs` 内

**Interfaces:**
- Produces: `pub struct CompactState { branch: Option<String>, last_commit: Option<String>, dirty_files: Vec<String>, test_status: Option<TestStatus>, active_plan: Option<ActivePlan>, last_user_confirm: Option<UserConfirm>, open_files: Vec<String>, unresolved_questions: Vec<String>, generated_at_ms: u64, source_gaps: Vec<String> }`

**spec 说 7 个字段，本仓能给的不止这些，但有三个必须诚实处理：**

| spec 字段 | 真实数据源 | 判定 |
|---|---|---|
| `branch` / `last_commit` | `tools/git.rs:30 ALLOWED_SUBCOMMANDS` 已含 `"status"`(:31) 与 `"rev-parse"`(:48)；经 `trait GitRunner`(:91) `.run("status", &repo, &[])` 调用（用例见 `git.rs:338/348`） | ✅ 可导出 |
| `dirty_files` | 同上 `git status --porcelain`（子命令在白名单内；破坏性旗标由 `DENIED_FLAGS`:68 挡住） | ✅ 可导出（替换 spec 的 `stash`） |
| `stash` | 引擎**从不写 stash**：文件系统回滚走 `session/rollback.rs::SideGit`(:64，tar+sha256 快照，`snapshot_before_turn`:132)，由 `recovery/mod.rs::RecoveryManager.checkpoint`(:69) 包成整链安全网 | ⚠️ 不照抄：改导出 `dirty_files` + `snapshot_count`（**真实存在**的量），并在 `source_gaps` 记「本引擎不使用 git stash」 |
| `test_status` | `tools/terminal.rs` 有 exitCode，但没有「本会话最近一次测试命令」的结构化记忆 | ⚠️ 只在**本会话真跑过**测试类命令时填；否则 `null` + `source_gaps` 记一笔 |
| `last_user_confirm` | `guard/approval_registry.rs:67 pending_snapshots()`（只读、非消费式）+ 信任表 `guard/trust.rs:140 args_fingerprint` | ✅ 可导出（**只导出指纹不导出明文参数** —— 同 AGENTS.md「参数指纹」那条纪律） |
| `open_files` | `tools/file_change.rs:119 list_file_changes_for(session, limit)`、`:413 list_artifacts(...)` | ✅ 可导出 |
| `active_plan` | `context/session.rs:1384 ContextSessionSummary`（`session`/`message_count`/`updated_ms`/`title`/项目根）+ `plan/` 的计划态 | ✅ 可导出 |
| `unresolved_questions` | 无天然数据源 | ⚠️ 由压缩时的摘要产出（`last_mode == "llm"` 才有），`deterministic` 模式留空 + `source_gaps` 标注 |

> `source_gaps` 是这个设计的**核心纪律**：把「这一栏为什么是空的」写进结构里，让前端能显示「未知（本引擎无此数据）」而不是「0」。AGENTS.md 里反复出现的教训就是**缺失被当成阴性答案**（R57「某一侧对某个 id 的『不知道』被当成另一侧的『不存在』」）。

- [x] **Step 1：写失败测试（钉住「不造假」）**

```rust
#[test]
fn a_field_with_no_source_is_null_and_named_in_source_gaps() {
    // 没有跑过测试命令的会话：test_status 必须是 None，且必须**点名**为什么。
    let s = CompactState::collect_for("mgt-t4-none");
    assert!(s.test_status.is_none());
    assert!(
        s.source_gaps.iter().any(|g| g.contains("test_status")),
        "空白必须可解释：{:?}",
        s.source_gaps
    );
}

#[test]
fn collect_never_fabricates_a_branch_from_a_non_repo_directory() {
    // 未绑定项目 / 非 git 目录 ⇒ branch 是 None，不是 "" 也不是 "main"。
    let s = CompactState::collect_for("mgt-t4-nonrepo");
    assert!(s.branch.is_none(), "不得凭空给 main：{:?}", s.branch);
}

#[test]
fn compact_state_roundtrips_through_json_with_a_missing_field() {
    // 旧台账 JSON 没有 compact_state 键 —— #[serde(default)] 必须让它照样解析。
    let s: CompactState = serde_json::from_str("{}").expect("缺字段的旧载荷必须解析成功");
    assert!(s.branch.is_none() && s.source_gaps.is_empty());
}
```

Run: `env -u NO_COLOR cargo test -p marguerite-core --lib context::compact_state`
Expected: FAIL（模块不存在）。

- [x] **Step 2：实现 collect（每个 `Option` 都写清「取不到就是 None」**

`collect_for(session)` 顺序：读 `context::session::context_status(session)` 拿 `project_path`/`snapshot_count` → 若绑定了项目则经 `tools/git.rs` 跑 `rev-parse --abbrev-ref HEAD` 与 `status --porcelain`（**只走既有白名单**，不新增 git 动词）→ 读本会话是否有测试类命令记录 → 从 `approvals::pending_snapshots()` 与信任表取最近一条已裁决 → `generated_at_ms = SystemTime::now()`。任何一步失败：字段留 `None`，并 `source_gaps.push("test_status: 本会话未执行过测试命令")` 这类**具体**说明。

```rust
/// 压缩后的**结构化交接文档**（spec §7.2 第 1 层）。
///
/// 设计红线：**宁缺毋滥**。每个导不出的字段都是 `None` 且在 `source_gaps`
/// 里有一句话说明。绝不把「没数据」渲染成「值为空 / 值为 0 / 值为 main」——
/// 那是最难查的一类错（AGENTS.md R57）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompactState {
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub last_commit: Option<String>,
    #[serde(default)]
    pub dirty_files: Vec<String>,
    #[serde(default)]
    pub test_status: Option<TestStatus>,
    #[serde(default)]
    pub active_plan: Option<ActivePlan>,
    #[serde(default)]
    pub last_user_confirm: Option<UserConfirm>,
    #[serde(default)]
    pub open_files: Vec<String>,
    #[serde(default)]
    pub unresolved_questions: Vec<String>,
    #[serde(default)]
    pub snapshot_count: u32,
    #[serde(default)]
    pub generated_at_ms: u64,
    /// 「哪些字段为什么是空的」。前端据此显示「未知」而不是「0」。
    #[serde(default)]
    pub source_gaps: Vec<String>,
}
```

- [x] **Step 3：挂进压缩现场并暴露**

在 `context_compact` 成功后（`compact()` 返回 `CompactReport`:653 之处）调用 `CompactState::collect_for(session)` 写入 `SessionContext.compact_state`；`context_status()` 附加：

```rust
        "compact_state": ctx.compact_state,   // Option ⇒ null 时前端按 source_gaps 显示
```

`SessionContext` 加字段（同 Task 1 的加法）：

```rust
    /// 压缩后的结构化交接状态（M1 Task 4）。旧台账缺此键 → `None`。
    #[serde(default)]
    pub compact_state: Option<crate::context::compact_state::CompactState>,
```

- [x] **Step 4：跑该层测试 + clippy**

Run: `env -u NO_COLOR cargo test -p marguerite-core --lib context:: && env -u NO_COLOR cargo clippy -p marguerite-core --all-targets -- -D warnings`
Expected: PASS，零 warning。

**落地偏差（2026-10-30 回填，以代码为准）**：

1. **`collect_for` 的签名不是 `(session)`，而是 `(session, project_path, snapshot_count)`**。
   计划里写 `CompactState::collect_for(session)`，内部再去读 `context_status(session)`。
   这条**会自死锁**：`compact()` 是拿着 `ledger()` 互斥锁调用的（`session.rs:1585` 附近），
   而 `context_status()` / `session_project()` 各自再取一次同一把锁。改成由调用方把
   已经持有的 `ctx.project_path` / `ctx.snapshot_count` 显式传进来，`collect_for` 内部
   **只**再碰 `tools::git` 与 `tools::file_change`（后者用自己的 ledger，不同锁，安全）。
2. **`stash` 不导出**：按表格判定改导出 `dirty_files` + `snapshot_count`，并在
   `source_gaps` 写明「本引擎不使用 git stash（回滚走 tar+sha256 预压缩快照）」。
3. **`test_status` / `active_plan` / `last_user_confirm` / `unresolved_questions` 当前恒为
   `None`/空 + `source_gaps` 点名**。核实过程推翻了表格里两个「✅ 可导出」的判断：
   - `last_user_confirm`：`pending_snapshots()` 只有**尚未裁决**的项，且 `PendingApproval`
     带**明文参数**（与「只导出指纹」纪律冲突）；信任表 `args_fingerprint` 有指纹但没有
     「本次会话最近一次裁决」的时间与工具归因 → 拿不到就不填；
   - `active_plan`：`plans.json` 的 `PlanRevision` / `PlanRun` **都没有 session 键**，
     无法诚实回答「这个会话当前在跑哪个计划」。
4. **写回点在 `Ledger::compact()` 内、`self.persist(&ctx)` 之前**（`session.rs:662`），
   不是 `context_compact()` 里 —— 这样自动压缩与手动压缩都留下一样的交接状态；
   `context_status()` 只负责把它原样吐给前端。
5. 新增回归 `test_compact_writes_structured_state_with_explainable_gaps`（`session.rs`）：
   未绑定项目的会话压缩后 `branch` 必须是 `None`（不是 `""`/`"main"`）、`source_gaps`
   必须点名 `branch`，并换一个**全新 `ContextLedger` 实例**从磁盘读回 —— 内存缓存里
   有、落盘没有，是这个字段最容易出的假绿。

---

## Task 5：输入区脚注用量表（芯片已有，脚注是空位）

**Files:**
- Create: `webui/src/components/chat/ChatContextMeter.vue` + `ChatContextMeter.test.ts`
- Modify: `webui/src/components/chat/ChatComposer.vue:130`（`.chat-input-footer`）
- Modify: `webui/src/composables/chat/useChatUsageWidget.ts`（导出 `tier`/分项/`sourceGaps`）
- Modify: 6 × `webui/src/locales/*.json`
- Test: `webui/src/components/chat/ChatContextMeter.test.ts`

**Interfaces:**
- Consumes: Task 1/2 的 `contextStatus`（含 `chatTokens`/`toolTokens`/`tier`/`ladder`）与 Task 4 的 `context.status.compact_state.source_gaps`
- Produces: `ChatContextMeter` props `{ status: ContextStatus | null }`，无数据 ⇒ **`v-if` 整块不渲染**

- [x] **Step 1：写失败的组件契约测试**

```ts
import { mount } from '@vue/test-utils'
import ChatContextMeter from './ChatContextMeter.vue'

function mk(status: unknown) {
  return mount(ChatContextMeter, {
    props: { status },
    global: { mocks: { $t: (k: string, n?: Record<string, unknown>) => `${k}:${JSON.stringify(n ?? {})}` } },
  })
}

it('renders nothing when the gateway sent no contextStatus', () => {
  // 关键否定面：纯 Rust 且尚未上报的会话 = 没有这块数据。
  // 渲染一个 0% 会教用户误读成「窗口很空」。
  const w = mk(null)
  expect(w.find('[data-testid="context-meter"]').exists()).toBe(false)
  expect(w.find('[data-testid="context-meter-zero"]').exists()).toBe(false)
})

it('shows used / window / per-band breakdown from the real numbers', () => {
  const w = mk({ contextTokens: 60_000, contextWindowTokens: 100_000, chatTokens: 20_000, toolTokens: 40_000, tier: 'advise', ladder: [0.6, 0.75, 0.85] })
  expect(w.find('[data-testid="context-meter"]').exists()).toBe(true)
  expect(w.text()).toContain('60')
  expect(w.text()).toContain('100')
  expect(w.text()).toContain('40') // 工具占比
  expect(w.find('[data-testid="context-meter--advise"]').exists()).toBe(true)
})

it('labels an unknown window as unknown, never as 0%', () => {
  // windowSource=budget_fallback 时窗口不是真模型窗口 → 必须另给一句说明
  const w = mk({ contextTokens: 500, contextWindowTokens: 4000, windowSource: 'budget_fallback', tier: 'normal', ladder: [0.6, 0.75, 0.85] })
  expect(w.find('[data-testid="context-meter-window-approx"]').exists()).toBe(true)
})
```

Run: `cd webui && npx vitest run src/components/chat/ChatContextMeter.test.ts`
Expected: FAIL（组件不存在）。

- [x] **Step 2：实现组件（尺寸/颜色/动效全走 token，禁裸值）**

约束来自 `webui/scripts/`：颜色只能 `var(--token)` 或 `color-mix`（`check-webui-colors.mjs`），圆角走 `--radius-*`（`check-webui-radius.mjs`），时长走 `--dur-*`/`--ease-*`（`check-webui-motion.mjs`），每条选择器带主题前缀的只有 world.css（`check-theme-scope.mjs` 不适用本组件）。文案全走 `t()`（`check-i18n.mjs` 会扫 `t('字面量')` 与 en 字典求差集 ⇒ 漏一个键就 FAIL）。

```vue
<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

type CS = {
  contextTokens?: number
  contextWindowTokens?: number
  chatTokens?: number
  toolTokens?: number
  pressure?: number
  tier?: string
  ladder?: number[]
  windowSource?: string
}
const props = defineProps<{ status: CS | null }>()
const { t } = useI18n()

const pct = computed(() => {
  const s = props.status
  if (!s) return null
  const p = Number(s.pressure ?? 0)
  if (!Number.isFinite(p)) return null
  return Math.round(Math.min(1, p) * 100)
})
const show = computed(() => props.status !== null && pct.value !== null && props.status.contextWindowTokens! > 0)
const usedK = computed(() => Math.round(Number(props.status?.contextTokens ?? 0) / 100) / 10)
const windowK = computed(() => Math.round(Number(props.status?.contextWindowTokens ?? 0) / 100) / 10)
const toolK = computed(() => Math.round(Number(props.status?.toolTokens ?? 0) / 100) / 10)
</script>

<template>
  <div v-if="show" class="ctx-meter" :class="`ctx-meter--${status!.tier ?? 'normal'}`" data-testid="context-meter">
    <span data-testid="context-meter-used">{{ usedK }}k</span>
    <span class="ctx-meter__sep">/</span>
    <span>{{ windowK }}k</span>
    <span class="ctx-meter__pct">({{ pct }}%)</span>
    <span class="ctx-meter__tool" data-testid="context-meter-tool">{{ t('chat.contextMeter.tool', { k: toolK }) }}</span>
    <span v-if="status!.windowSource === 'budget_fallback'" class="ctx-meter__approx" data-testid="context-meter-window-approx">
      {{ t('chat.contextMeter.windowApprox') }}
    </span>
  </div>
</template>
```

> ⚠️ 类名与 CSS 变量以仓库既有 `--dur-*`/`--radius-*` 清单为准（`webui/src/themes/*/foundation.css`）；不要新增裸 `120ms`/`6px`。

- [x] **Step 3：6 份 locale 同步加键**

`chat.contextMeter.tool` / `chat.contextMeter.windowApprox`（位置紧挨既有 `chat.contextPressure`，`en.json:4889`）。六语齐备后跑：

Run: `cd webui && node scripts/check-i18n.mjs && node scripts/check-webui-colors.mjs && node scripts/check-webui-motion.mjs && node scripts/check-webui-radius.mjs`
Expected: 四条 PASS。

- [x] **Step 4：挂进 `.chat-input-footer`（ChatComposer.vue:130）**

在既有 footer 内**追加**（不替换任何现有节点），数据来自 `useChatUsageWidget` 已有的 `contextStatus` ref —— 该 composable 已在 `:193` 做 `contextStatus.value = current.contextStatus ?? current.context_status ?? null`，只需把 `status` 透出并在 composer 侧接线（若跨组件不便，走 `ChatView.vue` 已有的 props/emits 通道传下来，**不新增 store**）。

Run: `cd webui && npx vitest run src/components/chat/ChatContextMeter.test.ts src/components/chat/ChatContextPressureChip.test.ts src/composables/chat/useChatUsageWidget.test.ts`
Expected: 全部 PASS，且**后两份既有用例一字未改**（证明没削弱既有断言）。

- [ ] **Step 5：真机自检（不看单测看像素）**

```bash
MARGUERITE_CONFIG_DIR="$TEMP/mgt_m1_probe" target/debug/marguerite-core.exe gateway --port 18899 &
# usage.status 只在 WS/RPC 通道上（不是 REST），curl 量不到它 —— 用一次性 WS 探针
# 落在 %TEMP%（探针永不碰真实 ~/.marguerite），断言两件事：
#   ① result.sessions 是 **数组**（不是数字）
#   ② sessions[i].contextStatus 是对象，且 contextWindowTokens 等于该会话的真实窗口
```

判据：① 与 ② 必须**同时**成立 —— 只查「payload 里有没有 contextStatus 这个键」会漏掉类型漂移，
而类型错了前端根本取不到。② 之外再核对 `windowSource`：`config`（拿到真窗口）与 `budget_fallback`
（只有预算）两种取值都要能实测出现一次。前端则用 `python scripts/demo/capture_console.py` 既有管线
截一次图，肉眼确认脚注在、数字对、且 `windowSource=budget_fallback` 时多那句说明。

---

## Task 6：收口 —— 全量闸门 + 基线/文档同步

- [x] **Step 1：停掉一切会锁文件的进程**

```bash
tasklist | grep -iE "marguerite|python"
```

Expected: 空。有残留逐个 `taskkill //PID <pid> //F`（含**下划线**命名的测试二进制与 `target/debug/resources/python/python.exe` 孤儿侧车 —— 后者正是本轮 `os error 32` 的来源）。

- [x] **Step 2：全量 clean 闸门**

```bash
env -u NO_COLOR CODEBUDDY_SAFE_DELETE_ENABLED=0 MARGUERITE_PYTHON="C:/Python314/python.exe" \
  bash scripts/verify.sh --clean --all
```

Expected: 每条 `[PASS]`，**0 `[FAIL]`、0 `[SKIP]`**，exit 0，汇总行不得出现「未通过 —— 禁止声称『完成/通过/已修复』」。

- [x] **Step 3：把新基线写回三处文档**

core lib / webui 的测试计数若变化，同步 `AGENTS.md` §2、`README.md` 与 `README.zh-Hans.md` 的 Quality gates 表（`scripts/check-readme-claims.py` 会比对中英两份的硬事实与闸门总数，但**不会**核对数字与现实一致 —— 过期只能靠人复测）。

Run: `python3 scripts/check-readme-claims.py && python3 scripts/check-website-claims.py && python3 scripts/check-doc-cli-drift.py`
Expected: 三条 exit 0。

- [x] **Step 4：把本轮实测事实回填研究方案文档（防止下一次读它的人被过期前提带偏）**

在 `artifacts/next-gen-architecture-research-2026-10-29.md` §7.1 加一段「实测校正」，指向本计划 §0 的表。`artifacts/` 被 gitignore，不随仓库发布 —— 所以这条只是给本机会话的记录，**不得**在被发布文档里引用它。

---

## Self-Review（本计划对照 spec §7 + §8 的覆盖清单）

| spec 条目 | 覆盖 |
|---|---|
| §7.2 第 0 层 middle truncation | T3 ✅ |
| §7.2 第 0 层「太大 hook 输出写临时文件只给路径」 | T3 ✅（spool + 真取回工具） |
| §7.2 第 0 层「PTY 原始输出只留头尾，总量上限」 | 部分：源头已有 `DEFAULT_MAX_OUTPUT` 256KB + `MAX_TOOL_READ_CHARS`；历史侧新增 T3 ⇒ ✅ |
| §7.2 第 0 层「历史清理：不成对 tool call/孤儿输出/不支持图片」 | ❌ **未覆盖** —— 需先实测本仓是否真出现不成对消息；不照抄一个可能不存在的问题（记入「待实测」） |
| §7.2 第 0 层「系统上下文只追加变化部分」 | ❌ 未覆盖，属 M2（与记忆注入同一条线） |
| §7.2 第 1 层 结构化 CompactState | T4 ✅（`stash`→`dirty_files` 的替换有实测依据） |
| §7.2 第 2 层 Prompt Scaffold 重建 | ❌ 未覆盖，属 M1.5（与 `tool_catalog`/`capability_cards` 同线，需单独对齐红线 #1） |
| §7.3 当前/分项/预警/压缩后 | T1+T2+T5 ✅（分项 = chat/tool + `compression_ratio`） |
| §7.3 新增 `rpc_compact.rs` | 刻意不照抄 ⇒ 复用 `context.status`/`context.compact`（理由见 §0） |
| §8.2 第 1 条「新会话加载最近 N 个会话的 CompactState」 | ❌ 未覆盖，属 M1.5，依赖 T4 先落地 |
| §8.2 第 2 条「开场注入 git status/diff --stat」 | ❌ 未覆盖，属 M1.5 |
| §8.2 第 3/4 条 Fork / Replay | ❌ 未覆盖，属 M2（Rust 侧 `sessions.fork/resume/replay` 目前 **0 实现**，只有 pyengine `rpc_sessions.py:2294/2302`，而前端 `forkTransition.ts:90` 已上线 ⇒ 与 T1 同一类「前端等后端」缺口，值得单独一个计划） |

**结论**：M1 = spec §7 的**第 0 层缺口 + 用量展示**，外加 §7.2 第 1 层的结构化状态。§7.2 第 2 层与 §8 全部留给后续切片，且已写明依赖顺序 —— §8.2 的三条**不是**不做，是必须排在 T4 之后。

---

## 实施状态（2026-10-29 17:2x，fresh evidence）

| 任务 | 状态 | 本轮新鲜证据 |
|---|---|---|
| T1 压力载荷 + `sessions` 行数组 | 已落地 | `cargo test -p marguerite-core --lib context::` → **103 passed / 0 failed**（含 pressure 跨实现守卫、usage 行形状） |
| T2 60/75/85 阶梯 + `context.*` | 已落地 | 同上（`ladder_*` / `sanitize_*` / `attach_tier` 用例）+ `persistence::config` 零迁移用例 |
| T3 入口截流 + 磁盘留底 + 真取回工具 | 已落地 | `context::spool` 6 用例（含路径遍历、超大不半写、容量淘汰）全过；`agent::stream_loop` **18 passed**（含三向分裂 `stream_loop_splits_tool_output_across_events_history_and_ledger`）；`tool_output` 过滤 **10 passed**（`tool_output_retrieve_returns_the_exact_spooled_text` / `..._fails_loudly_instead_of_returning_empty`） |
| T4 结构化 `CompactState` | 已落地 | `context::compact_state` 用例全过（非 repo 目录给 gap 而不是空成功、`#[serde(default)]` 旧载荷可解析） |
| T5 输入区脚注用量表 | **本轮补齐** | 新增 `webui/src/components/chat/ChatContextMeter.vue` + `.test.ts`（5 用例：无数据不渲染、窗口未知不渲染、真实分项、缺 tier 按阶梯推导、预算回退标注）；`useChatUsageWidget` 导出 `contextStatus`，`ChatComposer`/`ChatView` 接线，6 语种键齐备；`npm run typecheck`（含 13 项架构守卫 + i18n 对齐）exit 0 |
| T6 全量闸门 + 基线同步 | **已跑通** | `verify.sh --clean --all`：**26/26 全绿、exit 0**；基线回填 core **4341 passed / 0 failed / 25 ignored**、webui **4927 passed / 0 failed（500 文件）**；`check-readme-claims` / `check-website-claims` / `check-doc-cli-drift` 三条 exit 0 |

> 反例纪律：T5 的「无数据 ⇒ 整块不渲染」是**产品判断**，不是样式选择 —— 渲染 `0%` 会把「未知」伪装成「很空」，与 `source_gaps` 同一类诚实要求。


---

## 交叉复核（2026-10-29 20:25，由另一会话执行）

**复验结论：文档声称的三组测试数字与实测完全一致**（不是引用，是重跑）：

| 声称 | 实测 |
|---|---|
| `context::` 103 passed | ✅ 103 passed / 0 failed |
| `agent::stream_loop` 18 passed | ✅ 18 passed |
| `tool_output` 10 passed | ✅ 10 passed |
| `cargo check -p marguerite-core` | ✅ Finished in 27.62s |
| `webui typecheck`（13 守卫 + i18n） | ✅ exit 0，六语言平价、CJK 基线未增长 |

交付物在位：`webui/src/components/chat/ChatContextMeter.vue`（5.5KB）、`context/spool.rs`、
`context/compact_state.rs`、`middle_truncate` → `tool_output_retrieve` 三处调用点、6 语言 `chat.contextMeter.*`。

### 复核发现的一个真实不完整（已修）
`sessions.fork` / `sessions.forkThroughTurn` **只有 Python 侧车实现**
（`pyengine/src/opensquilla/gateway/rpc_sessions.py:2294`），Rust `SUPPORTED_METHODS` 里没有。
前端 `forkTransition.ts` 无条件调用它 ⇒ **纯 Rust 部署**（侧车未装/降级，正是安装包常见形态）下
用户点「分支会话」会拿到一条 RPC 裸错误。
已修：`useChatForkController.forkConversation` 增加 `rpc.supportsMethod('sessions.fork')` 前置判定
+ 6 语言 `chat.toast.forkUnavailable`（说明需要侧车并给出替代操作）。

### 仍未完成（继承本计划 §实施状态的待办）
1. **T5 Step 5 真机像素自检**（未做）：需起隔离网关 + WS 探针断言 `sessions` 是数组、`windowStatus` 两种来源各出现一次。
2. **M1.5**：最近 N 个 `CompactState` 跨会话加载 / 开场 git 注入 / Prompt Scaffold 重建。
3. **M2**：Rust 侧 `sessions.fork` / `resume` / `replay` 仍未实现（前端已上线）。
4. **历史清理**（不成对 tool call / 孤儿输出）：本机可用会话库
   （`%LOCALAPPDATA%\Marguerite\sessions.db` 0 行、`artifacts/qa-gw-data/sessions.db` 4 行且 0 条 tool 行）
   **没有工具行样本** ⇒ 该假设**既未证伪也未证实**，缺证据前不动手。
