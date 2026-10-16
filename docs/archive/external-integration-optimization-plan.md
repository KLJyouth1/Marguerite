# Marguerite 外部项目调研与二次开发优化方案

> 生成日期：2026-08-16
> 范围：27 个 GitHub 仓库 + 13 个本地外部项目目录
> 目标：统一项目结构、扩展 Agent 能力、增强 Computer Use、修复网关缺陷

---

## 一、资源清单与完整性核验

### 1.1 Harness 类仓库（18 个，`third_party/harness/`）

| 仓库 | 语言 | 文件数 | HEAD | 核心价值 |
|------|------|--------|------|----------|
| deepseek-harness | TypeScript/Cordis | 7412 | `47f9438` | 插件化 Agent Harness 架构（Cordis 时空可组合性） |
| CodeWhale | Rust | 1552 | `5635b5c` | 多角色 Fleet 模型路由（每角色独立 provider/model/reasoning） |
| awesome-deepseek-harness | Markdown | 10 | `caf6dde` | DSH 插件/资源索引 |
| penguin-harness | TypeScript | 1031 | `d14be6f` | 自进化 Agent 构建器（一键创建自我改进 Agent） |
| awesome-dsh-plugin | Markdown | 39 | `86d4b18` | DSH 插件目录站 |
| CowAgent | Go | 860 | `b9a5b1d` | 超级 AI 助手：主动规划 + 电脑控制 + 技能 + 知识库 + 自进化 |
| dsh-agent-teams | TypeScript | 79 | `d1d31c1` | 多智能体团队协作（队长→拉成员→拆任务→邮箱直传） |
| Abu-Cowork | TypeScript | 1838 | `c9ad2c9` | 桌面办公助手（Claude Code Cowork 模式启发） |
| dsh-desktop | TypeScript | 35 | `cbcb931` | DSH 跨平台桌面壳（自动启动本地实例 + 随机回环端口） |
| deepseek-harness(HenryZ838978) | TypeScript | 82 | `dc861b2` | DSH MCP 包发布分支 |
| command-code | TypeScript | 16 | `5c8f1b4` | 学习编码风格的 Coding Agent（Taste 系统） |
| loushang | Rust | 2410 | `f6606df` | 方法原生 AI 工作系统（method→stage→role→tool→session） |
| deepseek-auto-evolving-harness | Python | 691 | `6004c67` | 6 轮自进化 harness（0.68→0.80 分，逼近 Claude Code） |
| superpowers-reasonix | Markdown | 104 | `93490fe` | 11 项工程纪律技能（TDD/系统调试/完成前验证） |
| iPolloWork | TypeScript | 8518 | `d658032` | 本地优先可视化 AI 工作台（代码/文档/PPT/网站/设计/视频） |
| awesome-deepseek-harness(Dominic789654) | Markdown | 40 | `85cee4f` | DSH ACP 插件索引 |
| LeAgent | Python/React | 2276 | `1f16bad` | 桌面 Agent：流式运行时 + 可视化工作流 + 生成式 UI + 100+ 离线工具 |
| Guided-Attention-Inference-Network | Python | 25 | `cd1cb0a` | GAIN 引导注意力推断网络（GUI 元素注意力监督） |

### 1.2 Computer Use 类仓库（9 个，`third_party/computer_use/`）

| 仓库 | 语言 | 文件数 | HEAD | 核心价值 |
|------|------|--------|------|----------|
| Ros_Qt5_Gui_App | C++/Qt | 411 | `b0825e3` | ROS Qt5 GUI 应用（多窗口布局/地图编辑/图像显示） |
| computer_use_ootb | Python | 47 | `21b05ae` | OOTB 桌面 GUI Agent（Claude Computer Use + ShowUI + UI-TARS） |
| robotgo | Go/Cgo | 169 | `766c6ab` | 跨平台桌面自动化（键鼠/屏幕/窗口/位图/全局事件） |
| computer-vision-guide | Python | 220 | `390cd5f` | 计算机视觉指南（OpenCV 基础到进阶） |
| Digital-Forensics-Guide | Markdown | 4 | `a3d0981` | 数字取证指南（证据采集/分析工具链） |
| awesome-computer-use | Markdown | 1 | `1ef73a8` | Computer Use 资源索引 |
| computer-agent | Python | 50 | `92795e7` | Computer Agent 参考实现 |
| ScreenSpot-Pro-GUI-Grounding | Python | 112 | `dbe0011` | 高分辨率 GUI Grounding 基准（SE-GUI 7B 达 47.2%） |
| OpenCUA | Python | 153 | `dfc91ba` | 开源 CUA 基础模型（AgentNet 数据集 + 7B/32B/72B，OSWorld 45%） |

### 1.3 本地外部项目（13 项，`third_party/local/`，Junction 联接）

| 项目 | 文件数 | 核心价值 |
|------|--------|----------|
| external/agent-s | 159 | Agent S 参考实现 |
| external/claude-code-plugins | 59 | Claude Code 插件集 |
| external/claude-code-source | 2269 | Claude Code 源码参考（权限模型/diff review） |
| external/claude-mem | 734 | 5 生命周期钩子 + SQLite + Chroma 向量记忆 |
| external/deepseek-tui | 455 | DeepSeek TUI 终端界面 |
| external/ds4 | 52 | DS4 工具集 |
| external/hermes-agent | 2616 | SQLite+FTS5 会话/记忆/Provider 三元组 + 凭据池 |
| external/openclaw | 16433 | Provider Plane 25 适配器 + Anthropic Messages + 多渠道网关 |
| external/opencode | 4657 | Plugin Slot（tool+skill 统一）+ LSP 集成 + Electron |
| opensquilla | 11345 | Python 微内核（已迁入 pyengine/） |
| _GUI_Agent_archived_20260713 | 231772 | GUI Agent 归档（含 external/ 全部） |
| GUI_Agent | 163725 | GUI Agent 主项目 |
| external/AUDIT_REPORT.md | — | 外部源码审计报告（差距分析） |

---

## 二、已修复问题

### 2.1 Rust 网关方法缺失（Phase 3 引擎桥接）

**问题**：`sandbox.path.list` 与 `onboarding.catalog` 在 Python 侧车离线时返回 `NOT_IMPLEMENTED`，导致：
- 项目工作区选择器报「无法加载平台目录」
- 设置页 / 渠道编辑器无法渲染 provider 选择

**修复内容**：

| 文件 | 改动 |
|------|------|
| `core/src/gateway/protocol.rs` | `SUPPORTED_METHODS` 新增 5 个方法：`sandbox.path.pick/list/create-directory`、`onboarding.catalog/status` |
| `core/src/gateway/methods.rs` | `ASYNC_ONLY_METHODS` 新增 `sandbox.path.*` 三个方法；`RUST_FIRST_METHODS` 新增 `onboarding.catalog/status` |
| `core/src/gateway/methods.rs` | 新增 `onboarding_catalog()` 函数：内置 9 个 LLM provider（DeepSeek/OpenAI/Anthropic/Qwen/Zhipu/Moonshot/SiliconFlow/OpenRouter/Custom）+ 搜索/图像/嵌入 provider 目录 |
| `core/src/gateway/methods.rs` | 新增 `onboarding_status()` 函数：基于 config.yaml 的确定性引导状态（侧车离线时可用） |
| `core/src/gateway/methods.rs` | 新增 2 个单元测试验证 catalog/status 契约 |

**验证**：`cargo test -p marguerite-core --lib onboarding` → 2 passed；`test_supported_methods_subset_of_dispatch` → passed（所有 SUPPORTED_METHODS 均有实现）。

---

## 三、优化方案

### 3.1 统一项目结构与依赖管理

**现状**：
- Rust workspace（core + gui + tui）+ Python pyengine + Vue3 webui + React Tauri
- third_party 下 harness/computer_use/local 三类外部资源已就位
- 依赖管理分散：Cargo.toml / pyproject.toml / package.json 三套

**优化措施**：

1. **third_party 清单化**：在 `third_party/` 下维护 `SOURCES.json`（已存在 `manifest.json`），记录每个仓库的 git URL、commit、用途标签、集成状态。新增本地 junction 项目的元信息。
2. **Cargo workspace 统一**：将可复用的 Rust 外部 crate（如 CodeWhale 的 fleet 路由、loushang 的 method 引擎）作为 git submodule 或 path dependency 引入，而非复制代码。
3. **Python 依赖隔离**：pyengine 的 `pyproject.toml` 增加 `[tool.optional-dependencies]` 分组（computer-use / cv / embedding），computer_use_ootb / OpenCUA 的重依赖（torch/vllm）作为可选组，不强制安装。
4. **Node 依赖统一**：webui 与 Tauri GUI 共享 pnpm workspace；harness 类 TS 项目（deepseek-harness/LeAgent/iPolloWork）的插件通过 npm 安装而非源码嵌入。

### 3.2 Agent 能力扩展（基于 Harness 类项目）

#### 3.2.1 多智能体团队协作（dsh-agent-teams / CowAgent / LeAgent）

**现状**：Marguerite 已有 `agent/team.rs` + `plans.parallel.run`，但团队协作仅支持简单的子任务派发。

**优化**：
- 引入 dsh-agent-teams 的「队长→拉成员→拆任务→邮箱直传」模式：成员间直接收发消息（无需队长中转），支持子代理聊天
- CowAgent 的主动规划循环：Agent 主动发现任务、创建计划、执行、自进化
- LeAgent 的三层记忆（episodic / semantic / procedural）与检查点续跑

**落地路径**：扩展 `agent/team.rs` 的 `TeamChannel`，增加 `Mailbox` 结构（成员间消息队列）；在 `plans.parallel.run` 中支持成员间依赖声明与直接消息。

#### 3.2.2 自进化能力（deepseek-auto-evolving-harness / penguin-harness / CowAgent）

**现状**：`harness/distill.rs` 已有 `SelfEvolveGate` + `EvolutionProposal` + `mutate_prompt`，但仅为框架。

**优化**：
- 参考 deepseek-auto-evolving-harness 的 6 轮进化方法论：缩短执行路径 → 修正工具调用兼容错误格式 → 调整 system prompt → 调整推理参数 → 增加执行后反思
- penguin-harness 的「一键创建自进化 Agent」：在 TUI/WebUI 中暴露 `harness evolve` 命令
- 将进化结果落盘到 `harness/evolutions/` 目录，支持回滚

#### 3.2.3 多角色 Fleet 模型路由（CodeWhale / loushang / openclaw）

**现状**：`model/` 模块 4352 行，有 `router/decision` + `selflearning.status`，但单 provider 为主。

**优化**：
- CodeWhale Fleet：每角色（architect/builder/reviewer）独立绑定 provider + model + reasoning level，便宜模型指挥贵模型
- openclaw 的 25 provider 适配器 + failover chain + Anthropic Messages API 格式支持
- loushang 的 method-native 路由：method→stage→role→tool 作为运行时对象
- 凭据池（hermes-agent）：多 API key 按预算轮转

**落地路径**：在 `model/` 下新增 `fleet.rs`（角色→模型绑定）、`failover.rs`（故障转移链）、`credential_pool.rs`（多 key 轮转）；扩展 `router/decision` 支持角色感知路由。

#### 3.2.4 工程纪律技能（superpowers-reasonix）

**现状**：`skills/` 模块 1422 行，支持 SKILL.md 渐进披露。

**优化**：移植 superpowers-reasonix 的 11 项技能为内置技能：
- brainstorming / writing-plans / test-driven-development / systematic-debugging
- verification-before-completion / executing-plans / git-worktrees
- finishing-branch / code-review / writing-skills / sync-upstream

这些技能以 `skills/builtin/superpowers-*/SKILL.md` 形式落盘，由现有 SkillLoader 加载。

#### 3.2.5 生成式 UI 与可视化工作流（LeAgent / iPolloWork）

**现状**：webui 是 Vue3 控制台，TUI 是 ratatui，无生成式 UI。

**优化**：
- LeAgent 的声明式 UI 树（KPI 板/幻灯片/画廊/步骤器）流式渲染到聊天中
- iPolloWork 的统一工作区（仓库/文件/浏览器/文档/PPT/网站/设计/视频）
- LeAgent 的 ReactFlow DAG 工作流编辑器（每个工具自动成为类型化节点）

**落地路径**：webui 新增 `GenerativeCanvas.vue` 组件，接收 `ui.tree` 事件流并渲染；远期引入 Vue Flow 做工作流编辑器。

### 3.3 Computer Use 增强

#### 3.3.1 现状

`core/src/tools/computer_use.rs`（490 行）已实现三层架构：
- `ScreenPerception`：PowerShell/screencapture 截图 + 多屏 + 降采样
- `InputControl`：点击/双击/拖拽/滚轮/输入/按键/组合键
- `GuiGrounding`：描述定位 + 坐标定位

#### 3.3.2 增强方向

| 层 | 增强项 | 参考项目 |
|----|--------|----------|
| 屏幕感知 | 窗口枚举（按标题/进程名选择窗口）、窗口置顶/聚焦、区域截图、UI 元素树（a11y）提取 | robotgo（窗口句柄）、OpenCUA（元素检测）、computer_use_ootb（多显示器） |
| 输入控制 | 右键/中键、长按、拖拽释放分离、剪贴板读写、全局热键监听、鼠标手势 | robotgo（全局事件监听）、OpenCUA（动作空间） |
| GUI 定位 | VLMM grounding 闭环（截图→VLM 输出坐标→置信度→执行）、ScreenSpot-Pro 评测对齐、GAIN 注意力监督辅助定位 | ScreenSpot-Pro（grounding 基准）、OpenCUA（7B/32B/72B CUA 模型）、GAIN（注意力图监督） |
| 动作执行 | 动作队列（批量执行+回滚）、失败重试、执行前确认（PlanGate 集成） | computer_use_ootb（动作循环）、OpenCUA（AgentNetBench 评估） |

**落地路径**：
1. 短期：扩展 `OsComputerUse` 增加窗口枚举/右键/剪贴板/全局热键（robotgo Go 代码可作为命令参考，Rust 侧用 win32 API 直接实现）
2. 中期：在 `vision/` 模块新增 `grounding.rs`，实现「截图→VLMM→坐标」闭环，复用现有 `VisionCapability`
3. 长期：集成 OpenCUA-7B 作为本地 CUA 模型（可选依赖，vLLM 推理），ScreenSpot-Pro 作为评测基准

### 3.4 记忆系统增强（claude-mem / hermes-agent / LeAgent）

**现状**：`memory/` 12 文件 3554 行，7 层记忆（sensory/short_term/long_term/recall/cross_project/behavior/experience），ExperienceLedger 持久化。

**优化**：
- claude-mem 的 5 生命周期钩子（SessionStart / UserPromptSubmit / PostToolUse / Summary / SessionEnd）→ 集成到 `agent/stream_loop.rs`
- hermes-agent 的 SQLite + FTS5 全文检索 → 替代/补充当前 JSON 文件持久化
- LeAgent 的三层认知记忆（episodic 情节 / semantic 语义 / procedural 程序）
- 隐私标签（claude-mem privacy tag）：记忆条目标记敏感度，支持遗忘

### 3.5 知识库增强

**现状**：`kb/` 6 文件 983 行，BM25 检索 + 状态 + 灌入。

**优化**：
- 引入向量嵌入（memoryEmbeddingProviders 已在 onboarding catalog 中预留）：SiliconFlow/OpenAI embeddings
- 混合检索（BM25 + 向量 + 重排序）
- 知识库增量更新（文件监听 + 哈希去重）

### 3.6 技能与插件系统

**现状**：`skills/` 1422 行（SKILL.md 渐进披露）+ `plugins/` 748 行。

**优化**：
- opencode 的 Plugin Slot：tool + skill 统一注册（MCP + ACP 双协议）
- deepseek-harness 的 Cordis 插件模型：一切皆插件，服务即插件
- dsh-plugin 生态兼容：支持安装 `dsh-plugin` 格式的插件包
- superpowers-reasonix 11 项内置技能（见 3.2.4）

### 3.7 模型并行与推理模式

**现状**：`model/` 支持多 provider，但并行能力有限；`mode.rs` 有 Plan/Ask/Yolo 模式。

**优化**：
- **模型并行**：CodeWhale Fleet 多角色并行（builder + reviewer 同时工作）；`plans.parallel.run` 已有框架，需扩展到模型层
- **推理模式**：支持 thinking/non-thinking 模型切换（deepseek-auto-evolving-harness 数据显示 thinking 模型在不同任务上差异显著）
- **模型集成**：多模型投票/共识（ensemble），`llm_ensemble` 配置已有雏形
- **成本路由**：按任务复杂度路由到不同价位模型（router.selflearning 已有自学习框架）

### 3.8 输出方式与对话 UI/UX

**现状**：
- TUI（ratatui）+ WebUI（Vue3）+ Tauri 桌面端
- WebSocket 事件流（tick/session.event.*/plan_revision）
- 流式聊天 + 计划卡片 + 文件更改卡片

**优化**：
- **流式输出增强**：openclaw 的 per-chunk yield + onChunk 回调（当前 Rust 侧已有流式，但 Python 侧 StreamingHandler 未完全接入）
- **生成式 UI**：LeAgent 风格的内联交互式组件（KPI 板/幻灯片/代码差异预览）
- **多模态输出**：图片/图表/表格内联渲染（doubao-visualization 技能可集成）
- **对话过程可视化**：dsh-agent-teams 的团队活动面板（右上角实时显示成员活动）
- **TUI 增强**：deepseek-tui 的界面参考；loushang 的 method-guided TUI 工作台
- **桌面宠物**：LeAgent 的侧边栏宠物（行走/跳跃动画 + 个性气泡，同步聊天状态）
- **命令面板**：iPolloWork/LeAgent 风格的 Ctrl+K 命令面板

### 3.9 浏览器操控

**现状**：`browser/` 模块有 CDP 桥（open/navigate/evaluate/screenshot/a11y/click/close），7 个 RPC 方法。

**优化**：
- 多标签页管理（当前单页）
- 网络请求拦截/修改（Playwright 风格的 route）
- 文件下载处理
- 浏览器状态持久化（cookie/localStorage）
- 参考 computer_use_ootb 的浏览器控制循环

### 3.10 本地电脑操控 / PC Use

**现状**：`tools/computer_use.rs` 三层架构 + `tools/terminal.rs` 常驻 shell + `sandbox/` 策略护栏。

**优化**：
- robotgo 的全局事件监听（键盘/鼠标全局钩子）→ 新增 `computer_use.listen` 工具
- 窗口管理（枚举/聚焦/移动/调整大小/最小化）
- 系统通知、剪贴板、进程管理
- 多显示器工作区（computer_use_ootb 已突破单显示器限制）
- 远程控制（computer_use_ootb 支持移动端远程控制电脑）
- 与 PlanGate/CompletionGate 集成：每个 GUI 动作执行前过策略检查

---

## 四、实施优先级

| 优先级 | 事项 | 预期工作量 | 依赖 |
|--------|------|-----------|------|
| P0 | 网关方法修复（已完成） | — | — |
| P0 | superpowers 11 项内置技能落盘 | 小 | skills/loader 已有 |
| P1 | onboarding catalog 补全 channels/audio providers | 小 | 本次修复基础 |
| P1 | computer_use 窗口枚举 + 右键 + 剪贴板 | 中 | win32 API |
| P1 | 记忆生命周期钩子（5 hooks） | 中 | agent/stream_loop |
| P1 | Fleet 多角色模型路由 | 大 | model/ 模块 |
| P2 | 团队邮箱直传（dsh-agent-teams） | 中 | agent/team |
| P2 | 向量嵌入混合检索 KB | 中 | kb/ + embedding provider |
| P2 | Plugin Slot 统一 tool+skill | 大 | plugins/ + skills/ |
| P2 | 生成式 UI 组件（webui） | 大 | Vue3 + 事件流 |
| P3 | OpenCUA-7B 本地 CUA 集成 | 大 | Python 可选依赖 |
| P3 | 自进化命令（harness evolve） | 中 | harness/distill |
| P3 | 全局事件监听（robotgo 参考） | 中 | computer_use |
| P3 | 桌面宠物 / 命令面板 | 中 | webui/Tauri |

---

## 五、third_party 目录结构（最终状态）

```
third_party/
├── harness/                    # 18 个 harness 类 GitHub 仓库
│   ├── deepseek-harness/
│   ├── CodeWhale/
│   ├── ... (18 repos)
│   └── HARNESS_BRIEF.md
├── computer_use/               # 9 个 computer use 类 GitHub 仓库
│   ├── computer_use_ootb/
│   ├── OpenCUA/
│   ├── ... (9 repos)
│   └── INTEGRATION_BRIEF.md
├── local/                      # 本地外部项目（Junction 联接，不占额外磁盘）
│   ├── external/
│   │   ├── agent-s -> E:\...\agent-s
│   │   ├── claude-code-plugins -> ...
│   │   ├── claude-code-source -> ...
│   │   ├── claude-mem -> ...
│   │   ├── deepseek-tui -> ...
│   │   ├── ds4 -> ...
│   │   ├── hermes-agent -> ...
│   │   ├── openclaw -> ...
│   │   ├── opencode -> ...
│   │   └── AUDIT_REPORT.md     # (真实文件副本)
│   ├── opensquilla -> E:\...\opensquilla
│   ├── GUI_Agent -> E:\...\GUI_Agent
│   └── _GUI_Agent_archived_20260713 -> E:\...\archived
└── manifest.json               # 仓库清单与集成状态
```

---

## 六、风险与注意事项

1. **许可证合规**：各仓库许可证不一（MIT/Apache-2.0/等），二次开发时需保留原始 LICENSE 与版权声明；`THIRD_PARTY_NOTICES.md` 需同步更新。
2. **依赖膨胀**：OpenCUA/computer_use_ootb 依赖 torch/vllm（数 GB），必须作为可选依赖组，不强制安装。
3. **Junction 可移植性**：本地项目通过 Windows Junction 联接，迁移项目目录时需重新创建联接；跨平台需改为符号链接或子模块。
4. **Python 侧车状态**：onboarding.catalog 的 Rust 回退为最小目录，完整 provider 目录（含 presets/tiers/verification）仍由 Python 侧车提供；两侧字段需保持对齐。
5. **既有测试失败**：`test_simple_task_skips_auto_decompose` 与 `e2e_fusion_methods_over_ws` 为既有失败（chat 模块异步时序问题），与本次网关修复无关，建议单独排查。
