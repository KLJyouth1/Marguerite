# Marguerite × 第三方仓库二开集成方案

> 统一项目结构与依赖管理 · 基于 harness 扩展 agent 能力 · 重构增强 computer use tool
> 生成时间：2026-08-14 ｜ 主理人：SoftwareWorkshop (gstack-lead) ｜ 状态：**进行中（可落地）**

---

## 0. 状态快照（Status Snapshot）

| 项 | 状态 | 说明 |
|----|------|------|
| 仓库克隆 | ✅ 27/27 | 18 个 agent‑harness + 9 个 computer‑use，全部 `.git` 有效（`manifest.json` 为权威源） |
| `computer_use.rs` 三层重构 | ✅ 已实现（磁盘未提交） | `ScreenPerception` / `InputControl` / `GuiGrounding` 三 trait + `locate` grounding 闭环 |
| 插件清单子系统 | ✅ 已存在且成熟 | `core/src/plugins/manifest.rs`（`PluginManifest` + `ManifestPlugin` + `HookRegistry`，13 个单测） |
| 配套子系统 | ✅ 已存在 | `skills/`、`memory/`、`agent/team.rs`、`agent/multi_strategy.rs`、`harness/distill.rs`、`stream_loop.rs`、`context/optimize.rs`、`events/bus.rs`、`chain/`、`collab/`、`automation/`、`browser/`(CDP) |
| 编译验证 | ⏳ 后台运行中 | 任务 `dFkY8W`，Windows 环境有 `os error 5`/Defender 锁风险 |
| 本方案落盘 | ✅ | 本文件 + `third_party/INTEGRATION_BRIEF.md` + `third_party/HARNESS_BRIEF.md` + `third_party/plugin-manifest-spec.md` |

**关键认知（避免重复造轮子）**：第三方仓库的价值不是"把它们的代码搬进来"，而是
(1) 把它们暴露的 **能力** 写成 Marguerite 的 **插件清单（PluginManifest）**；
(2) 用它们的架构模式 **增强已有子系统**（`team.rs`/`distill.rs`/`memory/`/`skills/`/`stream_loop.rs`）。
MARGUERITE 的插件系统已经比多数第三方 harness 更完整——本方案是"填充 + 增强"，不是"从零搭建"。

---

## 1. 集成策略（三需求 → 三个工作流）

| 用户需求 | 工作流 | 落点 |
|---------|--------|------|
| ① 统一项目结构与依赖管理，各模块可协同 | **WS‑A 结构/依赖治理** | `third_party/` 作为"参考 + 适配层"；`manifest.json` 为唯一事实源；`plugins` 为消费第三方能力的机制 |
| ② 基于 harness 项目扩展 agent 能力 | **WS‑B 能力扩展** | 17 个 harness → 插件清单 + 对已有子系统的针对性增强（见 §5） |
| ③ 基于 computer use 项目重构增强 computer use tool | **WS‑C 工具重构** | `core/src/tools/computer_use.rs` 三层重构（已完成，见 §4） |

---

## 2. 仓库清单（Repository Inventory）

权威统计见 `third_party/manifest.json`（`gen_manifest.py` 生成，`schema=marguerite-third-party-integration/v1`）。

**A. agent‑harness 基础（18，用于 ②）**
`Abu-Cowork`(TS/Electron) · `CodeWhale`(Rust) · `CowAgent`(Py) · `deepseek-harness`(TS) · `deepseek-harness-HenryZ838978`(Py+TS/MCP) · `deepseek-auto-evolving-harness`(Py) · `awesome-deepseek-harness`(MD) · `awesome-deepseek-harness-Dominic789654`(MD) · `awesome-dsh-plugin`(MD) · `command-code`(JS) · `CowAgent` · `dsh-agent-teams`(TS) · `dsh-desktop`(TS) · `iPolloWork`(TS) · `LeAgent`(Py+React) · `loushang`(Py) · `penguin-harness`(TS) · `superpowers-reasonix`(MD/JS) · `Guided-Attention-Inference-Network`(实验性视觉注意力)。

**B. computer‑use 优化（9，用于 ③）**
`awesome-computer-use`(MD 综述) · `computer_use_ootb`(Py) · `computer-agent`(Rust/Tauri) · `computer-vision-guide`(MD) · `Digital-Forensics-Guide`(MD) · `OpenCUA`(Py) · `robotgo`(Go) · `Ros_Qt5_Gui_App`(C++) · `ScreenSpot-Pro-GUI-Grounding`(定位模型)。

> 语言探测：`CowAgent`→Py，`computer-agent`→JS/TS，`computer_use_ootb`→Py，`robotgo`→Go，`Ros_Qt5_Gui_App`→C++，其余多为 MD 索引或混合；这与"第三方=参考层、不进 cargo 工作区"的治理策略一致。

---

## 3. 统一结构与依赖管理（WS‑A）

### 3.1 原则
- **Marguerite core = Rust cargo 工作区（权威运行时）**。`third_party/` = **参考 + 适配层（不编译进工作区）**。
- 不把第三方仓库作为 cargo 成员编译，原因：工具链混杂（Go/Py/TS/Electron/C++）、无需耦合构建、避免锁文件/供应链膨胀。
- 第三方的价值通过两条路径进入 Marguerite：
  1. **模式 → Rust 实现**：如 computer‑use 的模式落地为 `computer_use.rs`（已完成）。
  2. **能力 → 插件清单 / 外部适配器**：harness 的能力写成 `PluginManifest`；需要进程级能力的，经现有 `mcp/` 或 `gateway/` 作为外部进程/协议接入。

### 3.2 manifest.json 为唯一事实源
- `gen_manifest.py`（根目录 `os.getcwd()` 解析 + Defender 锁重试写）扫描 `harness/*/.git` 与 `computer_use/*/.git`，输出 `manifest.json`（含 `cat/name/role/remote/head/language/local_path/status/notes`）。
- **仓库增减后重跑** `python3 gen_manifest.py` 即可；`clone_status_live.json` 已由本方案同步为一致状态。

### 3.3 协同机制
- 能力注册：所有可复用能力经 `core/src/plugins` 的 `PluginRegistry` 路由（能力名 → 插件）。
- computer‑use 工具：作为能力注册进 `PluginRegistry`（见 §6），与第三方 harness 的 "tool‑use" 模式天然对齐。
- 跨进程：仅当某第三方仓库本身是兼容实现（如 Rust 的 CodeWhale、Go 的 robotgo）且确有必要时，才考虑 `mcp/`/外部进程桥接；优先用 Marguerite 自有 Rust 实现。

---

## 4. Computer‑Use Tool 重构（WS‑C，已完成）

文件：`core/src/tools/computer_use.rs`（543 行）。设计已验证（grep 确认三 trait、`SetCursorPos` 修复、`threshold` 门控均就位）。

### 4.1 三层架构
```rust
pub trait ScreenPerception { fn capture_cmd(&self, out_path:&str, mode:CaptureMode, max_edge:u32)->Result<String>; }
pub trait InputControl { click / double_click / drag / scroll / move / type / key / key_combo }
pub trait GuiGrounding { locate_by_description / locate_by_coords }
```
- **CaptureMode**：`Primary`(主屏) / `All`(虚拟桌面 `SystemInformation::VirtualScreen`，多屏) / `Window`(`GetForegroundWindow`+`GetWindowRect`，活动窗口)。
- **max_edge**：长边等比降采样，降低发给 VLMM 的截图体积（对齐 OpenCUA / computer_use_ootb）。

### 4.2 `locate` grounding 闭环（`dispatch`）
```
截图(ScreenPerception) → VisionCapability::locate(bytes, desc)(GuiGrounding/VLMM)
  → 坐标空间对齐(|x|<=1 ⇒ 按 image_metadata 宽高还原像素)
  → threshold 置信度门控(低于则不自动点击)
  → 可选 auto_click(InputControl) 完成闭环
```
- **Windows 点击 bug 修复**：原 `mouse_event` 在当前光标位置点击、忽略 (x,y)；现统一 `SetCursorPos(x,y)` 先移动再 `mouse_event`（user32），双击/拖拽/滚轮均"先移动到坐标"。
- Schema 扩展：`action` 枚举扩为 `screenshot/click/double_click/type/key/key_combo/scroll/drag/move/perceive/locate`；新增 `mode`、`max_edge`、`threshold`、各输入参数。

### 4.3 模式来源映射（来自 `INTEGRATION_BRIEF.md`）
| 增强点 | 来源仓库 |
|--------|---------|
| 固定 AI 空间 + 双向 `map_from_ai_space` | computer-agent |
| 先移动再操作（move‑then‑act） | computer-agent / computer_use_ootb |
| Windows DPI 缩放（坐标乘 ScaleF） | robotgo `MoveScale` |
| 多屏枚举并保留 offset | computer_use_ootb |
| 归一化 vs 像素坐标契约（>1.0 判像素） | OpenCUA |
| 特征门控的 OCR / 无障碍树辅助感知 | robotgo OCR / CDP `GetFullAxTree` / UIA |
| zoom 二次精修（全分辨率裁剪） | computer-agent |

---

## 5. Agent 能力扩展（WS‑B）

### 5.1 事实：插件清单子系统已存在
`core/src/plugins/manifest.rs` 已提供：
- `PluginManifest { name, version, description, author, capabilities, hooks(PreToolUse/PostToolUse/Notification/Stop), slash_commands, subagents(model 偏好), skills }`
- `ManifestPlugin`：把一份清单桥接为原生 `Plugin` 并 `register` 进 `PluginRegistry`。
- `HookRegistry`：按事件顺序派发，复用 L1/L2/L3 护栏（`security` 模块落地），支持短路 deny。
- 13 个单测覆盖解析 / 能力推导 / 注册桥接 / 钩子派发。

**结论**：harness 集成的首要交付物不是"再写一个插件系统"，而是 **(a) 用 17 个 harness 写出具体 PluginManifest**，**(b) 对 `PluginManifest` 做小幅扩展** 以容纳 harness 特有的字段，**(c) 增强已有子系统**。

### 5.2 把 17 个 harness 落成插件清单（具体交付）
每个 harness → 一份 `PluginManifest`（声明其 subagents / hooks / skills / capabilities），经 `register_manifests()` 注册即可被 Marguerite 编排消费。
- 范例见 `third_party/plugin-manifest-spec.md` 与 `third_party/plugin-examples/dsh-agent-teams.plugin.json`（可直接被 `PluginManifest::from_json` 解析）。

### 5.3 对 `PluginManifest` 的小幅扩展（harness 启发）
| 扩展字段 | 灵感来源 | 落点 |
|---------|---------|------|
| 每能力 `model` / fleet 路由 | CodeWhale `fleets` | `SubagentSpec.model` 已存在 → 扩展到 capability 级；`agent/multi_strategy.rs` 消费 |
| `dependencies` / 分层 bundle | deepseek-harness `cordis.patch.yml` | `PluginManifest` 增加 `dependencies: Vec<String>` + 有序加载 |
| `events` 分类（Session/Agent/Capability） | deepseek-harness | 扩展 `HookEvent` 枚举 |
| `steering`（运行中注入指令） | penguin-harness | `stream_loop.rs` 增加 steering 窗口 |
| `self_evolve` 钩子 | deepseek-auto-evolving-harness | 在 `automation/cron.rs` 注册自演化任务 |

### 5.4 对已有子系统的针对性增强（优先级表）
| # | 模式（来自 HARNESS_BRIEF 综合） | 已有子系统 | 缺口 | 目标文件 | 优先级 |
|---|-------------------------------|-----------|------|---------|--------|
| 1 | JIT 工程纪律技能包（TDD/系统化调试/完成前验证） | `skills/`(engine/loader/registry) | 缺"纪律"技能包与 `AGENTS.md` 常驻加载 | `skills/` + `agent/runtime.rs` | **P0（最便宜高杠杆）** |
| 2 | 自演化循环（评测→诊断→变异→门控） | `harness/distill.rs` + `automation/` | 无"跑评测→改 prompt/工具 schema→按分数门控"的闭环 | `distill.rs` + `automation/cron.rs` | P1 |
| 3 | 按角色/fleet 模型路由 | `agent/multi_strategy.rs` | 无每角色模型/推理档路由 | `multi_strategy.rs` | P1 |
| 4 | P2P 多智能体 + 依赖图 | `agent/team.rs` | 当前偏"主管扇出"，缺成员直连信箱 + 依赖声明 + 唤醒调度 | `team.rs` | P1 |
| 5 | 分层/认知记忆蒸馏 | `memory/`(behavior/experience/long_term/short_term/sensory/recall) + `distill.rs` | 缺"会话→技能/记忆"的自动蒸馏（Deep Dream 式） | `memory/` + `distill.rs` | P2 |
| 6 | 可视化 DAG + 持久检查点 | `agent/tool_cycle.rs` + `stream_loop.rs` | 无工具=节点 DAG、并发分支、断点续跑 | `tool_cycle.rs` + `stream_loop.rs` | P2 |
| 7 | 结构化 model↔UI 协议 | `events/bus.rs` + `tui/workbench` | 缺"声明计划步骤 / 团队活动面板 / 生成式 UI 卡片 / steering 窗口"事件 | `events/` + `tui/workbench` | P2 |

> 完整逐仓分析与模式综合见 `third_party/HARNESS_BRIEF.md`（已包含每仓 4 要素 + 8 条综合模式）。

---

## 6. 集成指南（如何接线）

### 6.1 编写一个插件清单（以 dsh-agent-teams 为例）
1. 在 `third_party/plugin-examples/` 写 `*.plugin.json`，字段对齐 `PluginManifest`：
   ```json
   {
     "name": "dsh-agent-teams", "version": "0.1.0",
     "description": "多智能体团队：队长拆解任务→成员 P2P 信箱协作",
     "capabilities": ["team.spawn"],
     "subagents": [{"name":"researcher","description":"调研","tools":["read","web"],"model":"inherit"}],
     "hooks": [{"event":"PostToolUse","handler":"team_audit","params":{}}]
   }
   ```
2. 在 Rust 侧：
   ```rust
   let m = PluginManifest::from_json(json_str)?;
   let mut reg = PluginRegistry::new();
   register_manifests(&mut reg, &[m]);        // 桥接为 ManifestPlugin
   // 钩子：build_hook_registry(&[m], |h| resolve_guard(h));
   ```
3. 验证：`cargo test -p marguerite-core plugins::manifest` 应全绿（现有 13 个用例覆盖此路径）。

### 6.2 把 computer‑use 注册为能力
- `computer_use.rs` 的 `dispatch` 经 `register_computer_use_tools()` 暴露；将其能力名（如 `computer_use.locate`）登记进 `PluginRegistry`，即可被插件/编排器按能力路由调用——与第三方 harness 的 "tool‑use framework" 模式一致。

### 6.3 构建与验证（注意 Windows 环境）
- 编译门禁：`cargo check -p marguerite-core`（或 `cargo test`）。
- 本机已知顽疾：`target/` 写盘报 `os error 5`(拒绝访问) / `LNK1104`(DLL 无法打开)——由被杀构建进程残留锁句柄 + Windows Defender 实时扫描 oplock 引起。解法：
  - 用**全新** `CARGO_TARGET_DIR`（如 `.ct_verify_<时间戳>`）跑收敛重试；
  - 设 `TMPDIR` 到非系统盘（如 `/d/Downloads/Marguerite/_tmp`）；
  - 写盘遇 `PermissionError` 时重试（gen_manifest.py 已用此策略）。
- 当前编译验证任务：`dFkY8W`（后台）。

---

## 7. 测试策略（Testing Strategy）

| 层级 | 内容 | 验证方式 |
|------|------|---------|
| 单元（computer‑use） | 归一化↔像素坐标对齐、dispatch 参数解析、`threshold` 门控 | 新增 `#[cfg(test)]` 于 `computer_use.rs` |
| 单元（插件） | 清单解析/能力推导/注册桥接/钩子派发 | 现有 `manifest.rs` 13 个用例须保持绿 |
| 集成（grounding） | 截图→`VisionCapability::locate`→（可选）点击 闭环 | 模拟 VLMM 返回坐标的冒烟测试 |
| 编译门禁 | `cargo check -p marguerite-core` | 每次改动后；失败须先过类型校验再交付 |

---

## 8. 下一步（Next Actions，按优先级）

**P0（立即可做，低成本高杠杆）**
1. **提交 `computer_use.rs` 重构**（当前在磁盘、未 commit）——固化 WS‑C 成果。
2. **落地 JIT 工程纪律技能包**到 `skills/` + 常驻 `AGENTS.md`（模式 #7，纯 markdown + 路由）。
3. **写 1 份参考插件清单**（`dsh-agent-teams.plugin.json`，已就绪），跑通 `register_manifests` 路径。

**P1（架构增强）**
4. 扩展 `PluginManifest`：加 `dependencies`(分层 bundle) 与 capability 级 `model` 路由；在 `agent/multi_strategy.rs` 消费 fleet 路由。
5. 自演化闭环：在 `harness/distill.rs` + `automation/cron.rs` 实现"评测→诊断→变异 prompt/工具 schema→按分数门控"。
6. 升级 `agent/team.rs` 为 P2P 成员信箱 + 依赖图 + 唤醒调度。

**P2（体验/高级）**
7. `memory/` + `distill.rs` 的会话自动蒸馏（Deep Dream 式）。
8. `tool_cycle.rs` + `stream_loop.rs` 的工具 DAG 并发 + 断点续跑。
9. `events/bus.rs` + `tui/workbench` 的结构化 model↔UI 协议（计划声明/活动面板/生成式卡片/steering 窗口）。

**工程卫生**
10. 仓库增减后重跑 `gen_manifest.py`；保持 `clone_status_live.json` 与 `manifest.json` 一致。
11. 环境受限时，编译验证用全新 `CARGO_TARGET_DIR` + `TMPDIR` 重试。

---

## 附录 A：third_party 与 core 子系统对照
- 参考层：`third_party/harness/*`（18）、`third_party/computer_use/*`（9）、`manifest.json`、`INTEGRATION_BRIEF.md`、`HARNESS_BRIEF.md`、`plugin-manifest-spec.md`、`plugin-examples/*`。
- 运行时：`core/src/{plugins,tools/computer_use.rs,vision,agent/{team,multi_strategy,stream_loop,tool_cycle}.rs,harness/distill.rs,memory,skills,context/optimize.rs,events/bus.rs,automation/long_running.rs,chain,collab,orchestration,plan,browser}`。

## 附录 B：引用文档
- `third_party/INTEGRATION_BRIEF.md` — computer‑use 9 仓逐仓分析 + 9 条可移植模式综合。
- `third_party/HARNESS_BRIEF.md` — harness 17 仓逐仓分析 + 8 条 agent 能力综合模式。
- `third_party/plugin-manifest-spec.md` — 如何把 harness 写成 PluginManifest（含扩展提案与范例）。
