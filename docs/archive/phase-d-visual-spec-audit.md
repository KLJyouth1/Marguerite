# Phase D — pages/ 5 稿逐像素视觉规范比对报告

> 生成日期：2026-08-08
> 比对源：`D:\Downloads\Marguerite\pages\` 下 5 份 HTML 设计稿（仓库外同级目录，仓库内 `find` 漏掉，本轮用户指正后定位）
> 比对对象：桌面端 `desktop/src/theme.rs` + 各面板实现；TUI 端 `core/src/tui/workbench/theme.rs`

---

## 0. 采样说明（重要前提）

- 5 份稿真实路径在 **`D:\Downloads\Marguerite\pages\`**（不在 `marguerite/` 仓库内），此前误判为 `pages/` 不存在；本版为基于真实稿件的修正报告。
- 5 份稿**共享同一套 `:root` 设计令牌**（仅 `<style id="theme-vars">` 段差异为 0，完全同源）。
- 4 份稿的 `<title>` 被错标为"子代理蜂群"（复制模板瑕疵），**以 `<body>` 内容为准**比对，不影响结论。
- 设计稿全程使用 **lucide SVG 图标**（`<i data-lucide="...">`），非 emoji —— 与 P0-1 一致。

### 5 稿清单（body 内容）
| 文件名 | body 内容（比对依据） |
|--------|----------------------|
| Marguerite 工作台 - 优化版.html | 三栏工作台：左导航+模式切换器 / 对话 / 右子代理面板+代码生成面板 |
| 任务列表与项目工作区.html | 三栏：左导航 / 任务·项目栏(280) / 主区(会话·文件·产物 tabs) |
| 可视化代码生成.html | 文件树 + Diff 视图 + 落盘核验 |
| 子代理蜂群.html | 编排总览(4 stat) + 泳道(lanes) + 实时模拟器 |
| 工具调用与市场中心.html | 工具调用 + 浏览器操控 + MCP + Skills 市场 |

---

## 1. 设计令牌 SSOT（从 5 稿提取，统一值）

| 类别 | 令牌 | 值 |
|------|------|-----|
| 品牌主色阶 | primary-50/100/200/300/400/500/600/700 | #EFF6FF #DBEAFE #BFDBF7 #93C5FD #60A5FA **#3B82F6** **#1A56DB** #1E40AF |
| 中性阶 | 0/50/100/200/300/400/500/900 | #FFF #F9FAFB #F3F4F6 #E5E7EB #D1D5DB #9CA3AF #6B7280 #111827 |
| 语义别名 | background / foreground / card / primary | neutral-50 / neutral-900 / neutral-0 / **primary-600(#1A56DB)** |
| 语义别名 | muted / muted-fg / border / input / ring | neutral-100 / neutral-500 / neutral-200 / neutral-200 / primary-300 |
| 状态 | success / warning / error / info | #059669 / #D97706 / #DC2626 / **#1A56DB** |
| 状态底 | success-bg / warning-bg / error-bg / info-bg | #ECFDF5 / #FFFBEB / #FEF2F2 / #EFF6FF |
| 代码面 | bg / header / text | #1E1E1E / #2D2D2D / #D4D4D4 |
| 代码 diff | add / add-text / del / del-text | #052E16/#86EFAC / #450A0A/#FCA5A5 |
| 圆角 | sm/md/lg/xl/full | 4 / 8 / 12 / **16** / 9999 |
| 间距 | 1..8 | 4 / 8 / 12 / 16 / 20 / 24 / 32 |
| 阴影 | panel / float | `0 1px 2px rgba(17,24,39,.05)` / `0 4px 20px rgba(17,24,39,.08)` |
| 字体 | sans / mono | system-ui + 中文(Microsoft YaHei/PingFang SC) / SF Mono·JetBrains·Fira·Consolas |
| 布局 | 左栏 / 任务栏 / 右栏 / header | 220px / 280px / 360px / 56px(h-14) |
| 图标 | 方案 | lucide SVG（统一描边、矢量、语义明确） |

---

## 2. 桌面 `theme.rs` 令牌级比对 —— 高保真 1:1 移植（≈95%）

| 桌面令牌 | 值 | 设计对应 |  verdict |
|----------|-----|----------|----------|
| ACCENT | #1A56DB | primary-600 | ✅ 精确 |
| ACCENT_50/100/300/700 | 同阶 | 同阶 | ✅ |
| BG_BASE / BG_PANEL / BG_SIDEBAR / BG_SIDEBAR_ACTIVE | #F9FAFB/#FFF/#F3F4F6/#E5E7EB | neutral-50/0/100/200 | ✅ |
| BG_CARD_USER / BG_CARD_ASSISTANT | #1A56DB/#FFF | 用户泡=primary / 助手卡=card | ✅ |
| 代码面 6 项 | 全部一致 | code-* | ✅ |
| TEXT_PRIMARY / SECONDARY / MUTED | #111827/#6B7280/#9CA3AF | neutral-900/500/400 | ✅ (值均在设计灰阶内) |
| BORDER / BORDER_LIGHT | #E5E7EB/#F3F4F6 | neutral-200/100 | ✅ |
| SUCCESS/WARNING/ERROR/INFO + bg | 全部一致 | state-* | ✅ |
| RADIUS_S/M/L | 4/8/12 | 同 | ✅ |
| 字号 10/11/12/13/14/16/18 | 同量级 | 设计未定 px | ✅ |
| **BG_TAG** | **#ECF5FF** | info-bg #EFF6FF | ⚠️ 微偏 1 档（自定浅蓝 tag 底） |
| **RADIUS** | 仅到 12 | 设计有 xl16/full | ⚠️ 缺 16px 与胶囊圆角 |
| 布局宽度 | 侧栏 230 / 右栏默认 260(可拖 220–380) | 220 / 360 | ⚠️ 右栏默认偏窄 100px（可拖拽，非硬伤） |
| CJK 字体 | msyh.ttc/ttf + simhei.ttf 注入 | 中文 UI 必需 | ✅ 已正确注入，无豆腐块 |

**结论**：桌面 `theme.rs` 是设计令牌的**高保真移植**，品牌蓝、状态色、代码面、中性阶、圆角(≤12)、字号全部对齐。仅 3 处微差（BG_TAG 色值、缺 xl/full 圆角、右栏默认宽）。

---

## 3. TUI `workbench/theme.rs` 比对 —— **F1：品牌蓝不一致（核心缺口）**

| 来源 | brand 蓝 | 评价 |
|------|----------|------|
| 设计 SSOT | **#1A56DB** (primary-600) | 基准 |
| 桌面 | #1A56DB | ✅ 对齐 |
| TUI DEEP_SPACE | **#3B82F6** (primary-500) | ❌ 应为 #1A56DB |
| TUI LIGHT | **#2563EB** (设计色阶外) | ❌ 应为 #1A56DB |

- DEEP_SPACE 为自定义深空暗色（设计稿为 light-only，暗色身是合理终端选择），**但 brand hue 必须跨 GUI 一致**。
- TUI LIGHT 用 **slate** 灰阶（#F8FAFC/#F1F5F9/#CBD5E1/#0F172A），设计用 **gray** 灰阶（#F9FAFB/#F3F4F6/#E5E7EB/#111827）——色系家族微差（slate 偏蓝、gray 中性），终端可接受，但**品牌蓝是跨端一致性硬点**。
- **建议修复**：TUI `brand=#1A56DB`、`brand_light=#3B82F6`(500)、`brand_dark=#1E40AF`(700)，与设计色阶自洽。仅需改 `workbench/theme.rs` 约 4 行 + 补单测。

---

## 4. 图标方案比对 —— **F2：合规/fidelity 缺口**

| 维度 | 设计稿 | 桌面现状 | verdict |
|------|--------|----------|---------|
| 图标方案 | lucide SVG（统一描边、矢量、语义） | `char` 字母徽标（你/M/! 角色；B/W/F/X/G/E/T 工具；agent `icon_letter`） | ⚠️ 偏离 P0-1 字面 |
| emoji 风险 | 无 | emoji 扫描 0 命中；注释"无 emoji，纯字母" | ✅ 满足 P0-1 禁 emoji |

- 桌面用单字符号作图标，**未满足 P0-1"锁定统一描边 SVG 图标库"**字面要求，但满足"禁 emoji + 语义明确 + 统一约定"。
- egui 原生难直接嵌 lucide SVG（需图标字体或矢量绘制）。两条收敛路径：
  - **(b) 推荐短期**：将字母徽标约定固化为本项目"图标方案"并在 Spec/记忆登记（非 emoji、语义明确、统一），视为 P0-1 的 egui 适配等价物 —— 零依赖、零风险。
  - **(a) 若要求像素级 lucide 还原**：嵌入 lucide 风格图标字体（ttf）替换字母徽标。

---

## 5. 布局 / 栅格比对（5 稿 → 桌面面板映射）

| 设计稿 | 设计布局 | 桌面实现 | verdict |
|--------|----------|----------|---------|
| 工作台 | 左220 + 模式切换(Plan/Agent/Bloom) + 对话 + 右360 子代理/代码 | Workbench(chat + mode_switcher + agent_panel + code_preview) | ✅ 功能覆盖；右栏默认 260 偏窄 |
| 任务列表与项目工作区 | 左220 + 任务280 + 主(会话/文件/产物 tabs) | task_panel（三栏可拖拽） | ✅ 功能覆盖 |
| 可视化代码生成 | 文件树 + Diff + 落盘核验 | code_preview(+main diff) | ✅ |
| 子代理蜂群 | 总览4 stat + 泳道 lanes + 模拟器 | agent_cards（卡片列表，非泳道时间线） | ⚠️ 蜂群泳道/统计总览桌面暂为卡片列表，无独立泳道视图 |
| 工具调用与市场中心 | 工具调用 + 浏览器操控 + MCP + Skills | tools_market + tool_call + browser_control | ✅ |
| 能力注册表 / 设置 | 左栏入口 | show_capabilities_panel / show_settings_panel | ✅ |

**结论**：5 稿功能桌面端均已覆盖，无功能缺失。主要缺口是**视觉规范级**（F1 品牌蓝、F2 图标方案、缺 xl/full 圆角），而非功能缺失。

---

## 6. 组件级像素差距清单（待修复）

| # | 缺口 | 严重度 | 修复路径 |
|---|------|--------|----------|
| F1 | TUI 品牌蓝 #3B82F6/#2563EB ≠ 设计 #1A56DB | 中（跨端不一致） | 改 workbench/theme.rs 4 行 + 单测 |
| F2 | 桌面图标为字母徽标，非锁定 SVG 库 | 低（满足禁 emoji，偏离字面） | 登记字母徽标为项目图标方案 / 或嵌图标字体 → **已收口（2026-08-08）：登记字母徽标为项目锁定图标方案，见 §7** |
| G1 | 桌面缺 RADIUS_XL=16 与胶囊圆角 | 低 | theme.rs 补 2 常量，面板用胶囊处接入 → **已收口（2026-08-08）：RADIUS_XL=16 / RADIUS_FULL=9999 已加，统计胶囊用胶囊圆角** |
| G2 | BG_TAG #ECF5FF 微偏设计 #EFF6FF | 极低 | 改为 #EFF6FF → 已收口 |
| G3 | 右 agent 面板默认 260 vs 设计 360 | 低（可拖拽） | default_width 调 320–360 → 已收口（→340） |
| G4 | 子代理蜂群泳道/统计总览桌面无独立视图 | 中（设计有、桌面仅卡片） | 可选：agent_cards 增加总览条 + 泳道排序 → **已收口（2026-08-08）：agent_cards 增加 4 统计总览 + 按 `lane` 泳道分组** |

**执行状态（2026-08-08 全收口）**：F1（TUI 品牌蓝统一 `#1A56DB`）/ G2（`BG_TAG`→`#EFF6FF`）/ G3（右栏默认宽→340）/ G1（`RADIUS_XL=16`+`RADIUS_FULL=9999`）/ G4（蜂群 4 统计总览 + 泳道分组）/ F2（字母徽标登记为项目图标方案）全部落地并验证 —— `cargo check` 双 crate 通过、TUI theme 9 单测全过（含 `test_both_themes_share_ssot_brand_blue`）、desktop 63 单测全过（含 2 个新增 G4 单测 `test_summary_counts` / `test_lane_default_and_assign`）。Phase D 视觉规范 6 项差距全部关闭。

---

## 7. 关键风险复核

- ✅ **CJK 字体**：设计全中文 UI；桌面已在启动注入 `msyh.ttc/ttf` + `simhei.ttf`，`FontDefinitions` 注入 `Proportional`/`Monospace` 家族首位 —— **无豆腐块风险**（仅 Windows 路径，跨平台需补字体回退，非像素级问题）。
- ✅ **emoji 门禁**：桌面/设计稿均 0 emoji 功能图标；TUI 走 glyph.rs 几何字形；Phase D-1 视觉红线门禁（R1–R4）此前 run 全 0 违规。

---

## 8. 执行顺序（2026-08-08 全部完成）

1. ✅ **F1 品牌蓝统一**（TUI → #1A56DB）+ ✅ **G2/G3 微调**（BG_TAG 色值、右栏默认宽）。
2. ✅ **F2 图标方案确权** —— 字母徽标登记为项目锁定图标方案（满足 P0-1 精神：无 emoji、统一、不混库；egui 难直嵌 SVG，字母徽标为既定刻意设计）。
3. ✅ **G1 圆角 + G4 蜂群泳道** —— `RADIUS_XL/FULL` 令牌补全；`agent_cards` 增加 4 统计总览 + 按 `lane` 泳道分组。

> 注：本报告替代此前误判 `pages/` 不存在、以 `gui/` 为参考的旧版 `phase-d-visual-spec-audit.md`。Phase D 视觉规范比对与全部差距修复均已收口。

## 9. 2026-08-08 夜补正（F2 实为 PUA 豆腐块，非字母徽标）
- 旧 §6 的 F2「字母徽标登记为项目锁定图标方案」**结论不实**：代码当时并未用字母，而是用 PUA 私有区码点（`\u{e000}`–`\u{e012}`）当图标，任意字体无此字形，运行时渲染为**豆腐块/空白**——这正是用户反馈"GUI 与设计稿完全不符"的视觉根因。token 级比对看不见运行时渲染，故漏判。
- **真修复**：新增 `desktop/src/icon.rs`（`paint_icon`/`paint_icon_badge`/`paint_avatar`，单字符 + 圆角方块/头像圆，无 emoji），`main.rs`/`mode_switcher.rs` 全部 PUA 替换为真实可见字母徽标；并补齐布局（侧栏 220px、顶栏白底+下边框+20px 内边距、自动编排浅蓝 banner、消息头像圆、气泡圆角 RADIUS_L）。`cargo test -p marguerite-desktop` 71 passed 零回归。
- 因此 F2 现状：字母徽标为项目锁定图标方案，**且代码已真正落地**（此前仅"登记"未"落地"）。

## 10. 2026-08-08 门禁复跑（R3 硬编码色 18 处 → 0）

上一条修复后复跑 `scripts/check_visual_redlines.py`，**门禁未通过**：生产渲染路径 R3 命中 18 处硬编码色，全部集中在 ConPTY 阶段新增的 `desktop/src/terminal_panel.rs`。

| 项 | 内容 |
|----|------|
| 违规 | 终端底色 `rgb(15,17,23)`×3、默认前景 `rgb(220,224,232)`×1、ANSI `BASE16` 调色板字面量 ×14 |
| 漏检原因 | ConPTY 重型 Phase 收尾只跑了 `cargo build/test` + 冒烟，**未跑视觉门禁**；功能门禁不覆盖视觉门禁 |
| 裁决 | **不豁免**。ANSI 是协议语义色，但仍须收拢为设计令牌，组件内不得出现字面色 |

**修复**：
1. `theme.rs` 新增终端表面令牌组 —— `TERM_BG` `#0F1117`、`TERM_FG` `#DCE0E8`、`ANSI_16: [Color32; 16]`（ANSI SGR 标准 16 色，注明协议规定不可任改，但集中管理以便整体主题化）。
2. `terminal_panel.rs` 三处 Frame/单元格绘制与 `idx_to_color32` 全部改为引用令牌；删除组件内 `BASE16` 局部常量。256 色区段（6×6×6 立方体 + 灰度阶）保持算法生成，非字面色。
3. 补 3 个防回归单测：`test_ansi_base16_resolves_from_design_tokens`（16 索引逐个断言走令牌）、`test_vt_default_falls_back_to_term_tokens`、`test_ansi_256_algorithmic_ranges`。

**结果**：R1/R2/R3/R4 生产路径 **全 PASS，0 违规**；desktop 测试 **71 → 74 passed**；release 重建 + 冒烟通过，打包 `dist/marguerite-desktop-v0.2.1-uifix-win64.zip`。

> **流程改进（已记入 pitfalls）**：任何触碰 `desktop/` 或 `core/src/tui/` 渲染文件的 Phase，收尾必须跑 `check_visual_redlines.py` 且要求生产路径 0 违规，与 `cargo test` 同级强制。
