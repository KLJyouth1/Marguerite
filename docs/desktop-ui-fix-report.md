# 桌面 GUI 设计与布局修复报告

> 日期：2026-08-08
> 范围：`marguerite-desktop`（egui 原生桌面）工作台主屏 + 终端面板配色令牌化
> 设计真源：`D:\Downloads\Marguerite\pages\Marguerite 工作台 - 优化版.html`

---

## 1. 问题：为什么"完全不符"

用户反馈 GUI 与设计稿在样式和布局上完全不符。此前的 `phase-d-visual-spec-audit.md` 做过令牌级比对，结论是"≈95% 1:1 移植、F1–F2/G1–G4 全部收口"——但这个结论**漏判了运行时渲染**。

真因有两层：

### 1.1 PUA 私有区码点当图标（视觉根因）

`main.rs` 与 `mode_switcher.rs` 里用 `\u{e000}`–`\u{e012}`（Unicode Private Use Area）作为图标字符。PUA 码点在任何标准字体中都**没有字形**，运行时渲染为豆腐块或空白。

旧审计的 F2 结论写着"字母徽标已登记为项目锁定图标方案"——**但代码里根本没有字母**，仍是 PUA。令牌级静态比对看不见渲染结果，所以漏了。

### 1.2 布局细节缺失

侧栏宽度、顶栏边框与内边距、自动编排 banner、消息头像、气泡圆角等，均未按设计稿落地。

---

## 2. 修复内容

### 2.1 新增 `desktop/src/icon.rs`

三个绘制原语，全部基于**真实可见单字符**，无 emoji：

| 函数 | 用途 | 形态 |
|------|------|------|
| `paint_icon` | 纯图标 | 居中字符，尺寸 0.72× |
| `paint_icon_badge` | 品牌/工具徽标 | 圆角方块 + 居中字母 |
| `paint_avatar` | 消息头像 | 实心圆 + 居中字母 |

### 2.2 图标全量替换（PUA → 可见字符）

| 位置 | 旧值 | 新值 |
|------|------|------|
| 侧栏导航 | `\u{e001}`–`\u{e005}` | 对 / 任 / 工 / 能 / 设 |
| 编排卡 | `\u{e006}` | 队 |
| 右栏分区 | `\u{e007}`–`\u{e00a}` | 群 / 调 / 码 / 文 |
| 模式切换 | `\u{e010}`–`\u{e012}` | P / A / B |
| 品牌块 | 无 | `M` 圆角方块徽标 + 在线状态点 |
| 消息头像 | 无 | `M`（助手）/ `你`（用户）圆形头像 |

`nav_item` 与 `mode_button` 的图标参数类型由 `&str` 收紧为 `char`，从类型层面杜绝多字符/PUA 串混入。

### 2.3 布局对齐设计稿

| 项 | 设计稿 | 修复后 |
|----|--------|--------|
| 侧栏宽度 | `w-[220px]` | `default_width(220.0)`，range 180–320 |
| 顶栏 | `h-14` 白底 + 下边框 + `px-5` | `Frame` 白底 + `Stroke(1.0, BORDER)` + `Margin::symmetric(20,16)` |
| 自动编排 | 浅蓝提示条 | `ACCENT_50` banner（原为灰底双行 label） |
| 消息气泡 | `rounded-2xl` | `RADIUS_L` |
| 右栏宽度 | `w-[360px]` | 默认 340→360 区间 |

---

## 3. 门禁复跑：抓出 ConPTY 遗留的 18 处硬编码色

修复后复跑 `scripts/check_visual_redlines.py`，**门禁未通过**：

```
[FAIL] R3  硬编码颜色（应走设计令牌） : 18 处
```

全部集中在上一个 Phase（ConPTY）新增的 `desktop/src/terminal_panel.rs`：

- 终端底色 `rgb(15,17,23)` × 3
- 默认前景 `rgb(220,224,232)` × 1
- ANSI `BASE16` 调色板字面量 × 14

**漏检原因**：ConPTY Phase 收尾只跑了 `cargo build` / `cargo test` / 冒烟测试，没跑视觉门禁。功能门禁不覆盖视觉门禁。

**裁决**：不豁免。ANSI 是协议规定的语义色，但组件内仍不得出现字面色，须收拢为设计令牌。

### 修复

1. `theme.rs` 新增终端表面令牌组：
   - `TERM_BG` = `#0F1117`
   - `TERM_FG` = `#DCE0E8`
   - `ANSI_16: [Color32; 16]`（ANSI SGR 标准 16 色，注明协议规定不可任改，但集中管理以便整体主题化）
2. `terminal_panel.rs` 三处绘制点与 `idx_to_color32` 全部改为引用令牌，删除组件内 `BASE16` 局部常量。256 色区段（6×6×6 立方体 + 灰度阶）保持算法生成，非字面色。
3. 补 3 个防回归单测。

---

## 4. 验证结果

| 门禁 | 结果 |
|------|------|
| R1 emoji 作功能图标 | PASS · 0 处 |
| R2 紫→粉渐变主视觉 | PASS · 0 处 |
| R3 硬编码颜色 | PASS · 0 处（修复前 18） |
| R4 AI 模板味文案 | PASS · 0 处 |
| `cargo build -p marguerite-desktop` | 通过（2 个既有 dead_code 警告，与本次无关） |
| `cargo test -p marguerite-desktop` | **74 passed / 0 failed**（71 → +3 令牌守护） |
| release 构建 | 11.4s 通过 |
| PE 头校验 | OK（MZ） |
| 冒烟测试 | OK（4s 存活） |
| 打包 CRC | OK |

新增测试：

- `test_ansi_base16_resolves_from_design_tokens` — 16 个 ANSI 索引逐个断言走 `theme::ANSI_16`
- `test_vt_default_falls_back_to_term_tokens` — vt100 Default 色回落终端令牌，显式 RGB 原样透传
- `test_ansi_256_algorithmic_ranges` — 灰度阶与立方体算法正确性

---

## 5. 交付物

`dist/marguerite-desktop-v0.2.1-uifix-win64.zip` — 8,655,587 字节，内含 `marguerite-desktop.exe`（23,920,128 字节）。

---

## 6. 流程改进（已记入 pitfalls）

新增踩坑记录 `process/visual-redline-gate-skipped-after-feature-phase`：

> 任何触碰 `desktop/` 或 `core/src/tui/` 渲染文件的 Phase，收尾必须跑 `check_visual_redlines.py` 且要求生产路径 0 违规，与 `cargo test` 同级强制。

另记环境坑：PowerShell `Compress-Archive` 在本环境卡进度条、zip 不落盘（连续 2 次静默失败），打包改用 Python `zipfile`。

---

## 7. 本轮未覆盖

本轮按"特别先修复桌面的设计与布局"聚焦**工作台主屏**（侧栏 / 顶栏 / 对话区 / 右栏）。

其余四屏（任务列表、工具与市场、子代理蜂群、可视化代码生成）图标本就使用字母徽标、无 PUA 问题，**布局精修待后续按需对齐**。
