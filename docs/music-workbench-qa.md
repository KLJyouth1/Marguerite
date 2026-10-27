# Marguerite「音乐工作台」QA 巡检报告

- 巡检日期：2026-10-20
- 巡检角色：独立 QA（非实现者）
- 被测对象：webui 音乐工作台（启动序列 / 顶栏播放器 / 毛玻璃氛围 / 音频可视化）
- 被测构建：`webui/dist`（vite 产物，302 files；桌面端部署副本 303 files）
- 环境：Windows，独立网关 18899（不占用用户 18790 桌面端）；真机浏览器 + Tauri 桌面网关复核

---

## 1. 第一部分：门禁复跑（真实执行，全绿）

工作目录 `D:\Downloads\Marguerite\webui`，node v22.23.2。

| 门禁命令 | 结果 | 证据 |
|---|---|---|
| `npm run check:architecture` | PASS | 12 个守卫全过：architecture / chat-security / colors / motion / radius / theme-contract / theme-scope / cross-surface-tokens / i18n(5 语种全键对齐) / channel-catalog-i18n / rpc-contract（24 门控方法 + 376 声明全有实现，幽灵 0）/ index-injection-contract |
| `npm run typecheck` | PASS | `check:architecture` + `vue-tsc --noEmit` 零错误 |
| `npm run test:unit` | PASS | **388 files / 3970 passed / 0 failed**（与基线一致） |
| `npm run build` | PASS | vite 10.91s；theme-bundle guard、runtime-bundle guard 过；`verify-dist.mjs --write` 校验 **302 files** |

门禁全部 `[PASS]`，无 `[SKIP]`。

---

## 2. 第二部分：非侵入式真机验证（独立网关 18899 + 浏览器）

启动独立网关 `target\debug\marguerite-core.exe gateway --port 18899`（PID 12060），token 取自 `/control/` 返回 HTML 的 `#Marguerite-data[data-ws-token]`。用户桌面端 18790 全程未动。

| # | 验收项 | 判定 | 证据 |
|---|---|---|---|
| a | 首访启动序列（CSS splash + `/splash` 全屏启动页） | PASS | HTML 含 `#mq-boot` 星座覆盖层（Agent/Subagent/Tools/Memory/Retrieval/Knowledge 6 节点）；根路由经 `sessionStorage.marguerite_splash_seen` 重定向 `/splash`；SplashView 播 `bg.mp4`（雷达环视频）+ HUD（150 BPM / DOLBY ATMOS / ALT 00515）+ 波形，约 6s 自动进 `/chat`。截图 `qa-shots/03-splash-view.png` |
| b | 顶栏播放器默认可见、展开面板 | PASS | `[data-testid=bgm-toggle]` 默认渲染；`bgm-menu-trigger` 展开后有 prev / play-pause / next、可寻址进度条 + mm:ss、repeat、shuffle、4 首曲目（Dusk Pad / Neon Arpeggio / Lo-fi Drift / Focus Pulse）、音量、「选择本地文件…」。截图 `qa-shots/05-bgm-panel.png` |
| c | 播放后毛玻璃 + 极光 + 雷达环/粒子/波形 | PASS（渲染层） | `body.ambience-on` 生效；音符变 3 条 EQ；整屏 scrim `bg-surface 58%` + `backdrop-filter: blur(9px)`；canvas 上同心雷达环（带径向频谱刻度）+ 粒子星丛 + 细波形清晰可见。截图 `qa-shots/07-seek-test.png`、桌面网关 `qa-shots/09-desktop-18790-play.png` |
| d | 浏览器 JS 核验播放态 | PASS | AudioContext 进入播放态；播放中时间码持续推进（0:24→0:26、0:59 等）；canvas 持续绘制（litPx 随时间变化，非空白）。**但见 D1：`--amb-amp` 能量恒 0** |
| e | 进度寻址 / repeat 三态 / shuffle | PASS | 点进度条比例位置 → currentTime 成比例跳转（点约 70% 落到 0:59 / 1:25）；repeat `off→all→one→off` 循环正确（aria-label 验证）；shuffle `false→true` 切换正确 |
| f | 暂停后毛玻璃/可视化退回 | PASS | 暂停约 2s 后 `body` 去掉 `ambience-on`、`.amb-scrim--active` 与 `.amb-bg--active` 均失活，磨砂/雷达环/极光全部退回清晰，顶栏 EQ 恢复为音符。截图 `qa-shots/08-after-pause.png` |
| g | reduced-motion 收敛 | PASS | CDP `Emulation.setEmulatedMedia(prefers-reduced-motion: reduce)` 后 `matchMedia(...).matches===true`，`canvas.music-viz` computed `display:none`，动效收敛，无报错 |
| h | 控制台 / 网络 / 同源 | PASS | 音频走同源 `/control/static/music/*.mp3`（200）与 `/control/splash/bg.mp4`（200）；无 CSP 违规、无外网请求、无音频 404。既有噪声 `/api/system/update`、`/api/elevated-mode` 为前端已正确降级的 501（AGENTS.md R57），不计缺陷 |

---

## 3. 第三部分：Tauri 桌面端 WebView2 闭环

1. 优雅停止旧桌面端：marguerite-gui PID 41548 + python 侧车 PID 24060 均已终止，18790 释放。
2. 备份旧 `webui/dist`（1476 files）→ `dist.bak-20261020`（回滚件保留）；新构建（303 files）复制为 `webui/dist`。
3. 重启 `marguerite-gui.exe`，新进程 PID 26152，18790 正常监听。
4. 部署校验（实测）：
   - 桌面网关 `/control/` 返回的入口 JS 哈希 = `assets/index-Dy7cKsNx.js`，与本次 `npm run build` 产物逐字一致 → **桌面端确已加载新构建**。
   - `/control/static/music/dusk-pad.mp3` → 200（1051107 B）；`/control/splash/bg.mp4` → 200（13295630 B）。
   - 在桌面网关 URL（即 WebView2 实际加载的同一地址）冒烟：播放中 `body=ambience-on`、时间码推进，雷达环 + 粒子 + 磨砂 + 顶栏 EQ 全部渲染（截图 `qa-shots/09-desktop-18790-play.png`）。

> 说明：本子代理无法直接驱动 cu 桌面平面（该能力限 MainAgent），故 WebView2 内目检改用「访问桌面网关 18790 的同一 URL」等效验证——Tauri 壳本身只是加载 `http://localhost:18790/control/` 的 WebView2 容器，而 WebView2 为 Chromium 内核，`backdrop-filter` 渲染与已验证的浏览器一致。如需 MainAgent 做一次真实 WebView2 窗口内目检（确认窗口内 blur 观感与降级行为），建议补做。

**桌面端结论：部署成功、新构建生效、音乐功能在桌面网关下正常，进程健康。**

---

## 4. 发现的问题（独立视角）

### D1（P1）可视化并非真正音频驱动——能量通道恒为 0
- **现象**：播放全程在 `:root` 上读到的 `--amb-amp` 恒为 `0.000`；aurora 透明度停在 `calc(0.5 + 0*0.35)=0.5` 基线；雷达环只在静态 0.25 基线绘制，不随 bass 扩张、beat 不闪、频谱刻度不随实时频谱起伏。
- **根因（代码路径）**：`startAudioReactivity()` 在整个生产代码中**仅一处调用**——`useBgm.ts:396`，位于 `initBgm()` 内、`audioEngine.unlock()` 之前。此时 AudioContext 尚未创建（`audioEngine.supported()===false`），函数早退（`if (running || !audioEngine.supported()) return`），`running` 从未置真；而 `unlock()` 创建/恢复 ctx 之后再无任何地方重启 rAF 分析循环（grep 确认无第二处生产调用点）。
- **后果**：`bass/energy/beat` refs 恒为初始 0。可视化「看起来在动」（粒子经 `sin(time)` 闪烁），但**没有真正随音频呼吸**，与「音频驱动的同心雷达环+粒子星丛」的功能承诺不符。
- **建议**：在 `unlock()` 成功创建/恢复 ctx 后补调一次 `startAudioReactivity()`（或让 `unlock()` 成功回调里启动循环），由 dev 复核 `--amb-amp` 随音量起伏。

### D2（P3 / 文案）启动页 logo 拼写错误
- `views/SplashView.vue:184` logo 渲染为 `MARGGERITE`，正确应为 `MARGUERITE`（多了一个 G、缺 U）。

### D3（P3 / 文案）启动页 HUD 版本号过期
- `views/SplashView.vue:156` HUD 硬编码 `v0.2.7 · R67 · DEEPSEEK V4.1 FLASH`，实际版本已是 `0.2.13`。应改为单一事实源注入或删除硬编码。

### D4（P3 / 路由）直连深链 `/control/splash` 返回 404
- SPA fallback 覆盖 `/control/chat` 等深链，但不覆盖 `/control/splash`；首访走根路由重定向正常，仅「手动刷新/直连 splash」边界 404。

---

## 5. 总体发布判断

**conditional_go（有条件放行）**。

依据：
- 门禁全绿，核心播放体验（播放/暂停、transport、进度寻址、repeat 三态、shuffle、曲目列表、音量、本地文件选择、暂停原路退回、reduced-motion 收敛、同源音频无 CSP 问题）全部真机通过；桌面端新构建已部署生效、进程健康。
- 无 P0 阻断项；不阻塞当前发版，但建议在对外宣传「音频驱动可视化」前修复 **D1**（一行级修复：unlock 后补启分析循环），并顺手修掉 **D2/D3** 两处启动页文案；**D4** 可纳入后续 SPA fallback 补全。
- 回滚件 `C:\Users\Administrator\AppData\Local\Marguerite\webui\dist.bak-20261020` 保留；如桌面端异常可将 `dist` 删除后把备份改名回 `dist`。

---

## 6. 收尾状态
- 独立网关 18899（PID 12060）已停止；临时浏览器标签已暂停播放。
- 用户桌面端 marguerite-gui PID 26152 正常运行，18790 监听中。
- 截图存档：`docs/qa-shots/`（随本仓库发布）。

---

# 附：D1–D4 修复后独立复核（2026-10-20 第二轮）

复核方式：新起独立网关 `--port 18899`（PID 35836，已停），bu 平面浏览器真机访问；桌面端当前 PID 39180 占 18790 未动。不看「动画在动」，专看音频能量通道是否真闭环。

| 项 | 判定 | 证据 |
|---|---|---|
| **D1 音频驱动可视化** | **PASS（已修复）** | 播放态连续采样 `--amb-amp`：`0.056 / 0.024 / 0.117 / 0.185`，密采样段 `0.115 / 0.167 / 0.125 / 0.182 / 0.116 / 0.137`——非零且随时间明显波动；点击暂停后连续 5 次采样恒为 `0.000`，同时 `body` 去掉 `ambience-on`、磨砂/雷达环退回清晰。播放↔暂停形成干净对照，证明能量分析循环已在 `unlock()` 后重启、并真实驱动可视化，而非上次的静态 0.25 基线。截图 `qa-shots/11-recheck-play.png`（播放：雷达环+粒子+磨砂+EQ）、`qa-shots/12-recheck-pause.png`（暂停：退回清晰）。 |
| **D2 启动页 logo 拼写** | **PASS（已修复）** | `.hud-title` 与 `.splash-logo` 均渲染 `MARGUERITE`（原 `MARGGERITE` 已更正）；源码 `SplashView.vue:167/196` 字面量为 `MARGUERITE`。截图 `qa-shots/13-recheck-splash-text.png`。 |
| **D3 启动页版本号** | **PASS（已修复）** | `.hud-sub` 运行时渲染 `v0.2.13`，与 `/api/health` 返回 `version=0.2.13` 一致；源码 `SplashView.vue:15-16` 改为从 `status` RPC 读 `version`，网关不可达时降级为 `agent workspace`，不再有硬编码 `v0.2.7·R67·DEEPSEEK V4.1 FLASH`。 |
| **D4 直连深链 /control/splash** | **PASS（已修复）** | `GET /control/splash` → 200（text/html）；资源目录已改名 `boot/`：`/control/boot/hero.mp3` → 200（3958796B）、`/control/boot/bg.mp4` → 200（13295630B），与路由同名冲突消除。 |

**回归项（仍正常）**：repeat 三态 `循环关闭→列表循环→单曲循环→循环关闭` ✅；shuffle `aria-pressed true→false` 切换 ✅；暂停后磨砂/可视化原路退回 ✅；reduced-motion（CDP 模拟 `prefers-reduced-motion: reduce`）时 `.music-viz` canvas `display:none`、动效收敛 ✅；控制台仅 1 条 404（favicon/导航取消类噪声），无 CSP 报错、无音频 404、无外网请求。

**复核后总体发布判断：go。** D1 真音频驱动闭环已用播放/暂停双态数值对照证实，D2–D4 文案与路由缺陷均已修复，回归无回退，桌面端新构建健康运行（PID 39180）。

第二轮收尾：独立网关 18899（PID 35836）已停；桌面端 marguerite-gui PID 39180 正常运行，18790 监听中。
