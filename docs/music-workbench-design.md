# Marguerite 音乐工作台原创设计规范（Constellation / Aurora Glass）

> 定稿日期：2026-10-20。
> 依据：`docs/music-workbench-research.md`（Hero MV 视觉语言 + 7 个启动界面案例）。
> 范围：webui（Vue3 + Vite + TS），由网关回环托管，Tauri 桌面壳加载同一产物。
> 原则：只增不删现有功能；颜色/动效/圆角全部走既有设计令牌；研究案例只学模式，不复制任何代码或素材。

## 0. 设计概念

「**Constellation / Aurora Glass（星丛·极光玻璃）**」——工作台在播放音乐时，从冷峻的石墨界面缓慢"呼吸"成一层轻度磨砂毛玻璃，界面背后透出由音频驱动的雷达环与粒子星丛；停止播放时原路缓慢退回。

视觉母题取自 Hero MV：纯黑/石墨底、自发光细线、四角 HUD 取景框、粒子汇聚、同心雷达隧道；强调色严格沿用 Marguerite 蓝主+橙语义令牌，不使用磷光绿（避免与 CLI `[OK]` 语义混淆）。

## 1. 分层模型（z-index，关键）

| 层 | 内容 | 层级 | 交互 |
|---|---|---|---|
| 背景层 | 音乐可视化（雷达环 + 粒子星丛 canvas）、极光 blobs | 0 | pointer-events: none |
| 内容层 | 侧边栏 / 顶栏 / 路由内容（现状） | 现状 | 正常 |
| 毛玻璃 scrim | 整屏 wash + backdrop blur、边缘 rim | 9999（低于 boot 的 2147483000） | pointer-events: none |

要点：**可视化与极光必须在内容之下，毛玻璃 scrim 必须在所有 chrome 之上**，整座工作台才能被"磨砂"。
现状 AmbienceLayer 的 z-index=1 且 scrim 与极光同层，需要拆成背景层与 scrim 层。

## 2. 启动序列（沿用现有三段结构，只做修正与增强）

1. 挂载前 CSS splash `#mq-boot`（boot.css）：保留。
2. 首访 `/splash`（SplashView，会话内一次）：bg.mp4 雷达隧道无声循环 + 四角 HUD + 波形 + 6s 自动进入 /chat，点击即进入；音频自动播放被拦截时静默降级。
3. 挂载后 BootOrchestra（`bootAnimation=full` 时，每配置档一次）：星座节点 + 自检日志；末行品牌 + ready 光标。
增强点：
- 日志沿用既有 `[ OK ]` 行节奏（错峰 120–150ms）；可在日志中体现 `BOOT 01/06` 分阶段命名，但不得在无真实就绪证据时提前显示 ready（仓库铁律）。
- 全部动效在 reduced-motion 下收敛为静态 mark；splash 始终可跳过（点击/Enter/Esc），不阻挡正常使用。

## 3. 内置音乐播放器（内核已实现，本轮做 UI 与默认可见性）

### 3.1 默认可见
- 全新配置档默认 `enabled=true`（播放器默认可见、可点按；**不自动出声**，浏览器自动播放策略本就要求手势）；设置项可关闭（持久化）。
- 旧持久化数据无该字段时按新默认处理，不破坏已有用户的关闭选择。

### 3.2 播放器形态（BgmControl 升级，不删任何现有能力）
- 收起态：顶栏音符按钮（播放/暂停），播放时按钮变为 3 根跳动的迷你均衡条；caret 展开播放面板。
- 展开面板包含：
  - 上一首 / 播放·暂停 / 下一首；
  - 可拖拽/点击寻址的进度条 + 当前时间 / 总时长（mm:ss 纯函数格式化）；
  - repeat 按钮：off → all → one 三态循环（图标/ARIA 随状态）；
  - shuffle 开关（ARIA pressed）；
  - 曲目列表（现有）、音量滑条（现有）、选择本地文件（现有）。
- 全部按钮可键盘操作，带 aria-label / role；面板沿用现有 .theme-menu 外观，避免漂移。

## 4. 音频驱动可视化（新增，2D canvas）

新建 `src/components/music/MusicVisualizer.vue`（指针透明、fixed、覆盖视口），数据源为已实现的
`useAudioReactivity()`（spectrum/waveform/bass/mid/treble/energy/beat）。场景（全部原创）：

1. **同心雷达环**（Hero 雷达隧道母题）：屏幕下半部居中 3–5 个同心圆环，随 bass 能量轻微扩张/增亮；beat 时最外环脉冲一次；环上沿频谱分布径向短 bar（青色/蓝用 color-mix 取令牌色）。
2. **粒子星丛**（Hero 粉尘粒子母题）：80–160 个粒子缓慢上浮；速度/亮度/大小由 mid/treble 能量驱动；beat 时粒子向心收束再散开。
3. 极细水平波形线（可选，贴近底部）随节拍呼吸。

工程约束：
- 纯 2D Canvas，按 devicePixelRatio 缩放；播放停止 / 标签隐藏 / reduced-motion 时停止 rAF；
- 颜色只用 `color-mix(in srgb, var(--token) X%, transparent)` 或 `getComputedStyle` 取令牌，禁止颜色字面量；
- 透明度克制，任何时候不降低正文可读性；视觉参数（粒子数、环数、速度）抽为可测纯函数/配置（如 `visualizerMath.ts`），与绘制分离。

## 5. 毛玻璃过渡（用户核心诉求：开始播放时背景缓慢变透明到轻度磨砂毛玻璃）

- 在 foundation.css 补齐缺失的 **`--dur-slow`（建议 1200ms）** 令牌——现状该变量只被引用、从未定义，导致所有"缓慢"过渡实际为瞬时（真实缺陷）。
- 播放开始（`playing→true`）时在 `--dur-slow` 内平滑爬升：
  - frost scrim：wash 0 → 58%（`--bg-surface` 混合），blur 0 → 9px；
  - 极光 opacity 0 → 0.5（并随实时能量微动）；
  - 边缘 rim 光 0 → 35%+；
- 暂停时原路、同时长退回。
- `prefers-reduced-transparency`：关闭 blur，仅保留 wash；`prefers-reduced-motion`：极光/环静止，保留 wash 渐变。
- 毛玻璃在 WebView2/Tauri 的实际表现必须真机截图验证；若 backdrop-filter 不支持，降级为半透明 wash（不阻塞功能）。

## 6. 音乐模式布局变动（克制且明确）

`body.ambience-on` 时（过渡时长统一 `--dur-slow`）：
1. 侧边栏与顶栏背景由不透明变为轻度半透明玻璃（加自身 backdrop blur，约 6–10px），让可视化在边缘隐约透出；
2. 路由卡片/面板表面轻度半透明（新增令牌如 `--glass-panel-bg`，由现有 `--card` 与透明混合），保证文字对比度不低于 4.5:1；
3. 一条 1px 内描边 rim 作为"音乐中"的签名提示（已实现，保留）。
除此之外不做位移/缩放，避免干扰操作；主内容可读性优先。

## 7. 发布门禁更新（放行自研 CC0 曲目，继续拦截个人版权音乐）

`webui/scripts/verify-dist.mjs`：
1. `officialMusicFiles` 白名单加入 4 首官方 CC0 文件（dusk-pad/neon-arp/lofi-drift/focus-pulse .mp3）；
2. 把"tracks 必须为空"断言改为：每个 track 的相对 src 必须命中白名单（https 流仍允许但不会出现在官方 playlist）；music/ 下出现白名单以外的文件依旧判错；
3. 同步镜像的 pyengine 断言：`pyengine/tests/test_scripts/test_verify_webui_artifact.py`（按其现有 fixture 同步更新，保持两边一致）。
安全意图不变：个人/版权音乐在官方构建中依然被拦截。

## 8. i18n

- 新增 key：上一首/下一首/seek/repeat 三态/shuffle/进度/时长/迷你播放器等 aria 与文案。
- 6 个 locale（en/zh-Hans/de/es/fr/ja）全部补齐（非中英可用英文译文占位），确保 check-i18n 键集一致；中文为主语言。

## 9. 测试要求（只增不减）

- 播放器组件：渲染、点击各按钮调用 store、进度条寻址、repeat 三态循环、shuffle 状态、面板开关/焦点管理、ARIA；
- 可视化：粒子更新/环参数等纯函数（visualizerMath）单测；
- 门禁：verify-dist 白名单/拦截行为（随 pyengine 镜像 fixture 或对应单测）；
- 默认 enabled 变更：同步更新断言旧默认值的既有测试；
- 测试总数不得低于当前基线（3950 passed / 385 files，完成 B 后）。

## 10. 验收（无新鲜全绿证据不得声称完成）

依次真实运行并贴出：
```
cd D:\Downloads\Marguerite\webui
npm run check:architecture
npm run typecheck
npm run test:unit
npm run build
```
并由真机（localhost:18790 网关，Tauri/浏览器）截图/录屏：启动序列、播放后毛玻璃+极光+雷达环/粒子、暂停退回。
