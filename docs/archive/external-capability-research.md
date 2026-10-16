# Marguerite 外部项目能力调研报告与分阶段升级开发计划

> 调研日期：2026-08-10 ｜ 项目：Marguerite CLI Workbench（Rust core + egui GUI + Tauri + webui + axum WS 网关）
> 调研对象：ds4、deepseek-tui、claude-code-plugins、agent-s、hermes-agent、openclaw、opencode、claude-mem、claude-code-source
> 维度：CLI / TUI / GUI / 桌面端 / 网页端 的能力与架构逻辑，并结合 Marguerite 现有 UI 及能力对比。

---

## 一、九个外部项目能力总览

| 项目 | 语言/运行时 | 核心形态 | 能力要点 | 与 Marguerite 的相关性 |
|---|---|---|---|---|
| **ds4**（antirez/ds4） | C / Metal | CLI + HTTP 推理服务器 | DeepSeek V4 Flash 本地推理引擎；OpenAI/Anthropic 兼容 API；磁盘 KV 缓存跨重启续会话；DSML 原生工具格式；Think/Think-Max 三档思考；REPL（`/think /ctx /read`） | 可作为 Marguerite 的本地推理后端（侧车替代/补充 pyengine）；KV 磁盘缓存思想可借鉴到会话持久化 |
| **deepseek-tui**（CodeWhale 系，Rust） | Rust（14 crate workspace） | TUI（ratatui）+ HTTP/SSE 运行时 API | Plan/Agent/YOLO 三模式；1M 上下文跟踪+手动/配置压缩；`checklist_write`/`update_plan` 计划工具；侧 git 前后快照回滚（不碰 .git）；子代理；MCP；持久任务队列；`deepseek serve --http` headless；Session save/resume | **YOLO 模式已映射到 Bloom**；上下文压缩、计划工具、侧 git 快照是重点借鉴对象 |
| **claude-code-plugins**（anthropics 官方） | Markdown/TS/Shell | CLI 插件生态 | 标准插件结构 `.claude-plugin/plugin.json` + commands/agents/skills/hooks/.mcp.json 自动发现；`feature-dev` 7 阶段工作流；`code-review` 5 并行 Sonnet 代理；hookify 规则钩子 | 插件目录即能力面，可参考为 Marguerite 的 skills/hooks 标准化结构 |
| **agent-s**（simular-ai） | Python | GUI 计算机使用框架 | Worker/Grounding/Knowledge/Manager 四组件；Mixture-of-Grounding 视觉定位（UI-TARS/OSWorldACI，描述→坐标）；Proactive Hierarchical Planning（多时间尺度动态细化计划）；pyautogui 执行；OSWorld SOTA 72.6%；Windows/macOS/Linux | **视觉 grounding「截图→坐标」与轨迹反思是 Bloom 视觉操控的关键参考**；分层规划补充 plans.* |
| **hermes-agent**（NousResearch） | Python 3.11+ | CLI(TUI)+Gateway(20+平台)+Electron 桌面 | 同一 AIAgent 核心跨 CLI/TG/Discord/Slack/TUI/桌面复用；`context_compressor`（有损摘要）+ prompt caching；SQLite+FTS5 会话；Honcho 跨会话记忆；70+ 工具 28 toolsets；5 终端后端；插件+skills 扩展；cron 调度；ACP server（IDE 集成）；CDP 连活 Chrome；PII 脱敏 | 多通道网关架构与 Marguerite 网关同构；上下文压缩与跨会话记忆实现范式可对照 |
| **openclaw** | 多语言（Node/Python/Go） | 桌面 daemon + CLI + 浏览器扩展(MV3 Relay) + 聊天网关 | 24/7 自主 Agent；Browser Relay 经 CDP 连**用户真实 Chrome 会话**（绕反爬/免登录态）；snapshot-ref 页面快照+act；持久记忆（memsearch：SQLite+向量混合检索）；skills 自生成；邮件/日历/家居等 50+ 集成 | **snapshot-ref 快照+act 是浏览器非视觉通道的范式**；记忆系统与跨项目记忆对照 |
| **opencode**（sst/anomalyco） | TypeScript/Bun | TUI（OpenTUI）+ 桌面 + Web，client-server | 核心 server 进程 + 多客户端（TUI/桌面/Web 同一后端）；`.opencode/` 配置即代码（agents/commands/skills/plugins 类型化）；build/plan 双 Agent Tab 切换；多模型路由（命令级选模型）；session SQLite 本地持久；LSP 反馈；Git 快照 undo/redo；MCP | **client-server 多面复用架构与 Marguerite 网关高度一致**；`.opencode/` 配置范式可迁移到 `.marguerite/` |
| **claude-mem**（codenamev/claude_memory） | Node.js + SQLite | CLI 插件 + Worker + Web UI + MCP server | Hook 事件驱动捕获（PreToolUse/PostToolUse/SessionStart 等）；三层记忆（summary→timeline→observation）；sqlite-vec KNN + 本地 embedding（fastembed-rb）；渐进式披露；隐私标签（`<private>`）；React Web 可视化（端口 37777） | **跨项目会话记忆的完整范式**：Hook 捕获→多粒度摘要→向量检索→渐进披露 |
| **claude-code-source**（anthropics） | TypeScript 512k 行 / Bun | CLI TUI（React+Ink 自研 fork） | QueryEngine（46k 行）会话引擎；**5 层错误恢复**；**上下文折叠/微压缩/自动压缩**三级压缩；40+ 工具；100+ 命令；Coordinator 多代理任务系统；memdir 持久记忆；hooks 生命周期；逐工具权限模型；Bridge（IDE 集成）；MCP 8 种传输；SQLite/Drizzle 状态 | 压缩与恢复机制、任务系统、权限模型是「业界黄金标准」，Marguerite 对齐的最终参照 |

---

## 二、CLI / TUI / GUI / 桌面 / Web 五维对比

| 项目 | CLI | TUI | GUI | 桌面端 | 网页端 |
|---|---|---|---|---|---|
| ds4 | ✅ `ds4 -p` 一次性 + REPL（/think /ctx /read） | ✅ 交互 REPL | ❌ | ❌ | ❌ |
| deepseek-tui | ✅ `deepseek` 分发 CLI + `deepseek serve --http` | ✅ ratatui 全功能（聊天/状态栏/工具展示） | ❌ | ❌ | ❌（HTTP/SSE 可被任何 Web 客户端消费） |
| claude-code-plugins | ✅ 斜杠命令注入 CLI | 随 Claude Code | ❌ | ❌ | ❌ |
| agent-s | ✅ `agent_s --provider ...` | ❌ | ✅ 屏幕截图观测 | ✅ Windows/macOS/Linux 桌面操控 | ❌（云托管 Simular Cloud 是 Web 入口） |
| hermes-agent | ✅ Rich CLI | ✅ TUI | ❌ | ✅ Electron 桌面壳 | ❌（Gateway 平台适配器是消息面） |
| openclaw | ✅ `openclaw` CLI + onboard | ❌ | ❌ | ✅ 桌面 daemon + 浏览器扩展 Relay | ✅ Web 控制面板（记忆可视化等） |
| opencode | ✅ YOLO 安装 + 命令 | ✅ OpenTUI（React/SolidJS/Vue 组件） | ❌ | ✅ 桌面 App（dmg/exe/deb） | ✅ Web 客户端连同一 server |
| claude-mem | ✅ `claudemem search/browse` CLI | ❌ | ❌ | ✅ Worker 常驻（37777） | ✅ React Web 记忆可视化 |
| claude-code-source | ✅ Commander.js CLI | ✅ React+Ink 自研 TUI | ❌ | ❌ | ❌（Bridge 供 IDE 前端） |
| **Marguerite 现状** | ✅ `marguerite` CLI + REPL（cli/repl.rs） | ✅ egui TUI（tui/） | ✅ Tauri GUI + WebView | ✅ marguerite.exe 桌面单文件 | ✅ webui（Vue，连接 WS 网关 18790） |

**架构结论**：
1. **client-server 单一后端 + 多前端复用**是业界主流（opencode/hermes/deepseek-tui serve），Marguerite 的「Rust 网关(axum WS) + GUI/TUI/CLI/webui 多前端」已符合此范式，无需推翻。
2. **TUI 化 GUI**（egui 渲染 IDE 式工作台）是 Marguerite 的差异化优势，deepseek-tui/opencode 的键盘驱动交互（Tab 切换 build/plan、`Shift+Tab` 切模式、reasoning-effort 循环）可直接映射为 Marguerite 的快捷键体系。
3. 外部项目的 GUI/Web 普遍弱于 TUI/CLI；Marguerite 已具备最强的「桌面单文件 + WebView」形态。

---

## 三、与 Marguerite 现状对比：核心缺陷差距

### 缺陷 1：上下文管理与上下文压缩机制

| 参照 | 机制 | Marguerite 现状（context/session.rs） | 差距 |
|---|---|---|---|
| claude-code | 上下文折叠→微压缩→响应式压缩→自动压缩，4 层渐进 | 确定性压缩：token 估算（bytes/3）+ 结构化摘要（目标/进展/决策/下一步/文件）+ KEEP_RECENT_TURNS=8 + 自动触发 `session.event.compaction` | 缺 LLM 语义压缩；缺压缩前折叠；缺压缩质量评估 |
| deepseek-tui | 1M 上下文 + 手动/配置压缩 + prefix-cache 遥测 | 同上 | 缺手动触发与预算配置界面 |
| hermes-agent | context_compressor 有损摘要 + prompt caching | 同上 | 摘要维度单一，无缓存优化 |

**差距结论**：确定性压缩已交付并可工作，但对比业界「4 层渐进压缩 + LLM 语义摘要 + 折叠」仍有明显差距；需升级。

### 缺陷 2：跨项目会话记忆功能

| 参照 | 机制 | Marguerite 现状（memory/cross_project.rs） | 差距 |
|---|---|---|---|
| claude-mem | Hook 捕获行为事件 → 三层记忆（summary/timeline/observation）→ sqlite-vec 向量 KNN + 本地 embedding → 渐进披露 → Web 可视化 | 检索 `~/.trae-cn/memory/projects/*/project_memory.md + topics.md`，n-gram TF 余弦相似度，密钥脱敏 [REDACTED] | 缺向量语义检索；缺行为事件自动捕获（只有文件级检索）；缺渐进披露 |
| hermes-agent | Honcho 跨会话记忆 + PII 脱敏 | 同上 | 同上 |
| openclaw | memsearch：SQLite + 向量混合检索，可跨会话 | 同上 | 同上 |

**差距结论**：跨项目记忆基础检索已交付，但语义层面（向量）与捕获层面（行为事件）均未达 claude-mem 水平。

### 缺陷 3：长难复杂任务的自动拆解与规划能力

| 参照 | 机制 | Marguerite 现状（plan/mod.rs） | 差距 |
|---|---|---|---|
| agent-s | Proactive Hierarchical Planning：多时间尺度动态细化计划，基于观测反馈 | PlanStore（revision/run 状态机）+ plans.analyze/create/list/run/cancel + decompose 复用 Orchestrator | 缺动态重规划（计划不随观测更新）；缺执行反思 |
| claude-code | Coordinator 多代理任务系统 + task 工具（并行子任务） | 同上 | 缺并行子代理执行与结果聚合 |
| deepseek-tui | `checklist_write`/`update_plan` 让模型在动作前先列清单 | 同上 | 缺执行前清单化强制 |
| opencode | build/plan 双 Agent 分离 | 同上 | 缺 plan/build 双轨 |

**差距结论**：规划状态机已交付，但缺「动态重规划 + 执行前清单 + 并行子任务」三件套。

---

## 四、需修复 Bug 的处置结论

| # | Bug | 状态 | 说明 |
|---|---|---|---|
| 1 | 实时更新不可用（历史可看无法发送） | ✅ 已修复（上一轮） | `py_call_timeout`(3s) 根治 Python 侧车慢调用阻塞 WS 事件循环；`sessions.messages.subscribe/snapshot/hydrate` 三件套 Rust 实现；心跳保活；真实网关验证通过 |
| 2 | `sandbox.policy.get` 未在 Rust 网关实现 | ✅ 已修复（上一轮） | `sandbox/policy_store.rs` Rust-first 实现 get/defaults/update + run_mode.*，版本冲突 `POLICY_VERSION_CONFLICT` 携带 currentPolicy；测试覆盖 |
| 3 | Agent 无法联网获取实时信息（知识截止） | 🔲 规划中 | 需升级 `web_search`/`web_fetch` 工具为 Agent 常用工具并确保模型工具清单暴露；规划：联网检索工具链（web_search 多引擎 + fetch_url 正文提取）接入 Bloom 门控 Network 类 |

---

## 五、Bloom 模式能力聚合差距

| 聚合项 | 参照 | Marguerite 现状 | 差距 |
|---|---|---|---|
| GUI_Agent 的 MAX 模式 | 全开免确认（skip_permissions） | `BloomPrivileges.skip_permissions` + `enable-auto-mode` + `allowed_tools`，`gate()` 已实现 | ✅ 已具备 |
| deepseek-TUI 的 YOLO 模式 | auto-approve 全自动 | `ModeKind::Bloom`（yolo 为别名） | ✅ 已具备 |
| opensquilla 的 AI 智能融合路由 | 多引擎 router.decision | 真实网关 `router.decision` 融合路由已验证（0ms 返回 MAX Dual-Leader/Solo Agent） | ✅ 已具备 |
| 内置 Agent 可控浏览器（非视觉） | openclaw snapshot-ref / GUI_Agent browser_bridge a11y | `browser_tool`：open/text/links/html/evaluate/click/type/scroll（DOM 通道）+ 有状态会话 | 🔲 **缺 a11y 语义快照（Accessibility.getFullAXTree）与 ref 定位点击** |
| 内置 Agent 可控浏览器（视觉） | agent-s Mixture-of-Grounding / GUI_Agent browser_real_control VLM | `browser_tool` screenshot + `VisionCapability`/`LlmVisionTransport`（OpenAI 兼容多模态） | 🔲 **缺「截图→VLM 坐标定位→坐标点击」闭环** |
| 第三方软硬件/系统级鼠标键盘 | GUI_Agent 三级回退链 / pyautogui | `system_control_tool`（Windows user32 P/Invoke：鼠标/滚轮/按键/剪贴板文本） | ✅ 已具备（仅 Windows） |

**结论**：Bloom 五合一中四合一已具备，浏览器双通道（a11y 非视觉 + VLM 视觉）是唯一实质性缺口 → 本轮实现。

---

## 六、分阶段升级开发计划

### Phase B1：浏览器双通道接入 Bloom（本轮交付）

**交付目标**：`browser_tool` 新增非视觉 a11y 语义快照与视觉坐标点击，构成「视觉 + 非视觉」双通道；经 Bloom 门控可被 Agent 调用。

**技术方案**：
1. `browser/cdp.rs` 新增 `Accessibility.getFullAXTree`、`DOM.getBoxModel`、`Input.dispatchMouseEvent` 构造器与响应解析。
2. `browser/mod.rs` `CdpSession` 新增 `a11y_tree()`（返回 role/name/ref/bbox 精简快照）与 `click_at(x,y)`（Input 鼠标事件）。
3. `vision` 新增结构化定位：`VisionTransport::locate()`（默认 None，LlmVisionTransport 实现为「截图→`{"x","y","confidence"}` JSON」），`VisionCapability::locate()` 门面，优雅降级。
4. `tools/builtin.rs` `browser_tool` 新增 action：`a11y`（快照）、`click_ref`（ref→坐标→点击）、`vision`（截图→描述）、`vision_click`（截图→VLM 坐标→点击）；`browser_tool` 接收 `VisionCapability` 注入。
5. `mode/mod.rs` `classify_tool` 已归 `browser`/`browser_a11y`/`see` 为 Network，Bloom 下 auto_approve 放行；无需改门控。
6. 新增单元测试（cdp 构造器/解析、坐标映射、VLM JSON 解析）+ 质量门禁（build 零警告 / clippy -D warnings / 全量测试）。
7. 真实网关冒烟验证：a11y 快照读取 + 截图描述 + 视觉坐标点击 + Bloom 门控放行。

**验收**：`cargo build --workspace` 零错误；`cargo clippy -D warnings` 全绿；`cargo test --workspace` 全绿（含新增）；真实网关 e2e 冒烟通过浏览器双通道。

### Phase B2：上下文压缩升级为「4 层渐进压缩 + LLM 语义摘要」

**交付目标**：把确定性压缩升级为业界级压缩链，显著降低长会话成本、提升压缩后可用性。

**技术方案**：
1. 压缩前折叠：对超长工具结果/代码块先行截断折叠（snip 语义：保留头尾）。
2. LLM 语义摘要：`context_compact` 新增 LLM 分支（复用 Orchestrator 或 LlmVisionTransport 同款通道），按「目标/进展/决策/下一步/文件」维度生成摘要；无 LLM 时回退现有确定性摘要（优雅降级）。
3. 自动压缩决策：token 预算（默认 128k）到达阈值（COMPACT_RATIO）时触发，保留最近 8 轮原文。
4. 压缩质量评估：摘要信息量评分 + 用户可一键 `/compact undo`（保留预压缩快照）。
5. webui/TUI 侧：`session.event.compaction` 事件已接，补手动压缩按钮与预算配置。

**验收**：长会话（>预算）压缩后 token 减少 ≥85% 且关键决策可回溯；LLM 摘要与确定性摘要双路径测试覆盖。

### Phase B3：跨项目记忆升级为「向量语义 + 行为事件捕获 + 渐进披露」

**交付目标**：从文件级关键词检索升级为 claude-mem 级跨会话记忆。

**技术方案**：
1. 向量语义层：本地 embedding（n-gram 哈希向量已具备基础，可升级为确定性 TF-IDF 向量 + 余弦；可选 sqlite-vec/llama.cpp 内嵌 embedding）→ 语义召回。
2. 行为事件捕获：在 `chat.rs` send/run_turn、工具执行（edit/exec/web_fetch/browser 等）埋点，按会话聚合为「summary→timeline→observation」三层。
3. 渐进披露：检索结果先给标题级摘要，命中后展开详情（防上下文爆炸）。
4. PII 脱敏保持（[REDACTED]）。
5. webui：记忆检索面板（claude-mem Web 可视化对照）。

**验收**：跨项目检索语义命中率提升（对比 n-gram 基线）；行为事件自动沉淀；脱敏策略测试覆盖。

### Phase B4：规划升级为「执行前清单 + 动态重规划 + 并行子任务」

**交付目标**：补齐长难任务自动拆解三件套。

**技术方案**：
1. 执行前清单化：`plans.run` 前置 `checklist` 生成（模型输出步骤清单 → 逐项勾选）。
2. 动态重规划：每步执行后对比计划与观测，差异超阈值时触发 `plans.replan`（对齐 agent-s Proactive Hierarchical Planning）。
3. 并行子任务：`plans.run` 支持 fan-out 子 Agent（复用 Orchestrator 的 sub-agent 通道）并行执行独立子任务并聚合结果。
4. webui plans 面板：展示 checklist 勾选进度、重规划 diff、子任务并行状态。

**验收**：多步骤复杂任务端到端自动拆解执行；重规划路径测试覆盖；并行子任务收敛。

### Phase B5：联网实时信息接入（Bug 3 收尾）+ 浏览器外扩通道

**交付目标**：Agent 具备实时联网能力；浏览器通道支持连接用户真实 Chrome（openclaw Relay 范式）。

**技术方案**：
1. `web_search` 多引擎（现有实现确认覆盖常见引擎）+ `web_fetch` 正文提取（readability 风格）→ 确保在模型工具清单首屏暴露；Bloom 下 Network 自动放行。
2. 浏览器 Relay 扩展（MV3）：把 Marguerite 网关 CDP 桥接到用户真实浏览器会话（登录态复用），作为 headless 通道的补充。
3. `system_control` 扩展非 Windows（macOS CGEvent / Linux XTest）按需。

**验收**：实时查询可检索当日信息；真实浏览器会话操控冒烟通过。

---

## 七、附：参考来源

- antirez/ds4（GitHub）
- staryxchen/DeepSeek-TUI、Hmbown/CodeWhale（GitHub，14 crate workspace 架构）
- anthropics/claude-code（plugins/ 目录 + SOURCE_ARCHITECTURE_ANALYSIS）
- simular-ai/Agent-S（S1/S2/S3 论文与代码，OSWorld 72.6%）
- NousResearch/hermes-agent（architecture.md、AGENTS.md）
- openclaw（agentopenclaw.io、Browser Relay MV3 说明、memsearch）
- sst/anomalyco opencode（aiwiki、datacamp、developersdigest 综述）
- codenamev/claude_memory、hms-claude-mem、zilliztech/memsearch ccplugin
- Claude Code 源码分析（zazaxbt/claude-code-breakdown、anthropics PR #41589）
