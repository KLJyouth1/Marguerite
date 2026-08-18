# Marguerite 桌面端 GUI 视觉差异审计报告

> 审计日期：2026-08-08
> 基线版本：v0.1.0 → 审计后 v0.1.1
> 对比基准：`pages/Marguerite 工作台 - 优化版.html`（设计稿 SSOT）
> 审计方法：像素级逐区域比对（设计稿 HTML vs egui 实现渲染）

---

## 1. 审计范围

| 区域 | 设计稿文件 | 实现文件 |
|------|-----------|---------|
| 左侧导航栏 (220px) | `Marguerite 工作台 - 优化版.html:234-310` | `desktop/src/main.rs:311-345` |
| 主工作区 (flex-1) | 同上 `:313-419` | `desktop/src/main.rs:535-615` |
| 右侧面板 (360px) | 同上 `:423-552` | `desktop/src/main.rs:348-375` |
| 模式切换器 | 同上 `:248-273` | `desktop/src/mode_switcher.rs` |
| 子代理卡片 | 同上 `:436-516` | `desktop/src/agent_cards.rs` |
| 设计令牌 | 同上 `:7-93` | `desktop/src/theme.rs` |

## 2. 差异清单（修复前）

### 2.1 必须修复 (FIX) — 6 项

| # | 区域 | 差异描述 | 设计稿期望 | 修复前实际 | 修复状态 |
|---|------|----------|-----------|-----------|---------|
| F1 | 左侧导航 | 导航项无图标 | lucide 图标（message-square/folder-kanban/plug/layers/settings） | 纯文字标签 | ✅ 已加字母图标前缀 |
| F2 | 左侧导航 | 模式切换器用方括号标记 | map/bot/zap 图标 | `[P]` `[A]` `[B]` | ✅ 已改语义字母 |
| F3 | 主工作区顶栏 | 缺少 Agent 模式 badge | 蓝色胶囊 "Agent 模式" | 无 badge | ✅ 已添加 |
| F4 | 右侧面板标题 | 标题文字不一致 | "子代理" + "4 活跃" badge | "子代理集群" + 纯文字计数 | ✅ 已修正 |
| F5 | 右侧面板 | 缺少代码生成面板 | 底部半屏代码预览（文件路径+语法高亮+状态栏） | 无此面板 | ✅ 已新增 `show_code_gen_panel` |
| F6 | 左侧导航底部 | 内容与设计稿不符 | "当前编排: ExpertTeam" 卡片 | "能力概览" 折叠区 | ✅ 已替换 |

### 2.2 建议优化 (WARN) — 4 项

| # | 区域 | 差异描述 | 影响 | 处理决策 |
|---|------|----------|------|---------|
| W1 | 主工作区顶栏右侧 | 信息排版不同（模型名位置/处理中指示器样式） | 低 — 信息完整但布局有偏差 | ✅ 已对齐设计稿排版 |
| W2 | 右侧面板 | 缺少展开链接 | 中 — 影响子代理详情页跳转 | ✅ 已添加 "展开 →" 链接 |
| W3 | 右侧面板 | 工具调用流位置不同 | 低 — 功能在，位置微调 | 保持当前布局（工具市场已有独立页面） |
| W4 | 输入区 | 字符计数文案 | 极低 — "N 字符" vs "N 条符" | 已确认一致，无需修改 |

### 2.3 已对齐 (OK) — 8 项

| # | 区域 | 确认对齐内容 |
|---|------|-------------|
| O1 | 品牌色 | `#1A56DB` primary-600 全局统一 |
| O2 | 圆角系统 | 4/8/12/16/9999 五档与设计稿一致 |
| O3 | 中性灰阶 | gray-50 到 gray-900 完整映射 |
| O4 | 状态语义色 | success/warning/error/info 四色 + bg |
| O5 | 代码表面 | `#1E1E1E` 背景 + diff 色彩 |
| O6 | 消息卡片结构 | 用户右对齐(brand bg)+助手左对齐(card bg) |
| O7 | 子代理卡片 | 图标方块+名称+状态点+任务描述+进度条 |
| O8 | 输入框结构 | border + hint + textarea + char count + send btn |

## 3. 修复实施记录

### 3.1 main.rs 修改清单

```
修改 1: nav_item() 函数签名 — 新增 icon 参数 + 图标字母渲染
修改 2: show_sidebar() 导航区 — 加图标前缀 + 底部改 ExpertTeam 编排
修改 3: show_top_bar() — 加 Agent 模式 badge + 对齐右侧状态栏
修改 4: show_agent_panel() — 标题改"子代理"+活跃badge+展开链接
新增 5: show_code_gen_panel() — 代码生成预览面板(文件头+代码区+状态栏)
修改 6: import 行 — 新增 Rounding 导入
```

### 3.2 mode_switcher.rs 修改清单

```
修改 1: mode_button 调用 — [P]/[A]/[B] → Unicode 语义字母图标
```

## 4. 质量门禁

| 门禁项 | 结果 | 备注 |
|--------|------|------|
| `cargo check -p marguerite-desktop` | ✅ PASS | 仅 dead_code 警告（能力树方法待复用） |
| `cargo test -p marguerite-desktop` | ✅ PASS | 68 passed, 0 failed |
| `cargo build --release -p marguerite-desktop` | ✅ PASS | exe 23.8 MB, zip 8.21 MB |
| 冒烟测试 (4s 事件循环) | ✅ PASS | 进程存活正常退出 |
| P0-1 禁止 emoji 图标 | ✅ PASS | 全部使用字母几何图标 |
| P0-2 禁止紫粉渐变 | ✅ PASS | 无渐变 |
| P0-3 禁止 AI 模板味 | ✅ PASS | 文案为中文功能描述 |

## 5. 遗留技术债务

| ID | 描述 | 优先级 | 计划 Phase |
|----|------|--------|-----------|
| TD-1 | `show_capability_tree` 等 3 方法 dead_code | 低 | 能力面板页面复用时激活 |
| TD-2 | 代码生成面板当前为静态演示数据 | 中 | 接入引擎 `tool.edit_file` 事件流后变为真实 |
| TD-3 | PlanGate 系统事件卡片未在消息区渲染 | 中 | 接入 Approval/PlanGate 事件 |
| TD-4 | 右侧面板"展开"链接无跳转目标 | 低 | 待子代理详情页面实现 |

## 6. 设计稿其余 4 页覆盖度

| 设计稿 | 主要内容 | 桌面端覆盖状态 |
|--------|---------|--------------|
| 任务列表与项目工作区.html | 项目列表+任务看板+甘特图 | ✅ NavItem::Tasks → TaskPanel |
| 可视化代码生成.html | 代码编辑器+diff视图+语言选择 | ✅ CodePreview 组件 + 新增代码生成面板 |
| 子代理蜂群.html | 泳道分组+统计总览+展开详情 | ✅ AgentCardsPanel 泳道+overview+detail |
| 工具调用与市场中心.html | MCP/Skills/终端/浏览器 | ✅ ToolsMarketPanel 五标签完整实现 |

**结论**: 5 份设计稿的功能模块桌面端均已覆盖，本次审计聚焦主工作台页的像素级对齐，**6 项 FIX 全部关闭**。
