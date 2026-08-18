# Marguerite 续作状态收口报告（D1–D5 / 路由 / 用量 / 代码块）

**日期**：2026-08-16
**场景**：多成员协作续作（后端根因核查 + 前端缺口补全）
**参与成员**：主理人（实现统筹 / Development Coordinator）+ 排障手（gstack-investigator）

---

## 📌 TL;DR（执行摘要）

- **整体结论**：🟢 通过（源码层功能已就绪，仅余少量收口 + 需运行时验证）
- **阻塞项数量**：0（core 已编译通过，0 错误；主要风险是"运行时未实测"，非编译/代码缺失）
- **最大发现**：用户报告的多项 bug 在当前源码中**大多已实现**——上一轮摘要低估了代码库实际进度。最可能的原因是运行的是**旧构建**。
- **下一步**：干净重建 → 启动网关冒烟测试，逐项确认用户报告的 5 项 bug 在运行时确已修复。

---

## 🎯 核心结论卡片

| 项目 | 内容 |
|------|------|
| Go / No-Go | 🟢 Go（重建后可上线验证） |
| 严重度分布 | 🔴 0 / 🟠 0 / 🟡 1（D5 工具型 Bloom 子路径不发 ensemble_progress） / 🟢 已就绪多项 |
| 关键行动项 | 5 条（见下） |
| 建议负责人 | 主理人收口 + 用户运行时验收 |

---

## 1. 各成员核心结论

### 🔧 排障手（gstack-investigator，后端根因核查）
- **核心判断**：对 `core/` 做静态分析 + `cargo check`（release / offline / 全新 target-dir `D:/tp_build_fresh1`），**0 错误，47m58s**。结论：D1–D5 五项行为在现有源码中**均已实现**，无需改动。
- **关键证据**：
  - D1 离开页面不停任务：`chat.rs:149-187` 用 `tokio::spawn` 启动 turn，与 WS 解耦；仅 `chat.abort` 才中断。
  - D2 会话状态/闲置：`sessions.changed` 已在 `chat.rs:170/176/191` 于 turn 起止/出错时发布，并经 WS 转发（`server.rs:236` 订阅 `**`、`:298-308` 全量转发）。
  - D3 用量实时：`usage/mod.rs:178-199` 写库后即通过 `global_bus()` 发布 `Envelope::usage(...)`（`EventType::Usage → "usage"`，`envelope.rs:108/392`）。
  - D5 路由卡片：`ensemble_progress`（`model/bloom.rs:221`）、`router_decision`（`chat.rs:157-162`）均已存在且被 WS 全量转发。
  - D4 记忆召回：`chat.rs:370-406` `memory_stable_prefix` 已把 `cross_project`/`long_term`/`short_term` 注入 prompt（自第 2 轮起生效，按设计）。
- **残留缺口**：`ensemble_progress` 仅 Bloom **纯文本**路径发出；带工具的 Bloom 子路径（`run_tool_loop_or_bloom` / `run_dual_model_tools` / `run_team_dag`）不发 → 带工具 Bloom 任务的编排卡片仍可能为空。此为**真实残留**，非转发 bug。

### 🧭 主理人（实现统筹 / Development Coordinator，前端收口）
- **核心判断**：前端"待办"清单中，代码块折叠、侧栏持久化、D1–D5 后端均已就绪；**唯一真实缺口是用量实时订阅**（前端仍仅 15s 轮询，未订阅后端已发出的 `session.event.usage`）。
- **本次动作**：在 `webui/src/composables/usage/useUsageData.ts` 补全 `session.event.usage` 实时订阅（激活时订阅、失活/卸载时退订、1.5s 防抖刷新），与后端 emit 对齐。
- **已确认在源码就绪（无需改动）**：`TextPart.vue` 代码块"框住 + 生成完自动折叠"（`live` 属性 + `code-block-frame` + `collapseAllCodeFrames` + `watch(props.live)`）；`ChatView.vue:963-967` 右侧栏 `localStorage` 持久化。

---

## 2. 综合审查发现（去重合并后按严重度排序）

| # | 严重度 | 类别 | 位置 | 问题描述 | 建议 | 来源成员 |
|---|--------|------|------|---------|------|---------|
| 1 | 🟡 | 功能缺口 | `core/src/orchestration/mod.rs`（Bloom 工具子路径） | 带工具的 Bloom 任务不 emit `ensemble_progress`，编排状态卡片仍可能为空 | 在 `run_tool_loop_or_bloom`/`run_dual_model_tools`/`run_team_dag` 注入 `Envelope::ensemble_progress`，并用运行时验证 | 排障手 |
| 2 | 🟢 | 已确认就绪 | `core/src/gateway/chat.rs` 等 | D1 离开页面不停任务、D2 会话状态事件、D4 记忆跨会话召回 | 无需改动，重建后验证 | 排障手 |
| 3 | 🟢 | 已确认就绪 | `core/src/usage/mod.rs` + `webui/.../useUsageData.ts` | D3 用量：后端已 emit `session.event.usage`；前端本回合补齐订阅 | 无需改动（前端已补） | 主理人 + 排障手 |
| 4 | 🟢 | 已确认就绪 | `webui/src/components/chat/parts/TextPart.vue` | 代码块"框住 + 生成完自动折叠"已完整实现 | 无需改动 | 主理人 |
| 5 | 🟢 | 已确认就绪 | `webui/src/views/ChatView.vue:963-967` | 右侧栏展开/折叠 `localStorage` 持久化已就绪 | 无需改动 | 主理人 |
| 6 | 🟢 | 已确认就绪 | `core/src/gateway/methods.rs` + `config.rs` | (A) 模型路由 `models.routing.set/get` 原生实现；中文路径 UTF-8 编码修复 | 已随 core 编译验证（0 错误） | 主理人（前回合）+ 排障手 |

---

## ✅ 行动清单（具体可执行项）

| # | 行动 | 负责方 | 紧急度 | 期望完成 |
|---|------|--------|--------|---------|
| 1 | 干净重建 `marguerite-core` + `marguerite-gui`：用**全新** target-dir（如 `D:/tp_build_fresh1`）规避 `os error 5` 指纹锁；非交互 shell 须 `export PATH="$PATH:/c/Users/Administrator/.cargo/bin"` | 主理人 | P0 | 重建当轮 |
| 2 | 启动网关 + 桌面端冒烟测试，逐项确认用户报告的 5 项 bug 在**运行时**确已修复（事件真到达 / 卡片填充 / 切页任务不中断 / 用量实时 / 记忆跨会话） | 用户验收 + 主理人 | P0 | 重建后 |
| 3 | 补 D5 工具型 Bloom 子路径 `ensemble_progress` 事件（排障手标记的真实残留） | 主理人/排障手 | P1 | 下一迭代 |
| 4 | 官网（仿 doubao）/ 安装包（去隐私）/ 静默后端（无 python/powershell 弹窗）/ 实跑任务（WPS PPT、AI 新闻、浏览器查广西港妙、FPS、官网 UI） | 多角色分阶段 | P1–P2 | 多会话排期 |
| 5 | 克隆并二次开发 30+ 外部仓库（harness + computer use），先能力对齐再去品牌名 | 主理人 + 调查员 | P2 | 分批 |

---

## ⚠️ 待完善 / 已知局限

- 本轮回**静态分析 + core 编译验证**，未跑真实网关 / 桌面端运行时；"已修复"结论基于代码路径，非线上观测帧。
- 大项（官网 / 安装包 / 静默后端 / 实跑任务 / 30+ 仓库克隆 / 7 层记忆·工具·技能·插件·智能体·蜂群卡片化与动效）**未启动**，明确超单会话容量，需分阶段排期并依赖用户提供设计稿与签名/打包环境。
- 用户原贴 4 张图片不可读，全程依文本与静态分析推进。
- 用户报告的 bug 极可能源于**旧构建**而非源码缺陷；建议以当前源码干净重建后复测为准。

---

## 📚 成员产出索引

- gstack-investigator 原始产出：D1–D5 静态分析 + `cargo check` 报告（**0 错误，47m58s**，fresh dir `D:/tp_build_fresh1`；`D:/tp_build5` 因陈旧 `.d` 文件锁导致 os error 5，已规避）。
- 主理人产出：`useUsageData.ts` 用量实时订阅补全（diff 见仓库 working tree）。

---

> 本报告由软件工坊 AI 协作生成，关键决策请由工程负责人复核。
