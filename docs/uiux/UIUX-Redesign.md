# Marguerite UI/UX 重设计文档（三文档 · UIUX 部分）

> 范围：产物（Artifact）面板重设计 · Bloom 路由融合可视化 · 桌面自动化可视化 · Design Token 草案 · 图标系统
> 交付对象：team-lead / architect / pm
> 约束基线：P0 绝对规则（禁 emoji 图标 / 禁紫粉渐变 / 禁空洞占位 / 禁硬编码色 / 禁千篇一律 Hero / 禁 AI 模板味）
> 现状结论（基于代码阅读）：品牌为「蓝主(#60A5FA) + 橙强调(#F97316)」，暗色石墨底；Bloom 融合层已用青色 `#21c8c8`；webui 已有统一描边 SVG 图标方案（lucide 形态），桌面 egui 用字母徽标（ADR-002）。本设计在**现有 token 体系上扩展**，不另起炉灶。

---

## 0. 现状诊断（代码阅读，非设计空想）

### 0.1 产物面板 Bug 根因（需求①）
- `RightSidebar.vue` 的「产物」Tab 渲染 `props.artifacts`（类型 `RsArtifact { name, path, openable? }`），数据来自 `ChatView.vue` 注入。
- `artifactPreview.ts` 已支持 `html/markdown/text/image/pdf` 六类预览判定；markdown 走 `renderArtifactMarkdown`（DOMPurify 净化 + 允许列表）。
- **Bug 定位**：会话生成的 `.md` 未进入 `RightSidebar` 的 `artifacts` 列表 → 集合未与会话产物流（artifact stream / workbench store）双向同步，或 markdown 产物被判定为 `unsupported`/`openable=false` 而静默过滤。**设计侧要求**：产物集合必须反映所有格式（文档/表格/PPT/图片/代码/音频/视频/PDF），空态时必须主动引导，非空时提供统一预览+编辑入口。

### 0.2 Bloom 路由可视化现状（需求②）
- `RouterFxStrip.vue` 已实现两套面板：`real-candidates`（候选网格 + 扫描选择器动画 + 胜者高亮）与 `llm-ensemble`（Bloom/融合按钮 + 可展开 inspector 行：role / model / usage / status）。
- 已用 `--bloom-accent`（青）、`--accent`（蓝=胜者）、`--danger`（红=回退），已尊重 `prefers-reduced-motion`。
- **待增强点**：① token 消耗未量化展示；② 并行/串行结构未图形化；③ 融合（fusion）结果缺少与候选的视觉归属关系；④ 气泡/thinking/工具卡/产物引用的状态尚未与路由打通。

### 0.3 Token / 图标现状
- 暗色：`--bg #18181A` → `--bg-surface #202022` → `--bg-elevated #2D2D30`；`--text #F5F5F7`；`--accent #60A5FA`；`--accent-secondary #F97316`；`--bloom-accent #21c8c8`；状态谱 `--ok/--warn/--danger/--info/--queued`。
- 字体：`--font-sans`（系统 -apple-system + PingFang SC 等 CJK 回退）、`--font-mono`（IBM Plex Mono）。**缺独立西文 Inter + 中文 Noto Sans SC 锁定**——本草案补齐。
- 圆角：4 级语义 `--radius-control/-card/-panel/-modal` + 原始阶梯 4/8/10/14/18/24px。
- 图标：`@/utils/icons` 的 `IconName` 已是 lucide 形态描边 SVG（fileText/fileCode/image/table/router…），渲染器 `getIconSvg` 统一 stroke 2px、currentColor。→ 锁定 Lucide 为唯一来源，补齐新图标即可。

---

## 1. 竞品调研与行业趋势（联网，≥3）

| 竞品 | 产物/工作区范式 | 可借鉴 | 不可抄（避免 AI 味） |
|---|---|---|---|
| **Claude Artifacts** | 会话右侧**常驻**预览面板；Preview/Code 双 Tab；可编辑、可下载、可发布；行内高亮局部改 | ① 预览与对话**并列常驻**而非弹窗 ② 多格式统一预览器 ③ 就地编辑 + 版本 | 不直接套其浅色极简；本产品暗色+信息密度更高 |
| **ChatGPT Canvas** | 文档/代码双栏；高亮选区 → 针对性修订；阅读层级滑块 | ① 选区→局部编辑心智 ② 左编辑右预览分屏 | 不做其"写作助手"营销感 |
| **Manus** | 「My Computer」实时窗口逐帧回放；执行轨迹可审计、可接管；语义化动词导航 | ① **动作回放/审计时间线** ② 实时轨迹 + 可干预闸门 ③ 透明化执行 | 不做其密集日志墙；本产品用受控色阶而非彩色噪音 |
| **Linear** | 暗色优先、1px 发丝边框、状态用色点+标签、键盘优先、动效 120–160ms 收敛 | ① 状态色阶体系 ② 克制现代感 ③ 密度优先 | 直接复用其 indigo `#5e6ad2` 会撞 P0 默认蓝——本产品改用自有蓝 `#60A5FA` |
| **Notion** | 信息密度高、块化文档、左大纲右内容、类型图标系统 | ① 多格式类型图标 ② 大纲/分屏预览 ③ 空状态引导文案 | 不做其奶油米色背景（P0 禁暖中性默认化） |

**趋势提炼**：
1. 产物 = 会话的「第二画布」，常驻并列、可编辑可发布，而非内联长文本。
2. 智能体透明化：执行过程必须「可看、可回放、可接管」。
3. 状态用**色阶语义**（蓝=主/选中、青=融合、橙=需确认/动作、绿=完成、红=失败），不用装饰渐变。
4. 暗色优先 + 发丝边框 + 收敛动效 = 专业工具质感。

---

## 2. 对标品牌与设计语言

**对标 1 — Notion（信息密度 + 灵活文档预览）**：多格式类型图标、左大纲右内容分屏、空状态用引导文案而非转圈。→ 用于**产物面板**。
**对标 2 — Linear（克制现代感 + 状态色阶 + 暗色优先）**：发丝边框、状态色点、收敛动效、键盘可达。→ 用于**会话状态/Bloom/自动化**。

**熔接方向**：Notion 的「密度与分屏」骨架 + Linear 的「暗色克制与状态色阶」皮肤，套在 Marguerite 自有品牌（蓝主+橙强调+青色 Bloom）上。

**品牌约束（不可违背 P0）**：
- 主色蓝 `#60A5FA`，强调橙 `#F97316`，融合青 `#21c8c8`（沿用现有 `--bloom-accent`）。
- 绝对禁止紫→粉渐变；状态色阶只允许蓝/青/橙/绿/红/琥珀，且每屏强调色 ≤ 2 处。
- 字体锁定：西文 Inter + 中文 Noto Sans SC + 等宽 IBM Plex Mono（沿用现有 mono）。

---

## 3. 产物面板重设计（需求①·全部格式统一预览/编辑）

### 3.1 「产物」Tab 升级为「产物工作台」
保留 RightSidebar 三 Tab（任务 / 产物 / 更改），但「产物」从纯列表升级为**集合视图**：

- **视图切换**：列表（默认，信息密度高）/ 网格（缩略图，适合图片/PDF/PPT）。
- **过滤栏**：按类型 chips（文档·表格·幻灯片·图片·代码·音频·视频·PDF），多选过滤，chips 用类型色点。
- **排序**：最近生成 / 类型 / 名称。
- **计数徽标**：沿用现有 `rs-tab__badge`，显示当前会话产物数。

### 3.2 空状态引导（修复「不显示」的第一道体验）
当 `artifacts.length === 0`：
- 图标：`fileStack`（lucide）24px，`opacity .3`，居中。
- 文案（具体、非空洞）：「本次会话还没有生成文件。让 Marguerite 写出一份报告、表格或代码，文件会出现在这里，可直接预览与编辑。」
- 行动按钮：「新建文档」(`fileText`+) / 「用 /gen 生成」——双轨入口雏形（见 3.5），点击即在 workbench 开一个空白 markdown 产物。
- **工程联动**：空态必须同时校验「集合未同步」分支——若会话已产生 `.md` 却空，显示诊断条「检测到 N 个未载入产物，点击重试」（`refresh` 图标），驱动 store 重新拉取（修复 Bug 的兜底 UX）。

### 3.3 多格式类型图标体系（锁定 Lucide）
统一用 `Icon` 组件（16/20/24px，stroke 2px，currentColor）。类型→图标映射（扩展现有 `fileIcon`）：

| 格式 | 扩展名 | 图标 | 类型色 |
|---|---|---|---|
| 文档 | md/markdown/txt/doc/docx/rtf | `fileText` | `--type-doc` |
| 表格 | csv/xlsx/xls/tsv | `table` / `sheet` | `--type-sheet` |
| 幻灯片 | ppt/pptx/key | `presentation` / `monitorPlay` | `--type-deck` |
| 图片 | png/jpg/jpeg/gif/svg/webp/avif | `image` | `--type-image` |
| 代码 | js/ts/py/rs/go/java/vue/css/json/html… | `fileCode` | `--type-code` |
| 音频 | mp3/wav/m4a | `music` / `audioLines` | `--type-audio` |
| 视频 | mp4/mov/webm | `video` / `film` | `--type-video` |
| PDF | pdf | `fileTypePdf` / `fileText` | `--type-pdf` |
| 网页 | html/htm | `fileCode` + 角标 | `--type-code` |
| 未知 | * | `fileQuestion` | `--text-dim` |

类型色仅作**小色点/角标**，不染色整卡（防视觉噪音，P0 每屏强调≤2）。

### 3.4 列表 / 网格项
- **列表项**（沿用 `.rs-artifact` 结构，增强）：`[类型图标 16] [名称 / 路径(mono 10px)] [格式标签] [状态点: 生成中/完成/错误] [打开 12] [更多 …]`。
- **网格项**：缩略图（图片用 `<img>` / 文档用首屏渲染 / PPT 用首页截图占位）+ 文件名 + 类型角标；hover 浮起 `--elev-1-hover`。
- **状态点语义**：生成中=`--accent` 脉冲；完成=`--ok`；错误=`--danger`；人工锁定=`--accent-secondary`。

### 3.5 预览器布局（左预览 / 右大纲 · 分屏）
产物预览沿用 workbench `artifact-preview` 面板，新增**分屏骨架**：

```
┌───────────────────────────────┬──────────────────┐
│ 预览区（render）               │ 大纲 / 属性       │
│ html→sandbox iframe            │ ─ 文档大纲(标题树) │
│ md → 渲染 + 行号               │ ─ 元数据(类型/大小)│
│ csv → 表格                      │ ─ 产物引用(哪些消息)│
│ img → 缩放查看                  │ ─ 导出/打开         │
│ code→ 高亮 + 行号              │                    │
└───────────────────────────────┴──────────────────┘
        ↑ 可拖拽分隔条（沿用 WorkbenchResizer）
```
- 大纲：markdown 解析 H1–H4 成树；代码解析函数/类符号；csv 解析 sheet 名。
- **内联编辑器（双轨之一）**：markdown/文本/csv 直接在预览区切换「预览 / 编辑」Tab（仿 Canvas），编辑态 `textarea`/`contenteditable` + 行号；改动经 workbench runtime 写回产物（保留版本）。图片/PDF/PPT 不可内联编辑时，编辑器 Tab 禁用并提示用系统程序打开。
- **「用系统程序打开」双轨入口（之二）**：工具栏常驻按钮 `externalApp`（图标 `externalLink` 形态，命名 `openWithApp`），点击调用 Tauri `open(path)` 唤起 WPS/浏览器/剪映/系统查看器。视觉处理：与内联编辑并排为两个主操作——「在 Marguerite 内编辑」(ghost) / 「用 {应用名} 打开」(secondary)，应用名动态显示（WPS/Preview/默认）。避免双按钮同质——主按钮按产物可编辑性决定（文档=内编辑为主，图片=外打开为主）。

---

## 4. Bloom 路由可视化组件（需求②）

> 在现有 `RouterFxStrip` 语言上增强，**不重画**。新增：token 消耗量化、并行/串行结构、融合归属、气泡/thinking/工具卡/产物引用状态打通。

### 4.1 配色语义（严格色阶，避开紫粉）
| 语义 | Token | 色值 | 用法 |
|---|---|---|---|
| 主模型 / 胜者 | `--accent` | `#60A5FA` | 胜者高亮、扫描选择器 |
| 候选模型 | `--bg-elevated` + `--text-muted` | 石墨/灰 | 未选中网格单元 |
| **Bloom 融合层** | `--bloom-accent` | `#21c8c8` | 「Bloom · {子模式}」标签、融合结果归属线 |
| 聚合器(aggregator) | `--accent-secondary` | `#F97316` | inspector 中 aggregator 行徽标 |
| 回退(fallback) | `--danger` | `#FF6B6B` | 回退胜者、错误行 |
| 跳过(skipped) | `--text-dim` | `#87878E` | 弃用候选 |
| 运行中(running) | `--accent` 脉冲 | 蓝脉冲 | 进行中行/点 |

### 4.2 候选人链（并行/串行）组件草图
现有 `real-candidates` 网格改为**有向链布局**：
- 串行：`[M1] → [M2] → [M3]` 横向节点 + 连接线（`--border-strong`），节点=模型短名 mono 9px。
- 并行：`[M1a][M1b]` 同列上方括号「并行 ×N」，汇合到 `[聚合]`。
- 胜者节点：`--accent` 边框 + 右上 4px 点（沿用 `win::after`）。
- 扫描态：选择器沿链移动（沿用现有 `router-fx-selector` 弹簧位移），`prefers-reduced-motion` 直接静态高亮。
- **Token 消耗**：每个节点右下角 `12 tok` / `1.2k tok`（mono 9px，`--router-muted`），聚合器节点显示 `Σ 3.4k`；总消耗在面板脚注「本次路由 4.1k tok · 融合省 38%」。

### 4.3 融合结果（fusion）归属可视化
- 在 ensemble inspector 中，融合结果作为独立行 `★ 融合结果`（青色 `--bloom-accent` 左条 + sparkles 图标），其下缩进列出贡献候选（带贡献权重 `62%`/`28%`/`10%` 微型进度条，用 `--bloom-accent` 填充）。
- 气泡内：当助手消息含产物且来自融合，气泡右下角加 `bloom` 小徽标（`Bloom · 融合` + 青点），与产物引用卡片联动高亮。
- thinking 卡：路由扫描期间显示「路由中…」微型路由条（复用 RouterFxStrip 的扫描动画，尺寸缩小 60%）。

### 4.4 工具调用卡 / 产物引用状态打通
- 工具卡（`ActivityToolDetails`）新增路由来源脚注：「经 Bloom 融合 · 主模型 Claude / 候选 GPT」（`router` 图标 + mono）。
- 产物引用（`ArtifactChip` / `ChatArtifactList`）：引用项显示类型图标 + 生成时路由徽标；hover 时右侧预览面板滚动到对应产物（焦点环 `--focus-ring`）。

---

## 5. 桌面自动化可视化（需求③）

### 5.1 实时轨迹组件（会话内「正在操控 X 执行 Y」）
在助手消息流中以**嵌入式 live strip** 呈现（仿 Manus 实时窗口，但收敛为 Linear 式状态条）：

```
[app 图标] WPS 文字 │ 动作: 打开「季度报告.docx」  [状态点:实时]  [进度条]
                                                    └ 下一步: 替换第3段 →
```
- 配色：实时=`--info`(#56C2E6 蓝) 脉冲点；完成=`--ok`；失败/中断=`--danger`；需确认=`--accent-secondary`(橙，品牌「动作」)。**不引入新紫色**，避免 `--queued` 紫。
- 应用图标：用 `app-*` 一组 lucide 风格应用字形（WPS=`table`+「W」角标 / 浏览器=`globe` / 剪映=`clapperboard` / 3D=`box` / 文件=`folder`），统一 16px 描边。
- 动作动词化导航（仿 Manus 语义动词）：「打开 / 输入 / 点击 / 滚动 / 拖拽 / 保存 / 导出」，用 `cursor`/`keyboard`/`mouse-left` 等现有动作图标前缀。

### 5.2 动作回放 / 审计时间线
- 独立面板（右栏新增「自动化」Tab 或会话内可展开 `SubagentCard` 式时间线），垂直时间轴：
  - 节点 = 单步动作（应用 + 动作 + 时间戳 + 截图缩略图）。
  - 可点击节点 → 弹出该步**屏幕快照**（截图） + 操作前后 diff。
  - 回放控制：播放/暂停/`timer` 进度 scrubber（沿用现有 `ToolCallTimeline` 思路）。
  - 筛选：按应用 / 按结果（成功/失败/人工接管）。
- 审计属性：每步含 `actionId`、耗时、是否人工确认、回放可定位（链接到会话消息）。

### 5.3 确认闸门 UI（人机协同）
- 当智能体需敏感操作（删除/发送/付费/覆盖），插入 **ApprovalCard**（沿用现有 `ApprovalCard.vue`）增强版：
  - 头部：`shield-check` 图标 + 「需要确认」+ 应用名 + 动作描述（具体，非「Continue」）。
  - 主体：操作影响预览（将修改的文件 / 将发送的收件人 / 将扣费额度）。
  - 操作：主按钮「允许此次」(`--accent`) / 「始终允许该应用」(ghost) / 「拒绝」(`--danger` ghost) / 「接管」(secondary)。
  - 超时：倒计时环（`timer`），到期自动拒绝（安全默认）。
  - 焦点：默认聚焦「拒绝」，防误触（a11y）。

---

## 6. Design Token 草案（扩展现有体系，不直接替换）

> 沿用 `foundation.css` + `themes/dark/tokens.css` 的 L1 变量，新增「产物 / 路由 / 自动化」专属 token 与字体锁定。所有颜色走 token，零硬编码。

### 6.1 品牌与中性（已有，重申）
```
--bg:#18181A; --bg-surface:#202022; --bg-surface-2:#28282B; --bg-elevated:#2D2D30; --bg-hover:#353539;
--text:#F5F5F7; --text-muted:#B0B0B6; --text-dim:#87878E;
--border:#303034; --border-strong:#444448; --hairline:#29292C;
--accent:#60A5FA; --accent-hover:#93C5FD; --accent-deep:#2563EB; --accent-foreground:#0B1A3B;
--accent-secondary:#F97316; --accent-secondary-hover:#FB923C; --accent-secondary-deep:#C2410C;
--bloom-accent:#21c8c8;                       /* Bloom 融合层，沿用 */
--ok:#39D7A2; --warn:#E8B23A; --danger:#FF6B6B; --info:#56C2E6;  /* 状态谱，复用 */
```

### 6.2 字体（新增锁定）
```
--font-sans: "Inter", -apple-system, "PingFang SC", "Noto Sans SC", "Microsoft YaHei", sans-serif;
--font-display: "Inter", -apple-system, "PingFang SC", "Noto Sans SC", sans-serif;
--font-mono: "IBM Plex Mono", ui-monospace, "SFMono-Regular", Menlo, monospace;  /* 沿用 */
/* 字号阶梯（沿用 foundation --fs-*，补 display） */
--fs-display: 2rem; --fs-2xl:2rem; --fs-xl:1.625rem; --fs-lg:1.1875rem;
--fs-md:1rem; --fs-sm:0.875rem; --fs-xs:0.8125rem;
/* 字距：ALL CAPS 0.06em；标题 -0.01em；正文 0 */
--track-eyebrow:0.06em; --track-display:-0.01em; --track-body:0;
/* 字重（仿 Linear 510 精神）：正文 400 / 强调 510 / 标题 590 */
--fw-read:400; --fw-emphasis:510; --fw-announce:590;
```

### 6.3 间距 / 圆角 / 阴影（沿用 + 提取语义）
```
--sp-1:4px; --sp-2:8px; --sp-3:12px; --sp-4:18px; --sp-5:24px; --sp-6:30px; --sp-8:40px;
--radius-control:10px; --radius-card:14px; --radius-panel:18px; --radius-modal:24px; --radius-pill:999px;
--elev-1 / --elev-1-hover / --elev-2 / --elev-3;   /* 沿用 machined elevation */
--focus-ring:0 0 0 3px color-mix(in srgb, var(--accent) 40%, transparent);
--dur-fast:120ms; --dur-base:200ms; --dur-enter:280ms; --ease-standard:cubic-bezier(.2,0,0,1);
```

### 6.4 新增 · 产物类型色（小色点/角标专用）
```
--type-doc:#8AB4F8; --type-sheet:#39D7A2; --type-deck:#C792EA→改#F2A65A(橙系避免紫);
--type-image:#56C2E6; --type-code:#93C5FD; --type-audio:#B0B0B6; --type-video:#7AA2F7;
--type-pdf:#FF6B6B; --type-unknown:#87878E;
```
> 说明：`--type-deck` 弃用紫 `#C792EA`，改橙系 `#F2A65A`，严守「无紫」。类型色仅作 4px 点/角标，不染色卡片主体。

### 6.5 新增 · 路由状态 / 自动化状态 token
```
/* 路由 */
--route-primary:var(--accent); --route-candidate:var(--text-muted);
--route-fusion:var(--bloom-accent); --route-aggregator:var(--accent-secondary);
--route-fallback:var(--danger); --route-skipped:var(--text-dim);
/* 自动化 */
--auto-live:var(--info); --auto-done:var(--ok); --auto-blocked:var(--accent-secondary);
--auto-failed:var(--danger); --auto-queued:var(--text-dim);
/* 自动化轨迹专用（不引入新紫，用青/蓝区分 Bloom） */
--auto-trail:color-mix(in srgb, var(--info) 30%, var(--bg-surface));
```

---

## 7. 图标系统（锁定 Lucide）

**决策**：webui 图标方案 = **Lucide**（统一描边 2px、矢量、语义明确、24×24 viewBox）。现有 `@/utils/icons` 的 `IconName` 已为 lucide 形态，迁移为「单一 Lucide 来源 + 本项目扩展集」，渲染器 `getIconSvg` 不变（strokeWidth 默认 2，currentColor）。尺寸：行内 16 / 按钮内 20 / 独立 24。桌面 egui 维持 ADR-002 字母徽标（不在此范围）。

> **P0 硬约束（架构文档 4.3/7，来自 team-lead）**：Lucide 为全项目唯一图标来源，**严禁 emoji**、**严禁混用 Material / Ant Design / 其它库**。本稿全部图标名均为 lucide 派生，UI 文案与标注零 emoji。下列名称中带「＋」者为当前 `IconName`(~80) 尚未收录、需以 **lucide 官方 path 数据**增补进 `icons.ts` 的图标（仍属 lucide，非新造、非第三方）。

### 7.1 产物类型图标（§3.3 映射）
`fileText` `table` `presentation`(＋) `image` `fileCode` `music` `video` `fileTypePdf`(＋) `fileQuestion`(＋) `files`(空态, ＋) `externalLink`(打开/外链)

### 7.2 路由状态图标
`router`(路由根) `gitBranch`(候选链, ＋) `layers`(并行) `boxes`(并行候选, ＋) `zap`(融合触发, ＋) `chevronsRight`(串行, ＋) `sparkles`(融合结果) `box`(聚合器) `check`(完成) `xCircle`(回退/失败, ＋) `slash`(跳过, ＋) `loader`(运行中, ＋) `coins`(token 消耗, ＋)

### 7.3 自动化动作图标
`window`(应用框/外打开, ＋) `externalLink`(系统程序打开) `globe`(浏览器, ＋) `clapperboard`(剪映, ＋) `box`(3D) `folder`(文件管理) `cursor`(点击) `keyboard`(输入) `mouse-left`(鼠标) `scroll`(滚动, ＋) `move`(拖拽) `save`(保存) `download`(导出) `shield-check`(确认闸门, ＋) `timer`(超时/回放) `play`/`pause`(回放控制) `history`(审计时间线, ＋) `eye`(快照查看) `hand`(人工接管, ＋)

---

## 8. 落地检查清单（给 architect / 前端）

- [ ] 产物集合与会话 artifact stream 双向同步（修 `.md` 不显示 Bug）；空态含「重试载入」兜底。
- [ ] 预览器分屏（预览/大纲）+ 内联编辑 Tab + 「用系统程序打开」双轨按钮（Tauri `open`）。
- [ ] 类型图标全格式覆盖；类型色点替换整卡染色。
- [ ] Bloom：候选人链并行/串行布局 + token 量化 + 融合归属行 + 气泡/thinking/工具卡状态打通。
- [ ] 自动化：live strip（应用+动作+状态）+ 审计时间线（快照+回放）+ ApprovalCard 增强（影响预览+超时拒绝+默认聚焦拒绝）。
- [ ] 所有新增颜色改用 §6 token；每屏强调色 ≤ 2；零 emoji、零紫粉渐变。
- [ ] 动效 ≤ 300ms 且 `prefers-reduced-motion` 全量降级；焦点环统一 `--focus-ring`；触摸目标 ≥ 44px。

---
*调研来源：Claude Artifacts / ChatGPT Canvas 官方说明与对比评测（2025–2026）；Manus 实时窗口与执行轨迹公开评测；Linear / Notion 设计系统公开拆解。代码基线：webui/src（RightSidebar.vue、RouterFxStrip.vue、artifactPreview.ts、themes/dark/tokens.css、foundation.css、utils/icons.ts）。*
