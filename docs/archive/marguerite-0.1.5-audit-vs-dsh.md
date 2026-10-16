# Marguerite 0.1.5 能力/架构探查 + 与 DSH 对比改进报告

日期: 2026-08-22  目标: marguerite/release/Marguerite_0.1.5_x64-setup.exe + 源码 D:\Downloads\Marguerite  对比: http://127.0.0.1:4567/ + dataelement/dsh-desktop + deepseek-harness

## 0. 视觉约束说明
本会话模型 deepseek-v4-flash 为 text-only，且 modlens/claude-cli/kimi-cli 均回退到该 text-only 模型，无法读像素。故采用: ① 运行产品 CLI 获取真实操作证据; ② 源码/目录审计; ③ 截图像素分析(深色主题)。下述结论均来自真实运行输出与源码，非臆测。

## 1. 实测证据 (真实运行)
### 1.1 marguerite-core doctor
- Rust x86_64 / Windows; 配置目录 ~/.marguerite; 主模型 deepseek-v4-pro; MCP 服务器 0; engine tools=63, research_docs=0, automations=10; sandbox 只读降级(Unsupported) → OS 级沙箱在本平台未生效; 自检 6/6 通过。

### 1.2 marguerite-core capabilities (能力/现实缺口 — 最具价值)
- 全量 89 | 已实装 83 | 纯叙事 6 | 分级: 独家37/增强41/基线11。
- 纯叙事/零实现(6): 2048 数字合成、井字棋 TicTacToe、贪吃蛇 Snake、UIA 桥、音频理解、本地量化模型。
- 部分实装(约 23): 结构化差异预览、Better Sidebar 打开路径、Games 启动入口、MAX 双带队、集群调度、swarm 有向无环图、DSH Bundle 拓扑加载、Router 自动路由、浏览器自动化、文件监听、MCP 工具桥、Market 插件安装/升级、私有知识库、OS 级沙箱、FPS 操作助手、UIA 跨应用桥、自动修复、Modlens 版面分析、自动评审、验证器、Usage 余额查询、自进化闭环、双模型配对、语义记忆、自进化记忆。

### 1.3 skills list → 未找到技能(目录: skills)
- 便携包内实际打包 22 份技能 YAML(commit/doc/explain/fix/refactor/test + superpowers 全家 + dsh-market 四件套)，但从任意 CWD 运行 CLI 均返回 0 → 技能发现按 CWD 查找，未默认到打包/配置目录。

### 1.4 model list / 模型后端
- 模型档: deepseek-chat / deepseek Cloud + custom openai-compatible gpt-4o-mini → 模型后端面窄(未覆盖 deepseek-v4 系列 / 云端多 provider)；主模型 deepseek-v4-pro 为本地路由默认。

### 1.5 前端与打包
- webui/: Vue3+Vite+Pinia+vue-i18n，重视无障碍(aria-label/inert/role=tooltip/kbd 快捷键/移动抽屉)，且有大量单测(artifactItems/layout/runtime/store .test.ts) → 源码质量高。
- gui/: Tauri+React(仅 App.tsx/main.tsx/theme.css) + src-tauri(内含 wix/ 目录)。
- 便携包 release/Marguerite-0.1.5-dev-win-x64-portable/: bin(core/gui/tui 三二进制) + webui(含 assets/img/music/wallpapers) + skills(22) + plugins/manifests(16) + modes + docs + config.yaml + 两个启动 bat。
- 安装器: Marguerite_0.1.5_x64-setup.exe(249MB) / x64 en-US MSI(266MB)；另有 0.1.4 旧件与 v22–v26 多代 desktop/tui 二进制 → 版本/产物存在多重代(0.1.4 vs 0.1.5-dev, README 0.1.4 vs changelog 0.1.5-dev)。

## 2. 改进点 (P0→P2)
### P0 (正确性/交付阻断)
1. 消灭能力/现实缺口：capabilities 自检已诚实标记 6 项纯叙事 + 23 项部分实装。Games 列表展示 2048/TicTacToe/Snake 但零实现 → 用户可见死入口。建议: 未实现项从产品面移除或置灰+标注(规划中)，避免有功能但点不动。
2. 修复技能发现路径：skills list/技能 JIT 触发应从打包 skills 目录与 `~/.marguerite/skills` 与 `third_party/skills/` 默认加载，而非 CWD；否则开箱即用的技能不可达。

### P1 (体验/完整性)
- 把部分实装管线化：优先完成差异化项——浏览器自动化、UIA 跨应用桥、桌面 Computer-Use、swarm DAG / 集群调度 / MAX 双带队(编排)、MCP 工具桥(MCP=0)、Market 安装/升级(商店闭环)、OS 沙箱(doctor 显示只读降级)、双模型配对。
- 模型后端扩展：对齐 DSH deepseek-v4 系列 + 云端多 provider(deepseek-official/anthropic/openai)，当前仅 deepseek-chat+custom gpt-4o-mini。
- 能力可见性：doctor 显示 MCP=0、sandbox 只读降级 → 应在 WebUI/TUI 增加能力健康度指示，避免把未生效能力当成就宣传(与 4567 DSH 相同痛点)。

### P2 (质量/对比)
- 版本一致性门禁: 统一 README/CHANGELOG/wxs/产物版本号(0.1.4 vs 0.1.5-dev)，避免多代二进制与安装器并存。
- 保留优势: 单 Rust 二进制+离线编排+回环托管(安全不变量)+三前端+诚实 capability 注册表；这些都是对 DSH 的差异化。

## 3. 与 DSH(4567) 及上游对比
| 维度 | 4567 DSH Web | Marguerite (本项目) | 上游 dataelement/dsh-desktop, deepseek-harness |
|---|---|---|---|
| 形态 | 插件主机 Web | 终端优先 Rust 引擎 + TUI/Tauri/Vue3 | Desktop for DSH; Everything is a Plugin |
| 引擎 | Node/plugin 栈 | Rust 单二进制(离线可用) | Node/plugin |
| 视觉/交互 | modlens+GenUI(真) | 图像理解/Modlens(部分) | 继承 DSH |
| 编排 | agent_teams/swap/plan 真 | 专家团队/集群/swarm(部分) | 继承 DSH |
| 记忆 | hindsight/openviking(需密钥) | 三层记忆(语义部分) | 继承 DSH |
| 能力自检 | 无显式 | capabilities 89 项自检(独有) | 无 |
| 安全 | 审批/沙箱多级 | 回环+PlanGate/CompletionGate | 继承 DSH |
对比结论: Marguerite 的优势是单二进制离线 + 三前端 + 诚实能力自检 + 回环安全；劣势是大量部分实装/纯叙事 + 技能发现路径 + 模型面窄。相对 DSH 的成熟插件生态，Marguerite 的 DSH-fusion 更偏复刻骨架，需把 23 项部分实装真正落地。

## 4. 建议开发改进动作 (优先级)
1. 修 skills 发现默认路径(打包+配置目录)，补 1 条单测。
2. gates: capability 自检中 纯叙事/部分实装 项在 UI 置灰 + 标注，避免死入口。
3. 扩充模型 provider 表(deepseek-v4/cloud/anthropic/openai) + 路由。
4. 把 browser/UIA/Computer-Use、MCP 桥、Market 安装、swarm/集群 从部分实装推进到已实装。
5. 增加能力健康度/未配置即提示横幅(参考 4567 的痛点)。
6. 统一版本号与产物矩阵。

> 说明: 真正再开发改进(改 React/Vue/Rust 并重编译)需在源码分支 + cargo/npm 构建管线内迭代；本报告给出经真实验证的问题清单与优先级路径。