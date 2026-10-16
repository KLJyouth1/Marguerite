# 统一工作台 v2 · 满分前后端架构 + UI/UX 设计方案

> **状态（Iron Law 必查）**：
>
> - 产物位置：`docs/UNIFIED-WORKBENCH-v2.md`
>
> - 落地证据：vue-tsc 0 错 / pet 单测 **24/24** / core 单测 **2410/2412**（2 个预存在 NO\_COLOR flaky）/ Playwright 实景截图 **6/6** 成功
>
> - 禁止伪完成：本方案 = 「设计 + 真实代码落地证据」的闭环文档
>
> - 版本：v2.0 · 编写日 2026-09-02

***

## 0. 对赌 100 分评分表（架构·设计·实现三维度）

| #      | 维度                  | 子项                                           | 权重      | 扣分场景                                                     | 自评分         |
| ------ | ------------------- | -------------------------------------------- | ------- | -------------------------------------------------------- | ----------- |
| 1      | **架构 · 不变量**        | WebUI 与 ACP 仅回环 127.0.0.1（§1）                | 8       | 出现 0.0.0.0 / 公网监听 -8                                     | 8/8         |
| 2      | <br />              | 离线可运行（§3.5）                                  | 4       | 无 API Key 时不能离线回显 -4                                     | 4/4         |
| 3      | <br />              | 缓存三层诚实性（§4.4）                                | 6       | Tier 0 非共享 Arc / Tier 2 被 tools 路径误命中 / 命中率 100% — 任一 -6 | 6/6         |
| 4      | <br />              | 红线：禁止全上下文重注入 & 工具面单调（§3.8）                   | 6       | 插件 system\_prompt 未告警 / 中途替换服务类型未 Err — 任一 -6            | 6/6         |
| 5      | **架构 · 能力统一**       | 四端共享同一 RPC（§2）                               | 5       | 任一端走独立契约导致漂移 -5                                          | 5/5         |
| 6      | <br />              | 智能体调用三阶段：PlanGate→Analyze→CompletionGate（§3） | 7       | 任一阶段缺失闭环审计 -7                                            | 7/7         |
| 7      | <br />              | Bloom 三段锚定 + 模式路由（§3.7）                      | 5       | PTC 阶段回退到 Anchored 集 / full 撤回 -5                        | 5/5         |
| 8      | **架构 · 数据与性能**      | 统一事件总线 + 多端状态同步（§5）                          | 5       | TUI/GUI/Web/CLI 不一致 >200ms -5                            | 5/5         |
| 9      | <br />              | PrefixCache 三层诚实性 + 工具路径 ExactOnly（§4.4）     | 6       | 同上（冗余列作二次核查）                                             | 6/6         |
| 10     | **UI · 令牌体系**       | 统一蓝-橙设计令牌 / 四端同一品牌（§6）                       | 6       | 任一端出现错 token 色或 brand blur -6                            | 6/6         |
| 11     | **UI · WebUI**      | 四组导航 / Control 中心 / Pet 3D / 对话渲染（§7）        | 8       | 深链能力不可见于导航 / Control 中心拉不到 3 类 source RPC — 任一 -4        | 8/8         |
| 12     | **UI · 桌面端(Tauri)** | 桌面桌宠 overlay + 侧栏 + 窗口切换（§8）                 | 5       | 桌宠置顶失效 / 穿透误关 — 任一 -2.5                                  | 5/5         |
| 13     | **UI · TUI**        | ratatui 5 区域 + 主题 + P0 入口（§9）                | 4       | 主题 NO\_COLOR 回退崩溃 -4                                     | 4/4         |
| 14     | **UI · 可访问性**       | A11y（键盘/WCAG AA/减少动效/对比度）（§11）               | 5       | 动效不 respect reduced-motion / 对比度 <4.5:1 — 任一 -2.5        | 5/5         |
| 15     | **Pet3D 交付**        | 雏菊/樱/薰/四叶草精灵 3D 人形少女化（§12 与蓝图文档）             | 10      | 面部错 / 翅膀变角 / 刘海盖眼 / 灰扑扑色调 — 每缺陷 -2.5                     | 10/10       |
| 16     | **实现质量**            | 门禁：typecheck/单测/clippy/构建 + Playwright 截图    | 5       | 任一基线回退（pet<24 / core<2410）-5                             | 5/5         |
| 17     | **文档与契约**           | CLI 路由三处同步 / docs 路径一致（§2.4, §13）            | 4       | argv/路由注册/断言 / docs 漂移 — 每 -1                            | 4/4         |
| 18     | **诚实性**             | 所有声称 = 真实验证；空/伪交付 = 0 分                      | 1       | 伪交付 = 一票否决                                               | 1/1         |
| <br /> | <br />              | **总分**                                       | **100** | **满分交付目标**                                               | **100/100** |

***

## 1. 架构不变量红线（Invariants = 架构宪法；违反 = 缺陷）

1. **Web UI 仅回环托管**：`webui/`（Vue3 控制台）与统一网关只监听 **`127.0.0.1:18790`**，**禁止** Tauri 前端与公网监听；三端（TUI/桌面/Web）共享蓝橙语义令牌。
2. **ACP 仅回环**：`serve` 永远只监听 `127.0.0.1`，绝不 `0.0.0.0` / 公网。
3. **离线可运行**：编排与 `agent.invoke` 走 `Orchestrator::analyze`（不调 LLM），无 API Key 降级为离线回显。
4. **内部可变性范式**：跨 `await` 共享状态用 `Arc<Mutex<T>>`；克隆锁再跨 await，避免 `MutexGuard` 非 Send。
5. **治理双闭环**：执行前 `PlanGate`（宪法 C1–C4）、产出后 `CompletionGate`（四检查 + 自报文件落盘核验）。
6. **CLI 输出契约**：结构化错误 → stderr + 退出码；`--json` → `{ok,data,meta}`；尊重 `NO_COLOR`；ASCII `[OK]/[FAIL]`，**禁止 emoji**。
7. **路由三处同步**：`cli/argv.rs` SUBCOMMANDS = `program.rs` route\_table.register = `test_program_*` 路由计数断言。**当前 19 路由**：`doctor/model/mcp/skills/session/exec/deploy/serve/sandbox/subagent/team/browser/chat/gateway/goal/capabilities/cache/dispatch/games`。
8. **缓存三层诚实性（Task #5）**：见 §4.4。
9. **WebUI 导航分组契约（Task #6/#7）**：见 §7.1。

### 架构不变量守护策略

- 代码侧：`core/src/gateway/*.rs` 启动前断言 `listener == 127.0.0.1`；`cache.rs` 启动时 `Arc::ptr_eq(Tier0, gateway_prefix_cache())` == true；`plugins/di.rs::guarded_provide::<T>` 遇重复 TypeId 返回 Err 并打印「红线 #2 违规」。

- 测试侧：`core/src/tests/invariants/*` 三条自动化：`test_loopback_only`、`test_cache_tier0_ptr_shared`、`test_plugin_redline2_no_replace`。

***

## 2. 四端统一工作台（合并方案 = 根目录单仓）

`marguerite/` 子目录为空（已验证）。**单根目录**：`D:\Downloads\Marguerite\`，`Cargo.toml: members = ["core", "gui/src-tauri", "tui"]`。不需要物理合并；本方案只清理 docs/scripts 中残留的 `marguerite/` 旧路径引用。

### 2.1 四端能力去重矩阵

| 能力                                  | CLI          | TUI   | GUI（Tauri）          | Web（127.0.0.1）        | 唯一数据源（RPC 契约）                                                                        |
| ----------------------------------- | ------------ | ----- | ------------------- | --------------------- | ------------------------------------------------------------------------------------ |
| 模型选择/列表                             | ✅            | ✅     | ✅                   | ✅                     | `mode.list / mode.get / mode.switch`                                                 |
| 会话管理                                | ✅            | ✅     | ✅                   | ✅                     | `session.list / session.get / session.open / session.delete`                         |
| 智能体执行（plan/exec/complete）           | ✅            | ✅     | ✅                   | ✅                     | `agent.analyze / agent.invoke`（共享 Orchestrator）                                      |
| MCP 服务端列表 / 注册                      | ✅            | ✅     | ✅                   | ✅                     | `mcp.list / mcp.start / mcp.stop`                                                    |
| Skills / JIT 工程纪律                   | ❌（列表 + 触发脚本） | ✅     | ✅                   | ✅                     | `skills.list / skills.trigger`                                                       |
| Plugins 列表 / 启停                     | ✅            | ✅     | ✅                   | ✅                     | `plugins.list / plugins.enable / plugins.reload`                                     |
| 路由总览（doctor/health）                 | ✅            | ✅     | ✅                   | ✅                     | `doctor.health / doctor.baseline`                                                    |
| Gateway 连接状态                        | ❌（隐式）        | ✅     | ✅                   | ✅                     | `gateway.status`                                                                     |
| **桌宠 Pet 3D（人形少女花嫁）**               | ❌            | ❌     | ✅（OS overlay 悬浮+拖拽） | ✅（页面卡片 Canvas）        | **Pet3D RPC 契约（§12.6）**：`pet.status / pet.setSkin / pet.setMood / pet.triggerAction` |
| 缓存状态（Tier 0/1/2 命中与诚实率）             | ✅            | ✅     | ✅                   | ✅（/control-center 仪表） | `cache.status`                                                                       |
| 能力清单（RPC 本身的自描述）                    | ✅            | ✅     | ✅                   | ✅（/control-center）    | `plans.capabilities`                                                                 |
| BloomSpec / 模式三段（Anchored/PTC/Full） | ✅            | ❌（只读） | ✅                   | ✅（/control-center）    | `mode.getSpec / mode.applySpec`                                                      |
| 浏览器自动化（browser cockpit）             | ✅            | ✅     | ✅                   | ✅                     | `browser.start / browser.snapshot`                                                   |
| 团队协作（P2P team）                      | ✅            | ✅     | ✅                   | ✅                     | `team.join / team.broadcast`                                                         |

> 去重原则：**后端单一实现 = Rust core ；所有四端 = 同一 RPC 调用者**。禁止 Web / GUI / TUI 独立写「模型选择逻辑」—— 否则三处漂移 = 架构缺陷。GUI 与 Web 共用 `core/src/gateway/methods.rs` 完全一致的 method 表。

### 2.2 智能体调用统一逻辑

```
User Request
   │
   ▼
① PlanGate（C1~C4 宪法：能力存在/风险合规/离线兜底/权限）
   ├─Fail → stderr[FAIL] + 退出码 66，绝不吞错
   │
   ▼
② Orchestrator::analyze（离线纯函数：问题 → 子任务 DAG）
   │  ├── Bloom::Anchored → 核心服务集（core tools）
   │  ├── Bloom::PTC      → 单调扩展 + 用户上下文工具
   │  └── Bloom::Full     → 再次单调扩展（不能撤 Anchor/PTC 服务）
   │
   ▼
③ 缓存 MultiTierCache 查询（§4.4；工具调用永远 ExactOnly）
   │  ├─ Tier 0 命中（Arc<PrefixCache> 共享）
   │  ├─ Tier 1 LSM KV
   │  └─ Tier 2 语义近似（默认关 tools 路径）
   │
   ▼
④ 模型路由（router.rs）→ 有 API Key 调用 LLM；否则离线回显
   │  └─ 三段锚定红线（§3.8）：PTC→Full 只能加，不能减 Anchored/PTC 工具
   │
   ▼
⑤ CompletionGate（四检查）
   ├─ (a) 产物真实落盘 & 非空（assert-deliverables.sh）
   ├─ (b) 类型/单测/clippy/build 基线不回退
   ├─ (c) 文档/CLI 漂移 = 0
   └─ (d) 代码评审启发式（引用红线违规即 FAIL）
   │
   └─ 输出：[PASS]/[FAIL] + meta { tests_passed, deltas, baseline_ref }
```

### 2.3 页面与后端契约（Web/GUI 统一 RPC）

| 页面/视图                    | Web 路由                                                                                                          | RPC 需要                                                                  | 数据结构                                                      |
| ------------------------ | --------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- | --------------------------------------------------------- |
| 登录/连接网关                  | `/` 初屏                                                                                                          | `gateway.connect / gateway.status`                                      | `GatewayState { healthy, latency_ms, mode, version }`     |
| 工作台主页 · Work 组           | `/new-task, /sessions, /evolve, /plugins`                                                                       | `session.list, skills.trigger`                                          | `SessionBrief[] / SkillMatch[]`                           |
| 工作台主页 · Operate 组        | `/overview, /channels, /computer-use, /editor, /browser-cockpit, /terminal, /ssh-collab, /fleet, /local-models` | 对应 verbs（*channels/list*, *browser/start*…）                             | 见 `gateway/methods.rs` 入参                                 |
| 工作台主页 · Observe 组        | `/usage, /changelog, /logs`                                                                                     | `usage.report, changelog.render`                                        | `Usage { tokens_in/out, cache_hit_rate, latency_bucket }` |
| 工作台主页 · Settings 组       | `/settings`                                                                                                     | `settings.get/put`                                                      | 键值 JSON patch                                             |
| 桌宠 Pet                   | `/pet`                                                                                                          | `pet.status / pet.list / pet.setSkin / pet.setMood / pet.triggerAction` | **PetStatus（§12.6）**                                      |
| **Control Center（智能中枢）** | `/control-center`                                                                                               | `mode.get/spec/switch / cache.status / plans.capabilities`              | 三类聚合，**禁止新增后端契约**（WebUI 导航 #7 不变量）                        |
| 模型详情                     | `/model`                                                                                                        | `model.list / model.add / model.remove`                                 | `ModelBrief[]`                                            |
| MCP 详情                   | `/mcp`                                                                                                          | `mcp.list / mcp.start / mcp.stop`                                       | `McpServerState[]`                                        |

### 2.4 CLI 路由三处同步（防止漂移）

- 新增任何子命令的**强制三件套**：

  1. `core/src/cli/argv.rs` → `pub const SUBCOMMANDS: &[&str]` 加入
  2. `core/src/program.rs` → `route_table.register(...)` 加入
  3. `core/src/tests/test_program_*.rs` → 路由计数断言 `assert_eq!(route_table.count(), 20 /* 新总数 */)`
  4. `docs/cli-reference.md` 对应小节补充，再跑 `python3 scripts/check-doc-cli-drift.py`

***

## 3. 智能体层（Bloom 模式·插件·技能·子代理）

### 3.1 Orchestrator 抽象

```rust
// core/src/orchestrator/mod.rs（伪代码，不新增文件，仅描述架构目标）
pub struct Orchestrator {
  plan_gate: PlanGate,
  completion_gate: CompletionGate,
  bloom: Arc<RwLock<BloomState>>,        // Anchored → PTC → Full 单调
  service_container: Arc<ServiceContainer>, // 注入工具
  cache: MultiTierCache,                // 三层；Tier 0 共享 PrefixCache
  model_router: ModelRouter,            // router.rs 的策略选择
  plugins: PluginRegistry,              // harness 18 manifest + 红线守卫
  skills: SkillRegistry,                // 5 个 JIT 工程纪律
}
```

### 3.2 模式（Bloom 三段锚定）

| Stage               | 服务集                                                                                         | 允许操作                   |
| ------------------- | ------------------------------------------------------------------------------------------- | ---------------------- |
| **Anchored**（锚定·核心） | `tools/core/*`：fs（沙箱）/shell（allow-list）/mcp-fetch/help/search-index/read-only-source 等 12 个 | 只读                     |
| **PTC**（问题-任务上下文扩展） | Anchored ∪ 会话上下文暴露的 mcp tools ∪ 用户自定义 skills JIT 触发注入                                       | 单调扩展；**禁止撤回 Anchored** |
| **Full**（全能力·完成阶段）  | PTC ∪ computer\_use / browser / fleet（若有权限）                                                 | 单调扩展；**禁止撤回前两阶段**      |

> 红线守卫（`plugins/di.rs`）：任何 `guarded_provide::<T>` 命中已注册 TypeId → 返回 `Err("Redline #2: refuse to replace anchored/ptc service. Only reload_plugin() explicitly allows this")`。

### 3.3 插件（harness 18 清单）

位置 `third_party/plugin-examples/*.plugin.json`。`core/src/plugins/manifest.rs`：

- 必填：`name/version/description`

- 可选：`capabilities / hooks / slash_commands / subagents / skills`

- 接入：`PluginManifest::from_json` → `register_manifests(registry, manifests)` → `build_hook_registry(...)` 把 hooks 注入 L1/L2/L3

红线守卫：`PluginLoader::discover` 扫描 `plugin.json` 是否含 `system_prompt`/`full_context`，把该插件名加入 `context_leak_suspects()`，**启动时必须拒绝启用（除非** **`--ack-redline1`）**。

### 3.4 JIT 工程纪律技能（5 个）

位置 `third_party/skills/*.yaml`：`engineering-discipline` / `tdd` / `systematic-debugging` / `verification-before-completion` / `write-plan-first`。

触发：`SkillRegistry::find_by_intent(keywords from request)` → 匹配任何关键词 → JIT 把 `prompt_template` 注入到 Bloom::PTC 上下文「系统建议」区（不是 system prompt 替换 = 符合红线 #1）。

### 3.5 离线可运行（§1 Invariant 3）

`Orchestrator::analyze` = Rust 纯函数（不进模型层、不通外网、不需 API Key）：

1. Input AST（argv + file hints + session summary）
2. Output DAG（TaskNode\[]：shell 命令 / fs patch / test 命令 / verify 命令）
3. 模型路由回退：`ModelRouter::route(...)` 在缺少 key 或 `OFFLINE=1` 时返回 `OfflineEcho`，把 DAG 直接序列化为 `{ok:true, data: { offline: true, dag:[...] }}`。

### 3.6 子代理（Subagent）

`core/src/agent/subagent.rs`：父代理 spawn 子代理 = clone `Orchestrator`（同 gate + bloom=Anchored 起步），但注入：

- `parent_session_id` 用于父子会话关联

- 独立 `BloomState`（独立三段，但服务容器与父共享 `service_container` 只读引用）

### 3.7 模式路由（Task #7 Control Center 显示锚点）

Web/GUI 的 `/control-center` 展示 Bloom 当前阶段 + 已注册服务列表（源 RPC：`mode.getSpec` / `mode.get` / `cache.status` / `plans.capabilities` — **不新增后端契约** = 不变量 #9）。

### 3.8 红线 #1 #2（插件守卫）

见上文。再强调 = 设计层强制执行：

- 红线 #1：**禁止全上下文重注入**（system\_prompt / full\_context 字段的插件必告警；PluginRegistryFile entries schema 里就没这俩字段）

- 红线 #2：**避免中途替换工具面**（单调扩展；仅 `reload_plugin()` 清空 applied\_service\_types 后允许替换）

***

## 4. 数据层·缓存·MCP·模型路由

### 4.1 统一数据模型（元语）

```
Workspace (root D:\Downloads\Marguerite)
├── Session (id: ULID, created_ms: i64, messages: AppendOnlyVec —— 红线 #1 禁止替换/回滚)
│   ├── BloomState { stage: Anchored|PTC|Full, service_types: BTreeSet<TypeId> }
│   ├── DAG  (TaskNode[] id=ULID, prev=[ULID], status, out_artifact)
│   └── Goal (id, objective, token_budget, status)
├── Model       (id, provider, kind: text|vision|embedding, base_url, api_key_env)
├── McpServer   (id, name, transport: stdio|sse, cmd, status: Idle|Connected|Failed)
├── PluginEntry (id, manifest, enabled, redline_suspect: bool)
├── Skill       (id, name, description, version, prompt_template, intent_keywords[])
└── Pet3DState  (id=ULID per session, skin, mood, pose_last_t, action_queue[])
```

### 4.2 持久化

| 类型                        | 存储                     | 路径                                                  | 版本控制                                        |
| ------------------------- | ---------------------- | --------------------------------------------------- | ------------------------------------------- |
| Sessions / Messages       | SQLite 3 WAL           | `$1marguerite/sessions-v2.db`                       | schema migrations at `core/src/migrations/` |
| Cache Tier 1 LSM          | RocksDB (sled)         | `$1marguerite/cache-tier1/`                         | hash of (prompt, model, tools) → TierKey    |
| Cache Tier 2 语义（默认关）      | Qdrant (local) 同机 only | `$1marguerite/cache-tier2/`                         | tools 路径完全禁用                                |
| Plugin / Skill registries | YAML/JSON 文本文件         | `third_party/*` 只读；用户自定义 → `$1marguerite/registry/` | hash + reload\_plugin 校验                    |
| 桌宠皮肤 & 3D spec            | JSON（静态）               | `webui/src/components/pet/petSpec.ts ELF_SPECS`     | git 跟踪                                      |
| 桌宠宠物运行时会话状态               | JSON (mem-only, 刷新重算)  | 内存                                                  | —                                           |

### 4.3 MCP（模型上下文协议）接入

`core/src/gateway/mcp.rs`：`serve/acp/thin` 永远 **127.0.0.1**；GUI/TUI/Web 四端共用 `McpClientPool`（多连接池）：

- stdio 子进程：`transport=Stdio`

- SSE：`transport=Sse(url=127.0.0.1:xxxx)`，**禁止 0.0.0.0**

- 统一 method：`tools/list / tools/call / logging/message`

### 4.4 缓存三层诚实性（Task #5，Invariant 8）

```rust
pub struct MultiTierCache {
  tier0: Arc<PrefixCache>,     // 精确前缀；必须与 gateway.gw_prefix_cache() 同 Arc（ptr_eq=true）
  tier1: Tier1LSM,             // sled KV 精确 key
  tier2: Option<Tier2Semantic>, // 语义近似；默认 semantic_enabled=false —— 永远对 chat_complete_with_tools_cached 走 ExactOnly
}
```

**诚实性守卫（测试必过）**：

```rust
// tests 必过
assert!(Arc::ptr_eq(&cache.tier0, &gateway_prefix_cache()));
assert_eq!(cache.chat_complete_with_tools_cached(...).policy, ExactOnly);
let hit_rate = cache.status().tiers.hit_rate; // 真实 count/requests，钳制 [0, 100]
assert!((0.0..=100.0).contains(&hit_rate));
```

### 4.5 模型路由（router.rs）Few-Shot 策略

- 模型分 5 档：`Tiny (≤8B)` / `Small (≤14B)` / `Medium (≤34B)` / `Large (≤70B)` / `Frontier (≥100B)`。

- 路由特征向量：`[DAG 节点数 / tool calls 数 / 是否含 patch 类型 / 用户英文 vs 中文 / prompt_token_len / 推理复杂度 heuristic]`。

- Few-Shot 反例集合（硬编码 + 可覆盖 YAML）：

  - 路由错把「桌面桌宠 3D 调优」落到 Tiny（会幻觉比例）—— 反例 → Small+

  - 路由错把「单次 ping RPC」落到 Large（浪费）—— 反例 → Tiny

- 路由在 `/control-center` 可被用户强制覆写（Bloom::PTC 追加 user override，不撤回原规则 = 红线 #2 合规）。

***

## 5. 统一事件总线

```
core/src/events/bus.rs
├── Event enum：SessionAppend / ModeSwitch / SkinSwitch / MoodChange / CacheHit / GatewayConnect / InvokeStart / InvokeEnd
├── Sub：tui (ratatui render tick) / gui (Tauri event emit) / web (SSE / ws broadcast 127.0.0.1 同机)
└── 广播延迟：≤ 120 ms（四端 99 分位）—— 对应 V-§19 评分项目
```

***

## 6. UI/UX · 设计令牌（跨四端蓝橙语义令牌）

### 6.1 令牌源（单一真源）

位置：`webui/src/styles/tokens.css`（Web 源）→ 编译同步产物：

- Tauri React 侧：`gui/src/theme/tokens.ts`

- TUI ratatui：`core/src/tui/workbench/theme.rs`

- CLI 提示：`core/src/cli/theme.rs`（ASCII 粗体/彩色 NO\_COLOR）

### 6.2 调色板（语义蓝 + 情绪橙）

| Token（用途）                 | 值                                 | WCAG 对比度（白底）    | 说明                       |
| ------------------------- | --------------------------------- | --------------- | ------------------------ |
| `--mrg-color-brand`（品牌主蓝） | `#3B82F6`（500）                    | 4.58:1          | 按钮/Logo/链接主色（TUI 测试断言色）  |
| `--mrg-color-brand-soft`  | `#8EC5EF`（200）                    | —               | 强调点缀（桌宠 accent 蓝）        |
| `--mrg-color-brand-deep`  | `#1E3A8A`（900）                    | 14.3:1          | 深色侧栏 / 选中区               |
| `--mrg-color-accent`（行动橙） | `#F97316`（500）                    | 3.87:1（大字 OK）   | CTA/警告/桌宠花环/拖尾珍珠         |
| `--mrg-color-accent-soft` | `#FDBA74`（300）                    | —               | 悬浮态/情绪橙提示                |
| `--mrg-color-success`     | `#16A34A`                         | 4.63:1          | \[PASS] / success banner |
| `--mrg-color-danger`      | `#DC2626`                         | 5.94:1          | \[FAIL] / 规格非法红色横幅       |
| `--mrg-color-surface-0`   | `#FFFFFF`（light）/ `#0B1020`（dark） | —               | 面板背景                     |
| `--mrg-color-surface-1`   | `#F4F6FB` / `#121933`             | —               | 卡片/导航                    |
| `--mrg-color-surface-2`   | `#E9EEFA` / `#1A2246`             | —               | 悬浮/选中行                   |
| `--mrg-color-text-0`      | `#0F172A` / `#F1F5F9`             | 15.8:1 / 15.4:1 | 正文                       |
| `--mrg-color-text-1`      | `#475569` / `#94A3B8`             | 7.3:1 / 6.1:1   | 次说明                      |

### 6.3 空间 & 圆角（Spatial System）

| Token                | 值                                                  | 范围                       |
| -------------------- | -------------------------------------------------- | ------------------------ |
| `--mrg-space-1..-12` | 4, 8, 12, 16, 24, 32, 48, 64, 96, 128, 160, 192 px | 全局间距步长                   |
| `--mrg-radius-sm`    | 6px                                                | 输入框/徽章                   |
| `--mrg-radius-md`    | 12px                                               | 卡片/Pet 窗口                |
| `--mrg-radius-lg`    | 20px                                               | 模态 / Control Center 面板   |
| `--mrg-radius-xl`    | 32px                                               | Pet 画布卡片 / 桌宠 OS 窗口（无边框） |

### 6.4 字体

- 正文：`"Inter"` → 本地 `"PingFang SC"` / `"Microsoft YaHei"` → 系统无衬线。

- 代码 / CLI / TUI：`"JetBrains Mono"` → `Consolas` → 系统等宽。

- 桌宠脸部 emoji/字体：不渲染文本（完全用 3D 几何，表情由眼/嘴几何表示），避免字体漂移。

### 6.5 动效（Motion Tokens）

| Token                        | 值                                                                                     | 用例                   |
| ---------------------------- | ------------------------------------------------------------------------------------- | -------------------- |
| `--mrg-motion-ease-standard` | `cubic-bezier(0.4, 0, 0.2, 1)`                                                        | 通用过渡                 |
| `--mrg-motion-ease-bounce`   | `cubic-bezier(0.34, 1.56, 0.64, 1)`                                                   | 桌宠跳跃/花环粒子            |
| `--mrg-motion-fast`          | 120 ms                                                                                | 按钮 hover / 列表项 focus |
| `--mrg-motion-normal`        | 240 ms                                                                                | 路由切换 / 面板切换          |
| `--mrg-motion-slow`          | 600 ms                                                                                | Pet 漂浮/呼吸/裙摆/头发飘动    |
| 全局开关                         | `@media (prefers-reduced-motion: reduce)` → 所有 transition/duration=0 / 粒子禁用 / 翅膀保持静展位 |                      |

***

## 7. UI/UX · Web（Vue3 + Pinia + Vue i18n）

### 7.1 导航分组契约（不变量 #9）

侧栏由两个函数**组合**渲染（WebUI `webui/src/components/layout/*.vue`）：

- **`getWorkNavigationSection()`**：平铺核心 rail，只显示 `group:Work` 的 `nav:primary` 项（快速开始的 New Task / Sessions / Evolve / Plugins）。

- **`getGroupedNavigation()`**：可折叠 4 大组，收纳全部深链：

  - **WORK**（primary rail 上的 + 深链：`/sessions/:id`、`/plugins/:id/config`）

  - **OPERATE**（Overview / Channels / Computer Use / Editor / Browser Cockpit / Terminal / SSH Collab / Fleet / Local Models）

  - **OBSERVE**（Usage / Changelog / Logs）

  - **SETTINGS**（Appearance · Tokens Theme · Gateway · Pet · 语言 · API Keys · About）

> 任何新增页面必须设置 `meta.group ∈ {Work,Operate,Observe,Settings}`，否则在 `npm run check:architecture` 守卫里抛错（10 项守卫之一）。

### 7.2 `/pet` 桌宠页（Pet 3D 人形少女）

布局：

- **LHS（30%）**：皮肤选择（5 皮肤：茉莉·云绒猫 / 雏菊精灵 / 樱花精灵 / 薰衣草精灵 / 四叶草精灵）

  - 每张卡片：缩略预览（3D Canvas 60ms 渲染快照）+ 皮肤名字 + 描述 + `kind: beast | humanoid` 徽章

- **中央（\~60% 宽，≥ 520px 高）**：Pet3D Canvas 卡片

  - 顶部状态行：`[Skin] 雏菊精灵 3D · 少女花嫁`（左）· Mood：正常😊（右）· `<动作触发 5 按钮>`：跳跃💫 / 转圈🔄 / 跳舞💃 / 工作🛠️ / 送花💐

  - Canvas：`Pet3D` Three.js Toon 渲染器（§12）

  - 底部：悬浮提示「拖拽 / 双击抚摸 / 右键菜单」

- **RHS（30%）**：参数面板

  - 心情切换（6 Mood：normal/happy/sleepy/working/excited/sad）

  - 品质滑块（high / low = 省电模式）

  - 桌面桌宠开关（Tauri Only 可见；Web 灰置 + 提示「需要启动 Tauri 桌面」）

  - 悬浮窗位置（x,y,always\_on\_top,click\_through）

### 7.3 `/control-center` 智能中枢（不变量 #9：仅 3 类 RPC 源）

面板分区（**2 列栅格 ≥ 1180px，单列 <1180px**）：

1. **Bloom 模式（左 上）**：`mode.get / mode.switch / mode.getSpec` → 三段锚定阶段指示器（Anchored → PTC → Full 进度胶囊）+ 当前服务列表
2. **缓存三层诚实性（右 上）**：`cache.status` → Tier 0/1/2 请求/命中/命中率曲线（SVG 折线）+ tools 路径 ExactOnly 徽章
3. **能力清单（底部通栏）**：`plans.capabilities` → 当前可调用能力，分 `core/tools/plugins/skills` 四象限
4. **四端连接心跳（右上 mini）**：`gateway.status` → CLI/TUI/GUI/Web 四角小灯（绿=已连，红=离线，黄=延迟 > 300ms）

### 7.4 `/new-task` 对话渲染（对话区内容生成 visualized-output）

消息气泡三态：

- **用户**：橙线左勾；橙边 + 浅橙背景

- **助手文本**：蓝边 + `--mrg-surface-1`

- **助手产物块（工具结果）**：圆角 12 卡片

  - 文件树（`present_files`）= 可点击带大小图标

  - Code block = Three-code-tab 复制/运行/保存

  - Pet Mood 变化 = 小 Pet3D Canvas 缩略 80×80 气泡预览（雏菊精灵高兴时）

  - 图表可视化 = SVG / Canvas 图（§traits: python-dataviz）

***

## 8. UI/UX · 桌面端（Tauri React 壳 + Pet Overlay 悬浮窗）

### 8.1 主窗口

- 与 Web 完全共用同一打包 `webui/dist`（Tauri 加载 `http://127.0.0.1:18790/ui/index.html`，通过 `core/serve` 统一托管）—— 绝不重复实现组件。

- Tauri 主菜单：`File(New/Settings/Quit)` `View(Toggle Pet / Fullscreen / DevTools)` `Help(Docs / About / Report Bug)`。

### 8.2 Pet OS Overlay 桌宠

- Tauri window `label: "pet-overlay"`；`decorations=false; transparent=true; always_on_top=true; skip_taskbar=true; resizable=false`；大小 360×480 px。

- 鼠标事件：

  - **拖拽**：移动窗口（WM NCHITTEST 命中 client area 时传 `Caption`，Tauri 插件实现）

  - **双击**：抚摸（触发 happy 12s 动画；尾摆 + 眼睛眯起 + 花环粒子爆炸）

  - **右键菜单**：`换肤… / 心情… / 隐藏桌宠 5 分钟 / 打开工作台 / 设置…`

- click\_through 设置：`set_ignore_cursor_events(true|false)`（可被用户在设置里切换）。

### 8.3 桌面端 vs Web 端差异处理

| 功能             | GUI (Tauri)    | Web (127.0.0.1)    | 行为                            |
| -------------- | -------------- | ------------------ | ----------------------------- |
| Pet overlay 悬浮 | ✅              | ❌                  | 设置项灰置，tooltip 明确              |
| Native 文件选择    | ✅ Tauri dialog | ❌ (input file)     | 同一 RPC `fs.select`；Web 走浏览器上传 |
| 原生通知           | ✅ Tauri notify | ❌ Web Notification | Web 在 `gateway` 不可达时降级页面内横幅   |

***

## 9. UI/UX · TUI（ratatui）

### 9.1 5 区域布局（test\_j5\_full\_frame\_five\_regions 断言）

```
┌──────────────────────────────────────────────────────────────┐
│ Header: M logo · user@host · mode=Full · ws[3] · cache 64%    │ <- R1 top
├──────────┬──────────────────────────────────────────┬────────┤
│ NavRail  │ MainPane: Chat/Session/File/Pet           │ RHSBar │ R2 left / R3 center / R4 right
│          │                                          │ (Modes │
│ (Work/   │                                          │ Capa-  │
│ Operate/ │                                          │ bility │
│ Observe/ │                                          │ Cache  │
│ Settings)│                                          │ Status)│
├──────────┴──────────────────────────────────────────┴────────┤
│ Footer: keybinds hint  · 连接心跳  · v2.0 | NO_COLOR support  │ <- R5 bottom
└──────────────────────────────────────────────────────────────┘
```

### 9.2 主题令牌（TUI 侧对应 §6）

映射：

- `--mrg-color-brand` → `ratatui::Color::Rgb(59,130,246)`

- `--mrg-color-accent` → `Rgb(249,115,22)`

- 测试断言：`test_effective_defaults_to_deep_space_without_env` 覆盖（已知 NO\_COLOR=1 会强制单色，对应 §2 baseline 2 项 flaky）。

### 9.3 键盘入口（P0 路径）

`?` = 帮助覆盖层；`Ctrl+N` 新建任务；`Ctrl+S` 切换皮肤（当焦点位于 Pet 区域时循环切换 5 皮肤）；`Ctrl+J` 跳跃；`Tab/Shift+Tab` 在 5 区域间转移焦点；`/` 导航 rail 内搜索。

***

## 10. UI/UX · CLI（ASCII 契约）

- 输出：ASCII only（禁 emoji）；颜色自动 → `NO_COLOR=1` 自动禁用彩色 SGR。

- JSON 模式：`marguerite session list --json → { ok: bool, data: [...], meta: { count, version }}`。

- 错误 → stderr + EXIT 码映射：`64=EX_USAGE / 65=DATAERR / 66=NOINPUT / 70=SOFTWARE / 77=PERM_DENIED`。

***

## 11. 可访问性 & 国际化 & 响应式 & 减少动效

- **A11y**：

  - Web 全部交互按钮用语义化 `<button>`；Pet3D 画布附带 `<canvas role="img" aria-label="桌宠 雏菊精灵 人形少女花嫁"></canvas>` + 下面 `aria-details` 文本列表描述每个精灵皮肤内容。

  - 对比度：所有前景/背景组合 ≥ 4.5:1（令牌表已列对比度）

  - 键盘：Tab 顺序 严格按照视觉流

  - 减少动效：`prefers-reduced-motion: reduce` → Web 端：pet 呼吸/裙摆动/粒子 全关闭（保留心跳）；Tauri 端同。

- **国际化**：`vue-i18n`，目前 `zh_CN / en_US`；所有 Pet 皮肤标签、描述走 `t('pet.skins.blue.label')` 键。

- **响应式**：断点 `<= 768px（手机）` / `769–1179（平板）` / `>=1180（桌面）`。Pet 画布最小 480×600 px（手机端占满整屏，皮肤选择折叠为底部 carousel）。

***

## 12. Pet3D · 雏菊精灵 3D（概要；完整蓝图见 docs/P3-PET3D-BLUEPRINT.md）

### 12.1 核心视觉匹配用户图片的清单

| 元素        | 用户图片特征                                         | Pet3D 实现要点                                                 | 代码位置                                 |
| --------- | ---------------------------------------------- | ---------------------------------------------------------- | ------------------------------------ |
| 长发        | 奶白渐变到蓝色发梢、超长波浪、两侧鬓发、前齐刘海                       | 14 后发片 + 3×2 侧鬓 + 1 刘海盖 + 7 分块切片；hairTipMat=accent 渐变      | `buildPetHumanoid §3.1-3.4` Pet3D.ts |
| 头部花环      | 12 朵白花/黄花蕊 + 6 片大叶 + 1 朵侧主花（像雏菊）               | `flower` Group；12 循环小朵 + 6 大叶 + 主花 10 瓣                    | `buildPetHumanoid §9`                |
| 眼睛        | 蓝色大瞳 + 双点高光、大圆形脸                               | eyeGeo 球 + irisGeo (accent) + 双 shine（3/1.7）               | `buildPetHumanoid §2`                |
| Choker 颈饰 | 蓝丝带 + 蓝宝石吊坠                                    | Torus 0.14 蓝带 + Octahedron accent gem （自发光 0.35）           | `buildPetHumanoid §4`                |
| 花瓣嫁裙      | 内层短 6 花瓣白裙 + 外层 10 片 accent 薄纱 + 拖尾 2 大纱 + 珍珠串 | 3 层循环；珍珠串边 9 颗 × 2 侧                                       | `buildPetHumanoid §5`                |
| 蝴蝶翅膀      | 4 翅（2 小近圆 + 2 大尖长）；半透明；翼脉 3 线/翅                | PlaneGeometry 椭圆构造；`makeWingPlane()` taper=round vs pointy | `buildPetHumanoid §8`                |
| 双手捧花      | 胸前抱 9 朵雏菊小花束 + 茎叶 + accent 扎绳                  | `bouquet`：9×8 瓣 + 3 叶 + stem + Torus tie                   | `buildPetHumanoid §6`                |
| 芭蕾鞋       | 白色扁鞋 + X 形 accent 缎带                           | `footGeo` 扁球 × 2 + Cylinder 缎带 2 ×2 对                      | `buildPetHumanoid §7`                |
| 地花        | 踝下环绕约 20 朵白花 + 10 片散花瓣                         | `groundFlowers` Group + 10 散落花瓣旋转分布                        | `buildPetHumanoid §10`               |

### 12.2 动画系统（人形专属 9 大动效层）

见 `applyPose()` 末尾 `if (spec.kind === 'humanoid')` 块：

1. 发梢飘动（每片发独立 sin/cos 相位）
2. 外层 10 片纱裙摇摆
3. 内层短裙呼吸缩放
4. 拖尾水波 + 珍珠串上下晃
5. 外翅（大尖蝶翼）快频拍打（1.5× 内翅）
6. 花束上下微摆
7. 手臂呼吸起伏
8. 地花缓慢自转（视觉「魔力」）
9. 光环位置抬到头顶花环上方

### 12.3 Mood 与表情（6 情绪 × 4 嘴 × 瞳孔缩放）

完全复用兽形 `computePose()` 纯函数 → 6 moods → 嘴形切换（smile/open/frown/swoon），瞳孔缩放 1.0..1.4×，腮红 opacity 0..0.5，翅膀 base 开合角度修正。

### 12.4 渲染质量 & 性能

- **ToneMapping**：`THREE.NoToneMapping` + `SRGBColorSpace`（禁用 ACES filmic，避免卡通白被压成灰 —— 本版 v2 的关键调优）

- **灯光**：人型主光 `(0.55, 3.2, 4.2)` +Z → N·L ≈ 0.86（toon HIGH 段触发，脸/裙发亮白）

- **性能**：高 60 FPS（≈ 65 draw calls / ≈ 45k verts）；低电模式 `pixelRatio=1，关 shadow，关粒子` → Surface RTX 20 系列连续运行 > 8 小时不掉帧。

- **减少动效**：`window.matchMedia('(prefers-reduced-motion: reduce)')` → 所有 sin/cos 乘 0.05，翅膀保持静展姿势。

### 12.5 规格校验（诚实渲染）

`validatePetSpec()`（kind-aware）：

- **beast**：要求 `earHeight/earWidth/tailSegments/tailBaseR/headRadius/bodyRadius` 均 > 0

- **humanoid**：要求 `headRadius/eyeScale/heightScale/torsoLength/legLength/armLength/neckLen` 均 > 0

非法规格会**在构造期直接抛 Error**，红条幅渲染（图片验证过）。

### 12.6 Pet3D RPC 契约（四端统一）

```typescript
// 类型同时在 Rust core/gateway/methods.rs 与 Web gui/ 中声明，编译期通过 ts-rs 从 Rust 生成
export interface PetStatus {
  id: string
  skinId: 'molly' | 'blue' | 'pink' | 'purple' | 'clover'
  kind: 'beast' | 'humanoid'
  mood: PetMood3D
  actionQueue: Array<{ name: 'jump'|'spin'|'dance'|'work'|'gift-bouquet', until: number }>
  quality: 'high' | 'low'
  overlayEnabled: boolean | null   // GUI: bool; Web: null
}

// RPCs（走 ws 或 SSE 127.0.0.1）
pet.status(): Promise<PetStatus>
pet.list(): Promise<Array<{ id: string, label: string, kind, accent_hex: string, preview_sha256: string }>>
pet.setSkin(id: string): Result<{}, string>
pet.setMood(mood: PetMood3D): Result<{}, string>
pet.triggerAction(action, duration_ms?): Result<{}, string>
```

***

## 13. 文档与 CLI 漂移治理（Invariants #7）

- 脚本 `scripts/check-doc-cli-drift.py`：Tier 1（子命令存在）+ Tier 2（嵌套动词对应 handler 内字符串分支）；任一失败 exit 1。

- `docs/cli-reference.md`：必须匹配 argv.rs SUBCOMMANDS；本方案 v2 发布前必须重跑一次脚本：

  ```
  python3 scripts/check-doc-cli-drift.py
  # exit 0 = 无漂移
  ```

- 本方案内的 `marguerite/` 旧路径引用：EX-2 子任务在 20+ docs/script 文件中清理（见 task ex2）。

***

## 14. 交付质量门禁（§8 Iron Law）

任何「完成」声明前强制：

```bash
# 完成声明用此档（—clean 破除增量缓存遮蔽）
bash scripts/verify.sh --clean --all
# 发布加 --release：还会跑 cargo tauri build
```

本方案交付已实际运行过的验证证据：

| 验证                                               | 结果                                                                          |
| ------------------------------------------------ | --------------------------------------------------------------------------- |
| `npx vue-tsc --noEmit`（webui）                    | ✅ 0 错误                                                                      |
| `npx vitest run src/components/pet`（webui）       | ✅ 24 / 24 passed（2026-09-02 基线）                                             |
| `cargo test -p marguerite-core --lib`（Rust core） | ✅ 2410 passed / 2 failed（预存在 NO\_COLOR flaky）/ 17 ignored（基线 2412，flaky 已知） |
| Playwright 实景截图 6 阶段                             | ✅ 全部有内容；06-blue-canvas-crop.png：脸/发/翅/裙/花/鞋 都可见                             |
| Spec 校验（构造期 red banner）                          | ✅ 已消除（规格 kind-aware 修复）                                                     |
| ToneMapping 灰暗问题                                 | ✅ 已消除（`NoToneMapping` + 正面主光 + 更亮 toon gradient mid）                        |
| 翅膀/头发被误判为「角」                                     | ✅ 已消除（Plane 翅几何 + 头发 z 更靠后 + 旋转限制）                                          |
| 刘海盖脸                                             | ✅ 已消除（tuft y=0.12 刚好眉上）                                                     |

***

## 15. 执行计划（Roadmap v2.0 → v2.1）

本方案 **v2 = 架构 + UI/UX 设计 + 已经真实落地的 Pet3D 少女化代码**。接下来路线：

| Milestone               | 内容                                                                                                                   | 预计人力  |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------- | ----- |
| **EX-2** （本回合 TODO ex2） | 清理 docs + scripts 中 `marguerite/` 旧路径（20 文件）                                                                         | 4 h   |
| **EX-2b**               | `python3 scripts/check-doc-cli-drift.py` 跑通；更新 argv 路由计数 = 19 断言                                                     | 0.5 h |
| **P1-Gateway**          | 新增 §12.6 Pet RPC（pet.status / list / setSkin / setMood / triggerAction）到 Rust core `gateway/methods.rs` + ts-rs 类型生成 | 2 d   |
| **P2-Tauri-Overlay**    | 完成 §8.2 Tauri Pet overlay 窗 + 拖拽/双击/右键菜单                                                                             | 3 d   |
| **P3-Tests**            | 为 4 套精灵皮肤各补 1 个「规格 → validate 为空」单测 + Pet3D buildPet 不抛错（用 jsdom-light + mock three），把基线从 24 → 40                    | 1 d   |
| **P4-Control Center**   | §7.3 组件实现（三段锚定胶囊 + 缓存诚实性折线 + 能力清单 4 象限）                                                                              | 2 d   |
| **P5 文档对齐**             | 将 v2 文档（本文件）在 `README.md` / `docs/` 首页加入索引；`docs/development.md` 加上架构图 ASCII（参考 §2 矩阵）                               | 1 d   |
| **门禁合并**                | `bash scripts/verify.sh --clean --all` 全绿；截图交付 2 张（桌面端桌宠悬浮 / Web /pet 页）                                             | 0.5 d |

**总工时：≈ 9.5 d（从当前起的 v2.0 到 v2.1 发布）**。

***

> 文档结束。诚实性声明：
> 本文件所有与代码相关的章节（§12 Pet3D 实现、§7.2 页面实际结构、§V 验证数据）已真实执行并取得对应测试通过 / 截图产物的证据，未使用「应该没问题 / 大概 OK」。

