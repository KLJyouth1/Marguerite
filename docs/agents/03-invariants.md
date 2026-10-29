<!-- 本文件原为 AGENTS.md §3，2026-10-29 由 scripts/split-agents-md.mjs 逐字节搬运；
     内容未删改，请勿在此加"摘要"，细节就该留在分册里。 -->
## 3. 架构不变量（invariants — 违反即缺陷）

1. **Web UI 仅回环托管**：`webui/`（Vue3 控制台）与统一网关只监听 `127.0.0.1`（默认 18790），**禁止** Tauri 前端与公网监听；三端（TUI/桌面/Web）共享蓝橙语义令牌。
2. **ACP 仅回环**：`serve` 永远只监听 `127.0.0.1`，绝不 `0.0.0.0` / 公网。
3. **离线可运行**：编排与 `agent.invoke` 走 `Orchestrator::analyze`（不调 LLM），无 API Key 降级为离线回显。
4. **内部可变性范式**：跨 `await` 共享状态用 `Arc<Mutex<T>>`；克隆锁再跨 await，避免 `MutexGuard` 非 Send。
5. **治理双闭环**：执行前 `PlanGate`（宪法 C1–C4）、产出后 `CompletionGate`（四检查 + 自报文件落盘核验）。
6. **CLI 输出契约**：结构化错误走 stderr + 退出码；`--json` 输出 `{ok,data,meta}`；尊重 `NO_COLOR`；文本标记用 ASCII `[OK]/[FAIL]`，**禁止 emoji 图标**。
7. **路由三处同步**：`cli/argv.rs` 的 `SUBCOMMANDS`、`program.rs` 的 `route_table.register(...)`、`test_program_*` 的路由计数断言必须一致。当前 **20 个路由**：`doctor` / `model` / `mcp` / `skills` / `session` / `exec` / `deploy` / `serve` / `sandbox` / `subagent` / `team` / `browser` / `chat` / `gateway` / `goal` / `capabilities` / `cache` / `dispatch` / `games` / `export-diagnostics`（计数断言在 `program.rs:4327`）。
8. **缓存三层诚实性（Task #5）**：`MultiTierCache` 的 Tier 0 必须与网关 `gateway_prefix_cache()` 共享同一 `Arc<PrefixCache>`（`Arc::ptr_eq` 为真），确保精确命中由共享方统一计量、零重复计数；Tier 2 语义近似默认关闭（`semantic_enabled=false`），且工具路径（`chat_complete_with_tools_cached`）永远走 `ExactOnly` 策略——近似命中会静默丢弃工具调用，是正确性缺陷；`cache.status` 的 `tiers.hit_rate` 由真实计数相除（夹紧 [0,100]），**绝不冒充 100%**。
9. **WebUI 导航分组契约（Task #6/#7）**：侧栏由 `getWorkNavigationSection()`（平铺核心 rail，仅 group:Work 的 nav:primary 项）与 `getGroupedNavigation()`（可折叠 Work/Operate/Observe/Settings 四组，收纳所有深链能力）共同渲染；新增深链能力必须设 `meta.group`，否则不可见于导航；`/control/control-center`（智能中枢，控制台 base 为 `/control`，故浏览器 URL 需带前缀）为模式/BloomSpec/缓存/能力的统一仪表，数据源仅限既有 RPC（mode.get/switch/spec、cache.status、plans.capabilities），不得新增后端契约。

### 3.1 Bloom 模式与插件系统的设计红线（anti-patterns；策划 §三 + bloom.yaml）

> 以下两条来源于 dsh-forge 的反模式警告，已在 `core/src/mode/modes/bloom.yaml` 的 `anti_patterns` 字段声明，同时由代码侧**强制守卫**；任何绕过守卫的实现都视作缺陷。

8. **红线 #1：禁止全上下文重注入**（dsh-forge："禁止全上下文重注入"）
   - 会话内只允许**增量追加**消息；禁止替换 system 提示、禁止回滚或重置整段历史。
   - 插件侧实现：`plugins/di.rs::PluginLoader::discover` 会扫描 `plugin.json` 是否包含 `system_prompt` / `full_context` 字段，并把插件名加入 `context_leak_suspects()`；上层启动时必须拒绝启用该列表中的插件（或至少显式告警 + require-ack）。
   - 持久注册表 `PluginRegistryFile::entries` 的 schema 中**不得**出现以上字段；新增序列化字段时代码评审要对照本 invariant。

9. **红线 #2：避免中途替换工具面**（dsh-forge："避免中途替换工具面"）
   - 会话内工具/服务集合必须**单调扩展**；Bloom 三段锚定（`core/src/mode/anchoring.rs::AnchorStage::{Anchored→Ptc→Full}`）只加工具不撤回；PTC 阶段不得回退到锚定工具集；不得随机重置已提供的服务。
   - 插件侧实现：`plugins/di.rs::PluginLoader::guarded_provide::<T>` 在 `T` 的 `TypeId` 已出现于 `applied_service_types` 时**返回 Err 并指明红线违规**；只有 `reload_plugin()` 作为显式管理动作会清空该集合并允许替换。
   - 任何直接 `ServiceContainer::provide`（无 guard 包装）替换了同一 TypeId 的代码，在 CI lint 中应被标注为「潜在红线违规」。

### 3.2 UI 设计方法论：液态玻璃与克制型商业界面

> 本节固化 `webui/src/themes/liquid-glass/` 背后的**设计方法**（不是代码）：四条克制原则、参数面板工作流、禁止清单、改世界层前的 5 项检查、守卫命令。
> 它适用于所有"要像商业产品而不是模板"的界面改动 —— 判据是通用的，只是这套主题把每条都实现了一遍、可逐条对照。

#### 3.2.1 四条克制原则（先约束，再谈效果）

| 原则 | 落地做法（可复核） | 为什么 / 漏做的后果（证据） |
|---|---|---|
| ① 留白优先 | 控制台收窄到 `max-width: 1180px` 居中；卡片内边距 `--glass-gap`（28px）、区块间距 `--glass-stack`（20px），两者都由参数面板驱动 | 内容铺满整屏时，"层级"只能靠不断加分割线和边框堆出来，越堆越脏；中轴两侧的留白本身就是层级（`world.css` §2） |
| ② 黑白灰为主 | 层次只用明度阶（`--bg` #070a12 → `--text` #e9eef8）+ 玻璃叠层 + 1px 白亮边；彩色只出现在主按钮、焦点环、状态点 | 大面积彩色会让人失去"先看哪里"；本主题主色只有极光蓝 `--accent` #8ab4ff 与薄荷 `--accent-secondary` #7ff0d8 两种，薄荷只作点缀（`tokens.css` 取值与注释） |
| ③ 字号/字重/间距建层级 | 展示字 600 字重、`-0.015em` 紧字距；标题 `clamp(1.45rem, 1.1rem + 1vw, 2rem)`；正文 1.72 行高；辅助文字降到 `--text-muted` | 靠字号/字重建立的层级在窗口缩到 80% 或窄屏时仍成立；靠颜色建立的层级最先塌（`world.css` §1） |
| ④ 真实场景代入 + 可交互的真实使用感 | 背景是云朵/海面/沙丘三套**大块柔和渐变**（`GlassBackdrop.vue`）；玻璃有厚度（内高光 + 底部冷色内阴影）、有折射（角部柔光）、悬停抬 `--glass-lift`（2px）、按压回 1px | 纯色背景 + 平面色块 = 模板感；只有"会响应"的材质才会被当成真实物体（`GlassBackdrop.vue` 文件头、`world.css` §3a/§4/§5） |

#### 3.2.2 参数面板工作流（调参靠导出的 JSON，不靠口头描述）

**所有尺寸与材质参数集中成一个面板 → 手动调 → 导出 JSON → 回写 `tokens.css`。** 七步：

1. **开面板**：设置 → 外观 → 玻璃参数。面板只在 `liquid-glass` 激活时渲染（`SettingsAppearancePanel.vue` 里的 `v-if="glassThemeActive"` → `GlassLab.vue`）。
2. **拖滑块**：每个可调项 = 一个 `--glass-*` 变量 + 区间 + step；拖动即写 `<html>` 内联变量并全应用即时预览，不改任何 CSS、不重启（`useGlassLab.ts` 的 `setGlassParam` / `applyGlassParams`）。
3. **自动记住**：值持久化到 localStorage `marguerite.glass-lab`；读回时逐项 `clampParam`（非有限值回落基准 / 越界夹紧 / 对齐 step），所以不存在"上次调崩了打不开"（`useGlassLab.ts`）。
4. **切背景复核**：云朵 / 海面 / 沙丘三选一（`useGlassScene.ts`，持久化 `marguerite.glass-scene`，默认 `clouds`）。切场景只换背景画法、**不动语义令牌**，所以绕不过对比度守卫（`useGlassScene.ts` 文件头）。
5. **导出 JSON**：面板底部 `<details>` 实时展示 `exportGlassParams()` 的输出，作为"这次就是这些参数"的唯一凭据（`GlassLab.vue`）。
6. **回写 `tokens.css`**：把导出里的 `cssVars` 逐项抄回 `tokens.css` 的 `--glass-*` 组；同时把 `params` 的数值同步进 `useGlassLab.ts` 的 `GLASS_PARAM_DEFS[].base`。**两处必须一致**，否则「恢复默认」会跳回旧值（`useGlassLab.ts` 的 `resetGlassParams`）。
7. **跑守卫**：见 §3.2.5。

导出结构（`exportGlassParams()` 的字段名就是契约，改字段名会打断"导出 → 回写"的链路）：

```json
{
  "theme": "liquid-glass",
  "scene": "clouds",
  "params": { "radius": 22, "blur": 30, "saturate": 165, "…": "共 11 项，键名 = GlassParamDef.key" },
  "cssVars": { "--glass-radius": "22px", "--glass-blur": "30px", "…": "已拼接单位，可直接贴进 tokens.css" }
}
```

⚠️ 导出的是**面板当前值**，不是主题基准；`scene` 由 `exportGlassParams` 直接从 localStorage 读取（为避开 `useGlassLab` ↔ `useGlassScene` 循环依赖），所以导出前先把场景切好。

#### 3.2.3 禁止清单（附判据）

| 禁止项 | 判据（怎么一眼判定） | 为什么不许 / 证据 |
|---|---|---|
| 杂乱 | 一屏内 >1 个"强彩色块"；或同一张卡片同时用边框 + 投影 + 渐变三种强调 | 多种强调互相抵消，用户读不出主次；本主题卡片只有"描边 + 内高光 + 角部柔光"一套（`world.css` §4） |
| 细碎纹理 | 能看到 <2px 的重复颗粒、噪点、细网格、粒子 | 细碎纹理是廉价感的主因；本主题 `--grain-opacity: 0`（`tokens.css`），且 `.particle-field` 必须关闭。⚠️ 这条规则要写 `[data-theme="liquid-glass"] .chat .particle-field`：ParticleField 的 scoped 规则（`.particle-field[data-v-*]`）与单层前缀同级，而组件样式在懒加载的 `world.css` 之后注入，同级会输给它（`world.css` §0） |
| 多彩 | 同屏 ≥3 种非状态色（状态色 = `--ok/--warn/--danger/--info/--queued`） | 语义色被当装饰后，"红色=出错"的约定就失效了（`tokens.css` 状态色分组） |
| 纯色背景 | `GlassBackdrop` 只剩单色（色块删掉 / 透明度归零） | 纯色底没有可折射的内容，"磨砂玻璃"退化成一块灰板（`GlassBackdrop.vue` 文件头与底色渐变） |
| 低画质 | 放大截图出现色带（banding）、模糊边缘露方框、图标/插图像素化 | 大半径模糊会把低分辨率素材的锯齿放大成脏边（`GlassBackdrop.vue` 的 `blur(72–96px)`） |
| 文字不可辨认 | 长段落直接压在玻璃上；或取色发现正文与底 <4.5:1 | 只有白玻璃层（`--glass-tint` 10%）时，背景一亮点就把白字吞掉 —— 必须叠暗纱 `--glass-scrim`（72%），长段落再加 `--glass-text-veil`（58%）（`world.css` §3a/§6） |

#### 3.2.4 改这个世界层之前，必须先检查的 5 件事

| # | 检查项 | 怎么查 | 漏查的后果（实测） |
|---|---|---|---|
| ① | 背景层可见性 | `.glass-backdrop` 必须挂在 `#app` 之内（`App.vue` 根模板里紧随 `<MargueriteSky>`、由 `v-if="glassActive"` 控制），并由世界层给 `z-index: -1` | 背景"画了但看不见"，玻璃后面只剩死黑（`world.css` §0） |
| ② | 不透明容器让开 | `.main / .main--chat / .chat / .chat--new-landing / .chat-thread / .chat-body` 的底色必须是 `transparent`。`!important` 是实测必需：base 层用多处 `background` 简写会把底色重置回来，仅靠 (0,2,0) 特异度压不住 | 背景层整块被盖住 —— 本主题第一个踩到的坑（`world.css` §0） |
| ③ | `backdrop-filter` 数量 | 只允许外壳层：`.content`、`.sidebar`、`.topbar--chat`、`.settings-overlay`、`.settings-modal`（现存 5 处声明）；`.card / .control-card / .control-section / .settings-card` 只做半透明 + 描边 | 每张卡片都要重算整屏背景，滚动掉帧（`world.css` 文件头"性能纪律"与 §4） |
| ④ | 对比度 | 四个基准：text/bg ≈ **17.4:1**、text-muted/bg-surface ≈ **9.4:1**、accent/bg ≈ **8.9:1**、accent-foreground/accent ≈ **11.6:1**；改完 `--glass-*` 要在云/海/沙丘三套背景下各看一次长段落 | 亮背景 + 低 scrim = 白字糊在玻璃上（`tokens.css` 文件头） |
| ⑤ | reduced-motion | 漂移必须停（`GlassBackdrop.vue` 的 `prefers-reduced-motion: reduce` 分支）、卡片悬停不得位移、主按钮流光不得播放 | 前庭敏感用户拿不到静态画面；manifest 已承诺 `respectsReducedMotion: true`，不实现即违约（`manifest.ts`、`world.css` §10） |

#### 3.2.5 守卫命令（改主题必须逐条跑；在 `webui/` 下执行，脚本实际位置 `webui/scripts/`）

```bash
node scripts/check-theme-contract.mjs
node scripts/check-webui-colors.mjs
node scripts/check-webui-motion.mjs
node scripts/check-webui-radius.mjs
node scripts/check-theme-scope.mjs
```

| 守卫 | 拦什么 | 与本节的关系 |
|---|---|---|
| `check-theme-contract.mjs` | `src/themes/*/tokens.css` 必须定义 `contract.json` 里全部 **29 个必需 L1 角色** | 少一个角色 = 运行时元素不可见（该守卫注释称之为 phantom theme） |
| `check-webui-colors.mjs` | `src/` 下的裸 hex / `rgb()` / `hsl()` 字面量（令牌定义文件内的自定义属性除外） | 玻璃全靠 `color-mix(in srgb, var(--token) …)` 拼；写死一个颜色，换主题就穿帮 |
| `check-webui-motion.mjs` | `transition/animation` 里的裸 `120ms`/`0.3s` 与 `ease`/`ease-in` 关键字 | 本主题动效必须走 `--dur-*` / `--ease-*`（定义在 `foundation.css`） |
| `check-webui-radius.mjs` | `border-radius` 里的裸 `px/rem/em` | 圆角必须走 `--radius-*` 阶梯；`--glass-radius` 只用于世界层的卡片/面板 |
| `check-theme-scope.mjs` | `world.css` 每条选择器都必须带 `[data-theme="liquid-glass"]` 前缀 | 漏一个前缀，这套玻璃就会泄漏到别的主题 |

> 这 5 条已并进 `npm run check:architecture`（`webui/package.json`），单独跑只是为了定位失败项。

⚠️ 口径差异（**待主代理确认**）：`world.css` 文件头把 `backdrop-filter` 的落点写成「四个外壳（内容区/侧栏/对话顶栏/输入框）」，实测 `.chat-input-panel` **没有**声明 `backdrop-filter`，而 `.settings-overlay` / `.settings-modal` 声明了两处 —— 上文以**实际声明**（5 处）为准，注释口径是否改写待确认。

