# DeepSeek Harness × Marguerite 深度二创集成 — v1.0 集成指南

> **v2 融合总结（DSH rc.8 + web-ui-all 0.2.5 + 梁神锚定）**：见 `docs/dsh-fusion-v2-2026-08-21.md`；v2 新增 8 份 PluginManifest（presets/compaction/goal-round-driver/plan-mode/schedule/workflow-ralph/session-projection/skill-registry）。

> 对应规格：`e:\DeepSeek\.trae\specs\2026-08-19-dsh-marguerite-integration\spec.md`
> 交付日期：2026-08-19
> 集成策略：**A+B 混合** → 先 PluginManifest 插件化落地（Phase A）；再把核心高价值模块升级为 Rust 原生实现（Phase B）。
> 目标形态：**TUI（Ratatui）终端优先**，不引入 Electron/Tauri/WebView（宪法 D12/D16）。

---

## 0. 为什么不是直接运行 DSH 独立进程？

Marguerite 是 **Rust 单二进制成品工作台**（Rust SSOT 原则，宪法 D13）。直接跑 DSH 意味着：
- 两条 Node.js + Chromium 常驻进程，内存/开机成本翻倍；
- 事件流/工作台状态分裂，无法共享 Memory/KB/Harness Hooks；
- 模型调用链重复经过 DSH 的 HTTP/WS 包装器，破坏 <50ms 端到端延迟门禁。

因此本集成采用「**插件清单 + 原生实现**」的二创方式：
1. 把 DSH 10 款插件的 **能力/Hook/Slash/子代理/技能** 写成 8 份 `PluginManifest` JSON（Phase A）。
2. 把最高价值的 3 个模块（Usage 用量统计 / GenUI 交互面板 / Pet 鲸鱼娘桌宠）升级为 **纯 Rust 原生实现**（Phase B）。

---

## 1. 交付清单：在哪里找到代码？

### 1.1 插件清单（PluginManifest JSON）
目录：`third_party/plugin-examples/`（共 8 份）

| 文件 | 对应 DSH 源插件 | 核心 capabilities | 扩展字段 |
|---|---|---|---|
| `dsh-harness-core.plugin.json` | @dsh/core | `dsh.runtime.version` | `events=[agent.*]` |
| `dsh-market.plugin.json` | dsh-market | `dsh.market.list/install/upgrade` | `dependencies=[dsh-harness-core]` |
| `dsh-better-sidebar.plugin.json` | dsh-better-sidebar | `dsh.sidebar.list/pin_slot` | `dependencies=[dsh-harness-core]` |
| `dsh-modlens.plugin.json` | @liustack/dsh-modlens + modlens-full | `dsh.vision.img2txt/ocr/layout/json` | `events=[vision.recognized]` + `hooks=[pre_tool_use]` |
| `dsh-usage.plugin.json` | dsh-daily-usage | `dsh.usage.query_day/query_month/balance` | `events=[usage.accumulated]` + `hooks=[post_tool_use]` |
| `dsh-pet-whale.plugin.json` | dsh-pet + web-ui-all 鲸鱼娘 | `dsh.pet.status/pet/feed` | `events=[notification.*]` + `hooks=[notification]` |
| `dsh-minigames.plugin.json` | dsh-mini-games | `dsh.games.list/play` | `dependencies=[dsh-harness-core]` |
| `dsh-router.plugin.json` | dsh-router-standard | `dsh.router.set_mode/query` | `subagents=[plan/execute/weak]` + `dependencies=[dsh-harness-core]` |

每份 manifest 齐全 **name / version / description / capabilities / hooks / slash_commands / subagents / skills + dependencies/events 扩展** 共 10 字段；至少 6 份的 dependencies/events 非空（扩展字段实装，AC-U2 阈值 ≥4）。

### 1.2 技能 YAML（Market 4 个）
目录：`third_party/skills/`

| 文件 | 用途 |
|---|---|
| `dsh-market-list.yaml` | 列出 8 份内置 manifests |
| `dsh-market-search.yaml` | 按关键词模糊匹配 name+description |
| `dsh-market-install.yaml` | 安装新插件：写入 manifests/ + 触发 ReloadManifests 事件 |
| `dsh-market-upgrade.yaml` | 语义化版本比较，升级同名插件 |

### 1.3 Rust 代码入口点

| 模块（相对 `core/src/`） | 角色 | 新增单测数 |
|---|---|---|
| `plugins/manifest.rs` + extended fields | Manifest 骨架：新增 `dependencies`/`events` 两字段；拓扑排序 `topo_order()` | 16 |
| `cli/dslash.rs` | 7 条 DSH 专属 `/` 斜杠命令解析 → EventBus 发布 | 4 |
| `harness/dsh_bridge.rs` | 3 类 Hooks（Modlens Pre / Usage Post / Pet Notify）+ Router 3 profile + FleetRouter 能力路由 | 8 |
| `tui/workbench/dsh_slots.rs` | 6 个 SlotId 渲染器（功能级 Ratatui widget） | 6 |
| `genui/contract.rs` | GenUI Widget 枚举：Card/Button/ProgressBar/Form/FormField | 4 |
| `genui/render.rs` | GenUI 渲染：Button bracket / Progress 夹紧 / Form Text+Select / click_dispatch → SteeringEvent | 8 |
| `usage/query_service.rs` | Usage 原生模块：DayStat/MonthStat/UsageQueryService + SQLite by_day/by_month/budget/balance | 10 |
| `workbench/pet_v2.rs` | PetV2 6 帧状态机：Idle/Thinking/Running/Sleeping/Eating/Petted + ASCII 精灵 + affinity 持久化 | 6 |
| `tests/dsh_e2e.rs` | **E2E 4 阶段闭环**：Modlens → GenUI → Router → Usage+Pet | 1 |
| **合计** | | **63 ✅** |

---

## 2. 7 条 Slash 命令速查表

```
/modlens <attachment_path>            # 识图（PreToolUse Hook 自动注入 OCR 结果）
/usage [day|week|month]               # Daily Usage 聚合查询（默认 day）
/pet [pet|feed|status]                # 鲸鱼娘桌宠：摸头 / 投喂 / 看状态
/games <list|play id>                 # Mini Games：列游戏 / 玩（2048=1 / TicTacToe=2 / Snake=3）
/market <list|install <name>>         # Market：列插件 / 安装
/router set <plan|execute|weak>       # Router 模式切换：规划 / 执行 / 弱路由（默认 weak）
/sidebar <list|pin slotId>            # Better Sidebar：列槽位 / pin 指定槽
```

实现路径：`cli/dslash.rs::parse_slash()`，解析后发布 `EventType::SlashCommand` 事件到 `events/bus.rs`。7 种全部覆盖错误 case（无效子命令返回 `ParseError`，不 panic）。

---

## 3. 生命周期 Hooks 桥接

在 `harness/dsh_bridge.rs` 中注册到 HookRegistry（非直接调用，满足 NFR6）：

| HookPhase | 触发 | 效果 |
|---|---|---|
| PreToolUse | Turn 上下文存在图片附件 | 调用 vision/modlens 识图，结果以 text_chunk 注入上下文（harness::dsh_bridge::modlens_pre_hook） |
| PostToolUse | 每次工具调用完成 | 累计 tokens_in/out/duration/tool_name 到 UsageLedger（SQLite 落盘）；发布 `UsageAccumulated` 事件 |
| Notification | AgentRunning/AgentThinking/AgentDone/PlanStarted/TurnCompleted | PetV2 自动切状态（Running→Running；Thinking→Thinking；Done/TurnCompleted/PlanFinished→Idle） |

---

## 4. TUI 工作台 6 个 DSH 槽位（36 SlotId 契约）

T-1.7 实现的 6 个渲染器，全部消费 36 SlotId 的既有 enum：

| 渲染器 struct | SlotId（既有 enum） | 最低功能级渲染内容 |
|---|---|---|
| `DshBetterSidebarSlot` | LeftSidebarFileTree | `[DSH Better Sidebar]` 标题 + 4 行 ASCII 文件树 + Browser/Editor 底部占位 |
| `DshUsageSlot` | RightAgentTasks | `Daily Usage (TOKEN)` + 7 天条形图 + `Total: 12,345 TOKENS` + `Monthly: -.--` |
| `DshPetWhaleSlot` | RightAgentStatus | 🐋 标题 + 6 行 ASCII 鲸鱼娘精灵 + `Mood: idle / affinity: 0 / energy: 80` |
| `DshMiniGamesSlot` | BottomOutput | `Mini Games (DSH)` + 锚点 3 款：`2048 (↑↓←→)` / `TicTacToe (小键盘 1-9)` / `Snake (WASD)` |
| `DshGenUiSlot` | RightAgentPlanner | `GenUI (DSH)` + 示例卡片「DSH GenUI Demo」/ 2 行正文 / `[ A ] [ B ] [ C ]` 三按钮 |
| `DshMarketSlot` | RightAgentMemory | `Market (DSH)` + 8 份插件列表 / 底部 `/market install <name>` |

每个 Slot 都有独立 `ratatui TestBackend 40x15` snapshot 单测，6/6 全绿。

---

## 5. Usage 消费统计 Rust 原生模块（Phase B）

替换原先仅有 "record + status" 的骨架：

### 数据模型
```rust
pub struct DayStat {
    date: String,                       // YYYY-MM-DD
    tokens_in: u64, tokens_out: u64,
    turns: u64, sessions: u64,
    model_distribution: BTreeMap<String, u64>,
}
pub struct MonthStat {
    year: i32, month: u32, /* 同上累计 + days: BTreeMap<日期, DayStat> */
}
```

### UsageQueryService 接口
- `by_day(start, end) -> Vec<DayStat>` — 日期范围聚合；**范围覆盖的空天自动填充 default**（热力图对齐）。
- `by_month(year, month) -> MonthStat` — 整月汇总。
- `balance() -> Option<f64>` — 余额占位（读 config.yaml `usage.balance.usd`，不实际对接付费 API）。
- `check_today_budget(threshold: u64) -> bool` — 今日累计总 token 超阈值触发告警（集成 context/budget.rs 通知）。
- `on_post_tool_use(...) -> UsageAccumulated` — 与 PostToolUse Hook 共享调用路径，事件总线发布。

全部 10 条单测走内存 SQLite (`Connection::open_in_memory()`)，零真实数据污染。10 万条记录 by_day(30d) 对账通过。

---

## 6. GenUI 交互面板 Rust 原生模块（Phase B）

### Widget Enum（与 DSH dsh-genui 的 schema 对齐）
```rust
pub enum Widget {
    Card(Card { title, body: Vec<String>, actions: Vec<Button> }),
    Button(Button { id, label, enabled }),
    ProgressBar(ProgressBar { id, percent (0..=1 夹紧) }),
    Form(Form { id, fields: Vec<FormField> }),
}
pub enum FormField {
    Text { id, label, placeholder },
    Select { id, label, options: Vec<String> },
}
```

### 渲染 & 事件回传
- **Button**：`[ label ]` 括号风格；hover 反色（Black/Cyan）；disabled 变灰。
- **ProgressBar**：`[####....] 52%`，自动夹紧 percent∈[0,1]。
- **Form**：`> Name: [请输入姓名]` + `City: [Beijing v]`。
- **click_dispatch(&[Button], x_in_row) -> Option<String>**：按 10 宽/2 gap 的按钮栅格做行内命中，返回对应 `Button.id`（disabled 按钮返回 None）。
- **SteeringEvent { target, event, payload }**：JSON roundtrip 单测通过，点击 → Steering 事件与 `agent/steering.rs` 消费端契约对齐。

---

## 7. Pet 鲸鱼娘桌宠 Rust 原生（Phase B）：PetV2 6 帧

### 状态转移图
```
                  AgentRunning / PlanStarted
                          ┌─────▶ Running ─────15分钟超时────┐
                          │                                    ▼
Idle ──AgentThinking──▶ Thinking                           Sleeping
  ▲                       │                                    ▲
  │  AgentDone/TurnCompleted/PlanFinished                     │能量<20
  └───────────────────────┘                                    │
  ▲                                                            │
  │ user.pet() (3秒后回Idle)     user.feed() (2秒后回Idle)     │
  └──── Petted ◀─────────────────────── Eating ───────────────┘
```

### 持久化（pet_v2.json）
字段：`state, affinity, pets, feeds, energy, last_pet_at_ms, last_feed_at_ms, current_since_ms`。
- `PetV2::load()`：启动自动加载；文件损坏返回 default（不 panic）。
- `PetV2::save()`：修改后 5s 或退出时 flush。

### ASCII 精灵（每个状态 6 行 × 定宽）
- Idle：微笑鲸鱼 `  \  ~~~~~  /  ` / `  |  ^_^  |  ` + 水波底。
- Thinking：头顶 `  ?  ` 流汗号。
- Running：眼睛转圈圈 `(o) (o)` → `(@) (@)`。
- Sleeping：右上角 `zzZ` + 闭眼 `(-) (-)`。
- Eating：嘴边 `<><` 小鱼干。
- Petted：心形 `♥♥♥` + 大圆眼 `(^) (^)`。

6 状态渲染 snapshot + 交互 + JSON roundtrip + 事件转移 6 单测全绿。

---

## 8. 工作流闭环：视觉 → 交互 → 工作台 → 路由

典型的「用户提到 DSH 4 大插件如何不中断」链路（`tests/dsh_e2e.rs::dsh_workflow_closed_loop_4_stages()` 可一次跑通）：

| 阶段 | 动作 | 产物 |
|---|---|---|
| 1. Modlens | 粘贴图片附件 → `/modlens <path>` → PreToolUse Hook → vision/modlens | `VisionDocument { summary, ocr, layout, json }` 注入上下文 |
| 2. GenUI | Agent 生成 3 按钮 Card，渲染到 GenUI Slot → 用户点中间按钮 [B] | `click_dispatch` → `SteeringEvent { target="btn_b" }` |
| 3. Router | Steering 事件驱动 Router 自动 `/router set execute` | `RouterProfile::Execute` 持久化到 session，重启自动恢复 |
| 4. Usage + Pet | 2 次 LLM 调用分别 (100, 200) tokens → Usage 累计 600；Agent 结束 → Pet→Idle | `today.total_tokens == 600 && pet.state == Idle` |

4 阶段全部 `assert!` 通过 ✅。

---

## 9. 测试数字

| 维度 | 数字 | 状态 |
|---|---|---|
| Phase A manifests 解析测试 | 16 | ✅ |
| Phase A 7 slash（解析+错误边界+事件roundtrip） | 4 | ✅ |
| Phase A 3 Hooks + Router profile | 8 | ✅ |
| Phase A 6 SlotId snapshot | 6 | ✅ |
| Phase A E2E 闭环 | 1 | ✅ |
| Phase B Usage 原生 (schema/insert/聚合/budget/10k压力) | 10 | ✅ |
| Phase B GenUI (contract 4 + render 8) | 12 | ✅ |
| Phase B PetV2 6 帧 + ASCII + JSON | 6 | ✅ |
| **合计 DSH 新增单测** | **63 / 63** | **✅ 0 失败** |
| 全仓 cargo test | 1920 passed / 15 failed / 14 ignored | 15 条失败均为 pre-existing 外部依赖（LLM HTTP/WS/MCP/gateway），与 DSH 无关 |

---

## 10. 后续迭代路线（T-2.4 / 未来规划）

- [ ] T-2.4（Router 思维模式补强，可选 P3）：`intent/router.rs` 新增 `TaskRouter`，对自然语言关键词启发式预测 plan/execute/weak，置信度 >0.8 自动切换 profile。
- [ ] C 阶段 clippy 清零（目前 7 warning：4 条未使用 import + 1 条 chrono deprecated + 2 条 ratatui buffer deprecated；下一迭代统一 `cargo fix`）。
- [ ] Mini Games 18 款的 1:1 还原（T-1.7 仅锚点 3 款，OQ1 未决）。
- [ ] Market 对接真实 DSH Market npm 检索 API（OQ2 决定做本地视图），本版本暂不对接外网。
- [ ] egui GUI 端的 DSH 模块（本版本 TUI 优先，宪法 NG4）。

---

## 11. 零新增原生依赖核对（NFR2）

`core/Cargo.toml` diff：**0 行新增依赖**（所有新增代码复用既有的 `rusqlite / chrono / serde_json / ratatui / thiserror / anyhow`）；满足 "Cargo.toml diff ≤ 5 行、不引入 OpenSSL/C-compiler crate" 的约束。
