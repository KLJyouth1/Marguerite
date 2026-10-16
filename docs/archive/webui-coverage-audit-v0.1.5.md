# WebUI 改进覆盖审计（v0.1.5 · 跨会话汇总）

> 汇总来源：CHANGELOG v12–v15、docs/marguerite-dev-progress-2026-08-22.md 第 3.16–3.21 轮、
> _sess6.jsonl（31712 行历史会话）、deliverables 计划与 dsh-fusion v2–v8。
> 结论：核心层（Rust）改进已全部可被 WebUI 触达；WebUI 侧新增/增强在「可见」层落地。

## 一、改进项 → WebUI 覆盖矩阵

| 改进项 | Core/CLI 状态 | WebUI 呈现 | 状态 |
|---|---|---|---|
| 原生 Anthropic provider | LLMProvider::Anthropic + 2 种子 | 新增模型档案面板（models.profiles.list），支持切换主/副模型 | 已落地 |
| OpenAI/OpenRouter/DeepSeek 全家族种子 | registry seed_revision=2 | 同上（模型档案面板） | 已落地 |
| games list/play | CLI + 网关 RPC | GamesView（/games 导航） | 已落地 |
| 能力健康度 real/partial/叙事 | domain/capabilities + plans.capabilities | GamesView 横幅 + EngineHealthView 卡 | 已落地 |
| 重型运行时（ASR/本地量化） | runtime.status RPC | EngineHealthView 下载源 + 选型 | 已落地 |
| 运行时模式切换 | mode.get/mode.switch（含 OmniTranscend） | EngineHealthView 一键切换 | 已落地 |
| MCP 工具桥 | HTTP MCP 自动接线 + mcp tools CLI | 网关 tools.catalog 可被各视图消费（MCP 零配置时 UI 不下沉） | 后端已接入 |
| 子代理/团队/集群/swarm | Rust orchestrator DAG | ChatView SubagentCard / RunTrace（现有） | 已有 |
| 计划/执行坞/回滚 | plans.* + context.* | ChatView PlanCard/RunTrace/FileChanges | 已有 |
| 多文件工作区编辑器 | workspace.file.read/write + sandbox.path.list | WorkspaceEditorView（/editor） | 已落地 |
| 知识库/关系图谱 | kb.* + kb.graph.* | GraphView / 知识库设置 | 已有 |
| 宠物/扑克 | pet/poker | PetView / PokerView | 已有 |
| 桌面/浏览器/电脑操控 | desktop/browser/computer_use | ComputerUseView / 设置桌面运行时 | 已有 |
| Fleet 多角色路由 | fleet.list/route | FleetView | 已有 |
| 自进化/使用统计 | evolve/usage | EvolveView / UsageView | 已有 |

## 二、WebUI 新增（本轮可见改进）

1. **GamesView（/games）**：三款小游戏可玩 + 能力健康度横幅。
2. **EngineHealthView（/engine）**：
   - 能力健康度（plans.capabilities）
   - Rust 模型档案（models.profiles.list / set_active / set_copilot）
   - 重型运行时（runtime.status 下载源/选型）
   - 运行时模式（mode.get / mode.switch：plan/agent/ensemble/bloom/omni-transcend）
3. **网关新 RPC**：models.profiles.*（Rust-first，侧车离线可用）、runtime.status（早前已上）。

## 三、模式运行逻辑 UI（EngineHealthView 对齐）

- Plan / Agent / Ensemble / Bloom / OmniTranscend 一键切换；当前高亮。
- mode.get 兜底读 config 的 bloom.parallel_enabled 终态（后端 contract 已有）。

## 四、仍待深化（不在本轮）

- MCP 双传输均已支持：HTTP URL 型 + stdio/command 型（McpClient::new_stdio + 自动接线 + mcp.servers.probe；WebUI EngineHealthView 表单可填 command）。
- 内置多文件工作区编辑器已落地（/editor：文件树 + 多标签文本编辑 + workspace.file.read/write）；语法高亮/符号跳转仍待做。
- 模型档案现支持 WebUI 新增/删除/切换（models.profiles.add/remove/set_active/set_copilot）；custom 字段级编辑仍走 CLI。
- 任务执行质量可视化已在 EngineHealthView 落地（long_running.list 汇总：完成率/重试/检查点/中断 + 最近任务）；「评审/验证器质量评分卡」仍待后端落盘后接 UI。
