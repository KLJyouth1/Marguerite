# 液态玻璃（liquid-glass）设计说明

> 主题 id：`liquid-glass` · 名称「液态玻璃 · 流光」 · `kind: 'value'` · 用户可选，`colorScheme: 'dark'`（`manifest.ts`）。
> 本文只记录**已实现的事实**：所有数值取自 `tokens.css`、`useGlassLab.ts`、`GlassBackdrop.vue`、`world.css`。
> 方法论、工程纪律与守卫命令见 `AGENTS.md` §3.2；本文只做该主题的规格说明。

## 1. 组成与事实来源

| 文件 | 职责 | 明确不做的事 |
|---|---|---|
| `webui/src/themes/liquid-glass/tokens.css` | 29 个必需 L1 语义角色 + 结构令牌 + 11 个 `--glass-*` 参数 | 不写任何选择器样式（除 `[data-theme]` 作用域本身） |
| `webui/src/themes/liquid-glass/world.css` | 全局世界层：排版、留白、外壳玻璃、卡片、控件、对话、降级 | 不定义新颜色，只消费语义令牌与 `--glass-*` |
| `webui/src/themes/liquid-glass/manifest.ts` | 注册：懒加载 `world.css`、`capabilities.respectsReducedMotion` | 不打包外部字体（走系统几何无衬线栈） |
| `webui/src/themes/contract.json` | 29 个必需 L1 角色的权威清单（`check-theme-contract.mjs` 的事实源） | 不包含 `--glass-*`（后者是本主题私有参数） |
| `webui/src/components/GlassBackdrop.vue` | 背景层：云朵 / 海面 / 沙丘三套大块柔和渐变 | 不改语义令牌、不接管交互（`pointer-events: none`） |
| `webui/src/components/settings/GlassLab.vue` | 参数面板 UI（滑块 + 场景切换 + JSON 导出） | 不改 `tokens.css`，只写 `<html>` 内联变量 |
| `webui/src/composables/useGlassLab.ts` | 参数定义、夹紧、持久化、导出 | 不感知具体组件 |
| `webui/src/composables/useGlassScene.ts` | 场景单一事实源 + 持久化 | 只影响背景画法 |

## 2. 设计语汇

| 维度 | 选择 | 说明 |
|---|---|---|
| 色 | 深色基底 + 两种主色 | 地面 `--bg` #070a12（不是纯黑：纯黑会吃掉玻璃层次）；主色极光蓝 `--accent`，次色薄荷 `--accent-secondary` 仅作点缀；文字不用纯白 `--text` #e9eef8 |
| 材质 | 白玻璃叠层 + 暗纱 | 上层白玻璃 `--glass-tint` 10%，下层暗纱 `--glass-scrim` 72%；两者缺一不可（只有白玻璃时背景一亮就吞白字） |
| 边界 | 1px 白亮边 + 内高光 | 边框 `--glass-edge` 42%；`inset 0 1px 0` 白高光 34% 造"厚度" |
| 折射 | 角部柔光 | 左上白光 `--glass-sheen` 16% + 右下主色冷光（×0.7），两块之间保持大面积中性 |
| 形 | 大圆角 | 控件 14 / 卡片 22 / 面板 26 / 弹窗 26 / 胶囊 999（`--radius-*`）；世界层卡片用 `--glass-radius` 22 |
| 字 | 紧字距几何无衬线 | `--font-display` 栈：Inter Tight → Geist → SF Pro Display → Segoe UI Variable Display → system-ui → Noto Sans SC；展示字 600 / -0.015em |
| 空间 | 1180px 中轴 | `.control-stage` `max-width: 1180px` 居中；内边距 `--glass-gap` 28，区块间距 `--glass-stack` 20 |
| 动 | 只动三种属性 | `transform` / `border-color` / `box-shadow`；时长与曲线只走 `--dur-*` / `--ease-*` |
| 层级 | 背景 → 外壳 → 卡片 → 控件 | 背景层 `z-index: -1`；外壳用实时磨砂；卡片只半透明；控件是短半径玻璃胶囊 |

## 3. 令牌表

### 3.1 L1 语义角色（29 个必需角色，本主题取值）

> 「用途」按角色语义登记；具体消费点以 `webui/src/assets/base.css` 与各组件为准。

| 角色 | 值 | 用途 |
|---|---|---|
| `--bg` | `#070a12` | 页面地面（玻璃后面的"夜色"，非纯黑） |
| `--bg-surface` | `#0c1018` | 无磨砂能力时的实色回退底 |
| `--bg-surface-2` | `#111823` | 次级表面（分组、嵌套区域） |
| `--bg-elevated` | `#161f2c` | 抬升表面（弹层、参数面板底板） |
| `--bg-hover` | `#1b2634` | 悬停底色 |
| `--text` | `#e9eef8` | 正文与标题（刻意不用纯白） |
| `--text-muted` | `#aebbcf` | 辅助说明、次要标签 |
| `--text-dim` | `#8494ab` | 三级信息、占位符 |
| `--border` | `#23303f` | 常规描边、分隔线 |
| `--border-strong` | `#33455a` | 强调描边（悬停、选中） |
| `--border-focus` | `#8fc7ff` | 焦点边框 |
| `--card` | `#0d131c` | 卡片实色底（回退用） |
| `--hairline` | `#1c2733` | 极细分隔线 |
| `--accent` | `#8ab4ff` | 主色：主按钮、选中态、焦点 |
| `--accent-hover` | `#a8c7ff` | 主色悬停 |
| `--accent-deep` | `#5d7fd6` | 主色深阶（背景色块、渐变暗端） |
| `--accent-secondary` | `#7ff0d8` | 次色薄荷：状态点、点缀，不铺大面积 |
| `--accent-foreground` | `#04101f` | 主色之上的文字（深色字） |
| `--ok` | `#7fe0a8` | 成功 |
| `--warn` | `#f2cc7a` | 警告；同时是「沙丘」场景的暖色源 |
| `--danger` | `#ff8b9a` | 危险 |
| `--info` | `#8fd0ff` | 信息 |
| `--queued` | `#c9b3ff` | 排队 |
| `--syntax-comment` | `#7c8bb8` | 代码注释 |
| `--syntax-keyword` | `#8ab4ff` | 关键字 |
| `--syntax-string` | `#7ff0d8` | 字符串 |
| `--syntax-literal` | `#f2cc7a` | 数字 / 字面量 |
| `--syntax-title` | `#ffd9a3` | 标题 / 函数名 |
| `--syntax-attr` | `#c9b3ff` | 属性 / 特性名 |

对比度基准（`tokens.css` 文件头，已按 WCAG AA 校验）：`text/bg ≈ 17.4:1`、`text-muted/bg-surface ≈ 9.4:1`、`accent/bg ≈ 8.9:1`、`accent-foreground/accent ≈ 11.6:1`。

### 3.2 结构令牌（主题覆写）

| 令牌 | 值 | 用途 |
|---|---|---|
| `--font-display` | Inter Tight / Geist / SF Pro Display / Segoe UI Variable Display / system-ui / Noto Sans SC | 展示字栈（无外部字体文件，避免 FOUT） |
| `--radius-control` | `14px` | 按钮、输入框、分段控件 |
| `--radius-card` | `22px` | 卡片、统计块 |
| `--radius-panel` | `26px` | 整块面板、抽屉、内容区 |
| `--radius-modal` | `26px` | 弹窗、对话输入区 |
| `--radius-pill` | `999px` | 胶囊、开关、头像 |
| `--focus-ring` | 双层冷色环（2px 72% + 6px 16%） | 玻璃上的可见焦点 |
| `--focus-ring-inset` | `inset 0 0 0 1px`（62% 主色） | 输入框内嵌焦点环 |
| `--shadow-color` | `color-mix(in srgb, #01030a 78%, transparent)` | 投影基色 |
| `--shadow` | `0 28px 64px -34px` + `0 2px 10px -6px` | 冷色长投影 |
| `--grain-opacity` | `0` | 关闭全局胶片颗粒（细碎噪点会显脏） |

### 3.3 玻璃参数（`--glass-*`，面板可调）

| 令牌 | 默认值 | 面板区间 | step | 消费点 |
|---|---|---|---|---|
| `--glass-radius` | `22px` | 8–40px | 1 | 卡片 / 区块 / 设置卡圆角 |
| `--glass-blur` | `30px` | 0–48px | 1 | 内容区与顶栏模糊；侧栏 ×0.75；弹窗 +8px |
| `--glass-saturate` | `165%` | 100–220% | 5 | 所有外壳的 `saturate()` |
| `--glass-tint` | `10%` | 4–20% | 1 | 白玻璃叠层不透明度 |
| `--glass-scrim` | `72%` | 30–92% | 2 | 玻璃底部暗纱（压背景亮度，保白字） |
| `--glass-edge` | `42%` | 8–90% | 2 | 1px 白亮边强度 |
| `--glass-sheen` | `16%` | 0–30% | 1 | 卡片角部折射高光 |
| `--glass-gap` | `28px` | 16–48px | 1 | 内容内边距（`.control-stage` / `.control-section`） |
| `--glass-stack` | `20px` | 8–40px | 2 | 区块之间的间距 |
| `--glass-lift` | `2px` | 0–6px | 1 | 卡片悬停抬升距离 |
| `--glass-text-veil` | `58%` | 30–90% | 2 | 气泡文字区底板（越低越透、越难读） |

派生用法（写在世界层里，不可调）：侧栏 blur 取 `--glass-blur × 0.75`、scrim 取 `+8%`；顶栏 scrim `+10%`；弹窗 blur `+8px`、scrim `+4%`；内容区白玻璃在 0% / 46% / 100% 三处取 `tint+4% / tint / tint-2%`。

## 4. 三个背景场景

公共结构（`GlassBackdrop.vue`）：5 个超大椭圆色块（`width` 46–82vw / `height` 40–64vh），`filter: blur(72/72/72/84/96px)`，不透明度 0.85（第 5 块 0.5），漂移周期 96s / 118s / 132s / 108s / 142s，起始延迟 0 / -22s / -48s / -64s / -12s。底色另有一层托底渐变：主色 16%（26% 4%）+ 薄荷 12%（84% 92%）+ `--bg`；四周暗角压暗贴边文字。**没有任何噪点/粒子/细纹理。**

| 场景 | id | 色块配方（1→5） | 地平线柔光 | 语汇 |
|---|---|---|---|---|
| 云朵 | `clouds`（默认） | text 44% / accent 52% / queued 40% / accent-secondary 32% / text 22% | 关闭（`opacity: 0`） | 月光白 + 极光蓝，最安静的一套，适合长时间阅读 |
| 海面 | `sea` | accent-deep 58% / accent-secondary 42% / accent 40% / info 30% / accent-secondary 34% | 开：46% 起透明 → 58% 薄荷 18% → 72% 透明 | 深海蓝 + 薄荷水面 + 一条低平光带 |
| 沙丘 | `dunes` | warn 46% / accent 36% / warn 36% / accent-deep 40% / text 18% | 开：52% 起透明 → 62% 暖沙 16% → 76% 透明 | 暖沙 + 冷影，两色对照但仍不喧哗 |

场景持久化在 localStorage `marguerite.glass-scene`（非法值回落 `clouds`）。切场景**不改任何语义令牌**，因此不影响对比度基线（`useGlassScene.ts`）。`prefers-reduced-motion: reduce` 下色块漂移停止，画面静态但层次不变。

## 5. 参数面板工作流（摘要）

设置 → 外观 → 玻璃参数（仅本主题激活时出现）→ 拖滑块即时预览 → 切三个场景复核 → 导出 JSON → 回写 `tokens.css` 与 `useGlassLab.ts` 的 `base` → 跑 5 条守卫。完整七步与导出结构见 `AGENTS.md` §3.2.2。

- 持久化键：`marguerite.glass-lab`（参数）、`marguerite.glass-scene`（场景）。
- 面板只能落在 `[min, max]` 内并对齐 `step`；非法持久化数据逐项回落各自的 `base`，不影响启动。
- 「恢复默认」回到 `GLASS_PARAM_DEFS[].base`，**不是** `tokens.css` 的现值 —— 回写时两边必须一起改。

## 6. 验收清单

**主题契约**

- [ ] `node scripts/check-theme-contract.mjs` 通过（`tokens.css` 覆盖 29 个必需 L1 角色）
- [ ] `node scripts/check-theme-scope.mjs` 通过（`world.css` 每条选择器带 `[data-theme="liquid-glass"]`）
- [ ] `node scripts/check-webui-colors.mjs` 通过（源码零裸颜色字面量）
- [ ] `node scripts/check-webui-motion.mjs` 通过（零裸时长 / 裸 `ease` 关键字）
- [ ] `node scripts/check-webui-radius.mjs` 通过（`border-radius` 零裸 `px/rem/em`）

**背景层与玻璃**

- [ ] 背景层挂在 `#app` 内且 `z-index: -1`，切换主题后不残留
- [ ] `.main / .main--chat / .chat / .chat--new-landing / .chat-thread / .chat-body` 均为 `transparent`
- [ ] `.particle-field` 在本主题下不可见
- [ ] 长段落压在玻璃上仍可读（三套场景各看一次）
- [ ] 只有外壳层声明 `backdrop-filter`；卡片无实时模糊
- [ ] 无 `backdrop-filter` 能力时回退到 88% 实色玻璃且文字不发虚

**参数面板**

- [ ] 拖动任一滑块即时生效，且刷新后保持
- [ ] 手动写入越界 localStorage 后仍能正常启动（值被夹紧）
- [ ] 「恢复默认」回到 11 项基准值
- [ ] 导出 JSON 含 `theme / scene / params / cssVars` 四个字段，`cssVars` 可直接贴进 `tokens.css`
- [ ] 三个场景切换即时生效且刷新后保持

**可访问性**

- [ ] `prefers-reduced-motion: reduce` 下：漂移停止、卡片悬停不位移、主按钮流光不播放
- [ ] 焦点环在玻璃上可见（键盘 Tab 走一遍外壳与控件）
- [ ] 对比度不低于 `tokens.css` 登记的四个基准

## 7. 未决项

| 项 | 现状 | 处理 |
|---|---|---|
| 「四个外壳」的口径 | `world.css` 文件头写「四个外壳（内容区/侧栏/对话顶栏/输入框）」，实测 `.chat-input-panel` 无 `backdrop-filter`，`.settings-overlay` / `.settings-modal` 声明了两处（共 5 处） | 待主代理确认是否改写注释；本文与 `AGENTS.md` §3.2.4 按实际声明（5 处）描述 |
| `--glass-radius` 与 `--radius-card` 的关系 | 两者默认都是 22，但一个是参数令牌、一个是阶梯令牌；世界层卡片用前者，基础组件用后者 | 待主代理确认是否需要显式绑定（`--glass-radius: var(--radius-card)`） |
