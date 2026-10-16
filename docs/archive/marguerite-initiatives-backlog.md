# Marguerite 剩余大件立项清单（Backlog / Initiatives）

日期: 2026-08-22  编制依据: marguerite-dev-progress-2026-08-22.md + domain/capabilities.rs 真实自检

> 说明：这些是审计中列为「纯叙事/部分实装」、需要**专项工程**的大件，未在本会话落地（非一次性有界改动）。
> 每题给出 能力映射 / 现状 / 范围 / 方案 / 验证 / 优先级 / 依赖风险，供逐项立项推进。
> 注：hindsight 知识页因缺少 API key(401) 无法建立，故以此文档为权威跟踪；接通密钥后可逐项补 memory 页。

> **状态更新（2026-08-22 新增轮次）**：以下项已由本会话逐一核验并落地（能力清单对应翻转 部分实装→已实装）：
> - ✅ Market 安装/升级（新增 market_upgrade：semver 升序 + tmp/rename 原子替换 + 单测）
> - ✅ DSH Bundle 拓扑加载（Rust Kahn 拓扑排序已实现，核验翻转）
> - ✅ Router 自动路由（intent×complexity 启发式已实现，核验翻转）
> - ✅ 结构化差异预览（diff + apply_patch 已实现，核验翻转）
> - ✅ 双模型配对（copilot 配对已实现，核验翻转）
> - ✅ 文件监听（notify + 事件总线已实现，核验翻转）
> - ✅ MAX 双带队 / 集群调度 / swarm DAG（dual_leader_specs/cluster_specs/dispatch_bloom_swarm 已实现，核验翻转）
> - ✅ Modlens 版面分析（VisionDocument.layout 已实现，核验翻转）
> - ✅ FPS 操作助手（computer_use_loop + humanize + skills/fps 已实现，核验翻转）
> - ⏳ 仍保持诚实 Partial：MCP 工具桥（现有为服务端，缺客户端桥接）、OS 沙箱（Windows 只读降级属设计）、语义/自进化记忆（证据不足未翻转）、私有知识库（核心 RAG 已 rdin，批量导入 Wizard 待做）
> - 🚫 外部大件（需专项/平台）：浏览器自动化（真机 Chromium）、UIA 桥、音频理解、本地量化模型

| 优先级 | 立项 | 能力映射 | 现状 | 一句话范围 |
|---|---|---|---|---|
| P0 | 浏览器自动化 | dsh.tool.browser | 部分实装 | 页面导航/截图/抽取（browser_bridge，需本机 Chromium） |
| P0 | MCP 工具桥 | dsh.tool.mcp_bridge | 部分实装 | 把任意 MCP server 工具暴露为原生工具 |
| P0 | OS 级沙箱 | dsh.sec.os_sandbox | 部分实装 | Landlock/Linux·Seatbelt/macOS 内核级隔离；Windows 命令级兜底 |
| P1 | UIA 跨应用桥 | dsh.auto.uia | 部分实装 | Win UIA 元素级：窗口句柄/控件树/触发；可与 desktop_control 组合 |
| P1 | 音频理解 | dsh.vis.audio | 纯叙事 | 语音/音频转写与理解（需音频管线 + ASR 模型） |
| P1 | 本地量化模型 | dsh.model.local | 纯叙事 | 本地量化模型离线运行（量化推理运行时） |
| P1 | Market 安装/升级 | dsh.market.install·upgrade | 部分实装 | 离线 manifest 原子安装/升级 + SKILL_SCAN 二次签名校验 |
| P1 | swarm/集群/MAX 编排 | orch.swarm·cluster·dual_leader | 部分实装 | DAG 并行 dispatch、fan-out 多 Worker、planner/executor 双轨 |
| P1 | 语义/自进化记忆 | mem.semantic·evolutionary | 部分实装 | 提炼知识片段 + 从经验沉淀可复用策略 |
| P1 | 自动评审/验证器 | gov.review·verifier | 部分实装 | 对 diff 结构化评审裁决 + 按验收标准判定达标（依赖 model.cloud） |
| P2 | 结构化差异预览 | core.diff | 部分实装 | 每次改动产出可读 diff + 一键接受/拒绝 |
| P2 | 文件监听 | tool.fs_watch | 部分实装 | 工作区递归 notify，GUI 侧接入「触发 Agent 反应」 |
| P2 | 私有知识库 | kb.ima | 部分实装 | 本地 RAG(BM25+向量) + cross_project + 画像导入；批量导入 Wizard |
| P2 | Router 自动路由 | dsh.router.auto_route | 部分实装 | 基于复杂度启发式选 plan/execute/weak profile |
| P2 | Modlens 版面分析 | dsh.vision.layout | 部分实装 | 图片版面分区供 extract_json 消费 |
| P2 | FPS 操作助手 | tool.fps_assistant | 部分实装 | Computer Use 封装的射击操作代理（需 Labs 显式开启 + FPS-agent 技能） |
| P2 | 双模型配对 | model.dual | 部分实装 | 规划/执行用不同模型（成本质量兼得） |
| P2 | DSH Bundle 拓扑加载 | dsh.bundle.topo_order | 部分实装 | 依据 dependencies 做 Kahn 拓扑排序 |
| P2 | Better Sidebar 打开路径 | dsh.sidebar.open_path | 部分实装 | 侧边栏聚焦并展开指定 path 节点 |

## 逐项立项（按优先级）

### P0-1 浏览器自动化
- 能力映射: dsh.tool.browser   |   现状: 部分实装
- 范围: 基于 browser_bridge（重型桥，需本机 Chromium）实现页面导航/截图/抽取；作为原生工具暴露给 Agent。
- 方案: 复用 core/src/browser/* + tools/builtin 的 web_fetch/web_search；接入 browser_bridge，注册 browser_navigate/screenshot/extract 工具，加离线 mock 测试。
- 验证: cargo test --lib browser + 真实启动 Chromium 冒烟；工具循环可调用。
- 依赖/风险: 依赖本机 Chromium/Playwright；注意回环/代理 gotcha。

### P0-2 MCP 工具桥
- 能力映射: dsh.tool.mcp_bridge   |   现状: 部分实装
- 范围: 把任意 MCP server 的工具注册为原生工具，Agent 无需感知 MCP 细节。
- 方案: core/src/mcp/* 已实现客户端；补 dispatch 桥接：MCP tool → 本地 ToolRegistry，加 wire-safe 参数映射 + 单测。
- 验证: cargo test --lib mcp + 工具注册计数断言；离线 mock MCP server 往返。
- 依赖/风险: 涉及 mcp 工具面与 schema 映射；注意红线「单调扩展工具面」。

### P0-3 OS 级沙箱
- 能力映射: dsh.sec.os_sandbox   |   现状: 部分实装
- 范围: Linux Landlock / macOS Seatbelt 内核级隔离；Windows 显式 Unsupported，命令级 sandbox/ 兜底。
- 方案: 复用 core/src/sandbox/*；为 linux/macos 补内核沙箱启用路径，Windows 保持只读降级如实上报；补平台分支测试。
- 验证: cargo test --lib sandbox + doctor 沙箱项分平台验证。
- 依赖/风险: 平台差异大；Windows 只能命令级兜底（现状符合安全约束）。

### P1-1 UIA 跨应用桥
- 能力映射: dsh.auto.uia   |   现状: 部分实装
- 范围: Win UIA 元素级跨应用：定位窗口句柄、读控件树、触发按钮/菜单；与 desktop_control（键鼠）组合。
- 方案: Rust 用 uiautomation crate 读控件树 + 触发；补齐 window/control 句柄封装；加 mock 控件树单测。
- 验证: cargo test --lib desktop_automation/uia + 驱动真实记事本冒烟。
- 依赖/风险: 仅 Windows 桌面安装包；需 uiautomation 依赖与受限环境。

### P1-2 音频理解
- 能力映射: dsh.vis.audio   |   现状: 纯叙事
- 范围: 语音/音频转写与理解。
- 方案: 引入 ASR 管线（whisper.cpp/侧车），提供 audio→text 工具；无后端时离线降级如实上报。
- 验证: cargo test --lib vision/audio 降级路径 + 真音频(可选)转写。
- 依赖/风险: 需要音频解码 + ASR 模型；体积/依赖大；先做降级与占位。

### P1-3 本地量化模型
- 能力映射: dsh.model.local   |   现状: 纯叙事
- 范围: 本地量化模型离线运行（离线可用）。
- 方案: 接入本地推理运行时（llama.cpp/Ollama 经 custom 接入点），作为 local 类模型档案；补离线路由与阈值。
- 验证: cargo test --lib model + 本地模型加载/离线回退。
- 依赖/风险: 需量化模型文件 + 推理运行时；默认不预置（D-03）。

### P1-4 Market 安装/升级
- 能力映射: dsh.market.install·upgrade   |   现状: 部分实装
- 范围: 按名称离线安装/升级 manifest + 缓存清理，原子切换，含 SKILL_SCAN 二次签名校验。
- 方案: 补 gate 内 market_install/upgrade 原子写 + 版本语义比较 + 缓存清理；加幂等与失败回退单测。
- 验证: cargo test --lib plugins/manifest + market 相关断言
- 依赖/风险: 涉及 plugins 清单安全（红线：上下文重注入/工具面替换守卫）。

### P1-5 swarm/集群/MAX 编排
- 能力映射: orch.swarm·cluster·dual_leader   |   现状: 部分实装
- 范围: DAG 并行 dispatch、fan-out 多 Worker 真实并行 spawn + 聚合、planner/executor 双轨。
- 方案: 复用 orchestration/dag.rs + dispatch_bloom_swarm/cluster；补 MAX 双带队两阶段顺序执行；加拓扑/聚合确定性单测。
- 验证: cargo test --lib orchestration + 路由/聚合离线闭环。
- 依赖/风险: 核心编排改动，风险高；需保住 2116 基线 + 扇出/屏障不回归。

### P1-6 语义/自进化记忆
- 能力映射: mem.semantic·evolutionary   |   现状: 部分实装
- 范围: 提炼知识片段 + 从经验沉淀可复用策略。
- 方案: 在 memory 层加 semantic/evolutionary 检索与写入单元；与 KB 去噪/知识图谱衔接；加存储/检索单测。
- 验证: cargo test --lib memory + 写入/检索往返。
- 依赖/风险: 记忆噪声 → 需去噪/完整性审计；避免污染（retrieval 质量随噪声下降）。

### P1-7 自动评审/验证器
- 能力映射: gov.review·verifier   |   现状: 部分实装
- 范围: 对 diff 产出结构化评审裁决(blocking/advisory+证据) + 按验收标准+可选校验命令判定（依赖 model.cloud）。
- 方案: 接 model.cloud 运行时；产出结构化裁决/达标判定；离线时降级上报（不做假成功）。
- 验证: cargo test --lib governance + 离线降级/结构契约；真实 LLM 手动验证。
- 依赖/风险: 依赖 LLM 后端；需诚实降级 + CompletionGate 对接。

### P2-1 结构化差异预览
- 能力映射: core.diff   |   现状: 部分实装
- 范围: 每次改动产出可读 diff，支持一键接受/拒绝。
- 方案: 复用 tools/diff（若有）产出 unified diff；补接受/拒绝动作 gate；加 diff 解析单测。
- 验证: cargo test --lib tools + diff 往返。
- 依赖/风险: 较低风险；主要是 UI 侧接受/拒绝。

### P2-2 文件监听
- 能力映射: tool.fs_watch   |   现状: 部分实装
- 范围: 工作区递归 notify（新增/修改事件经总线发布）；GUI 侧触发 Agent 反应待做。
- 方案: 复用 notify 事件总线；补工作区 watcher 事件下发 + 监听取消边界；加 notify 单测。
- 验证: cargo test --lib + workspace_watcher 相关。
- 依赖/风险: 事件量/清理；避免无限递归（忽略 git 目录已有）。

### P2-3 私有知识库
- 能力映射: kb.ima   |   现状: 部分实装
- 范围: 本地 RAG(BM25+向量) + cross_project 跨项目记忆 + 画像导入(memory.import.*)；原生批量导入 Wizard 待做。
- 方案: 补批量导入 Wizard（Rust 侧 ingest 批量 + 分块）+ 跨项目作用域；加导入/检索单测。
- 验证: cargo test --lib kb + 批量导入往返。
- 依赖/风险: 向量嵌入依赖；跨项目作用域与隐私。

### P2-4 Router 自动路由
- 能力映射: dsh.router.auto_route   |   现状: 部分实装
- 范围: 基于任务复杂度启发式选 plan/execute/weak profile（P3 启发式）。
- 方案: 在 model/router.rs 补复杂度启发式选择；加路由矩阵单测（对齐现有 t1-t12）。
- 验证: cargo test --lib model/router + routing_matrix。
- 依赖/风险: 与现路由规则一致性；避免破坏现有路由测试。

### P2-5 Modlens 版面分析
- 能力映射: dsh.vision.layout   |   现状: 部分实装
- 范围: 图片版面分区：标题/段落/表格/代码块/插图；供 extract_json 消费。
- 方案: 在 vision/modlens 补 layout 分区解析 + 降级；加分区 JSON 契约单测。
- 验证: cargo test --lib vision/modlens + 契约。
- 依赖/风险: 依赖 modlens/vision provider；无 provider 时降级（不造假）。

### P2-6 FPS 操作助手
- 能力映射: tool.fps_assistant   |   现状: 部分实装
- 范围: 基于 Computer Use 的射击操作代理（瞄头/压枪/跟枪/急停/搜点/扫车/一键拉枪；默认 8ms 步长 + humanize 防反作弊）。
- 方案: 复用 tools/computer_use + skills/fps.*；补 humanize 轨迹 + 步长配置；加纯逻辑单测（不真跑游戏）。
- 验证: cargo test --lib computer_use + 轨迹/步长。
- 依赖/风险: 需 Labs 显式开启；仅做逻辑层，不代打反作弊边界。

### P2-7 双模型配对
- 能力映射: model.dual   |   现状: 部分实装
- 范围: 规划/执行用不同模型（成本质量兼得）。
- 方案: 在 registry/router 补 copilot 双轨绑定 + 路由；加双模型选择/成本单测。
- 验证: cargo test --lib model + 双模型路由。
- 依赖/风险: 与 Bloom 副模型一致性。

### P2-8 DSH Bundle 拓扑加载
- 能力映射: dsh.bundle.topo_order   |   现状: 部分实装
- 范围: 依据 dependencies 做 Kahn 拓扑排序，弱依赖插件先注册。
- 方案: 复用 plugins/di.rs 的依赖解析；补 Kahn 排序纯函数 + 断言（对齐 di_load_resolves_dependency_order）。
- 验证: cargo test --lib plugins/di + 拓扑。
- 依赖/风险: 与现有 DI 拓扑一致。

### P2-9 Better Sidebar 打开路径
- 能力映射: dsh.sidebar.open_path   |   现状: 部分实装
- 范围: 侧边栏聚焦并展开指定 path 节点。
- 方案: 在 workbench/sidebar 补 open_path 侧效（聚焦+展开）；加布局/pin 单测。
- 验证: cargo test --lib workbench/layout + sidebar 相关。
- 依赖/风险: UI 层；注意槽位/pin 计数测试。

## 建议推进顺序
1. P0-1..3（浏览器/MCP 桥/OS 沙箱）——工具面与安全，优先级最高，先补工具注册与降级，风险可控。
2. P1-1..7（UIA/音频/本地量化/Market/编排/记忆/评审）——能力主战场，逐项专项。
3. P2-1..9——体验/质量改进，可与 P1 穿插。

> 每项落地遵循既定标准：cargo check + clippy 零新增 + 全量 cargo test(基线 2116 不回退) + 运行验证；改动前先读 docs/cli-reference.md、mcp.md、development.md。