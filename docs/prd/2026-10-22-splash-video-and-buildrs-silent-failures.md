# PRD：启动动画（bg-v2.mp4 + 跳过按钮）+ build.rs 静默失败清理（S 档）

- 日期：2026-10-22
- 状态：执行中
- 触发：用户明确要求；改动进安装包（`webui/dist` + `build.rs`）

## 1. 问题

### 1.1 启动动画

现状（`webui/src/views/SplashView.vue`）：
- `BG_VIDEO = '/boot/splash-cyber.mp4'` —— **该文件不存在**（注释自承"personal-use, NOT shipped"），
  所以启动画面实际只有极光渐变，没有视频。
- `webui/public/splash/bg-v2.mp4`（4.4 MB）已提供但**未被引用**。
- SplashView 渲染在 `<main class="content">` 内，而 App.vue 的 `sidebar` 与 `topbar`
  （含自绘标题栏 `TitleBarControls`，`decorations:false`）在 main **之外** ⇒
  当前 splash **盖不住侧栏/顶栏**，不是"覆盖整个壳"。
- 退出路径只有"整屏点击"与 6 秒自动跳转；**无明确跳过按钮**，且整屏 `div` 无
  `role`/`tabindex` ⇒ **键盘用户无法跳过**（WCAG 2.1.1 键盘可达）。

### 1.2 build.rs 静默失败

实测 `gui/src-tauri/build.rs` 有 **8 处代码级 `let _ =`**（另有 2 处仅在注释里）。
关键事实：下游 `copy_dir_all` / `ensure_synced` / `verify_pyengine_sync` **只检查
"源有、目标无"**，而"删除目标目录失败"的后果恰恰是 **"目标有、源无"（旧文件残留）**
⇒ 该失败**完全静默**，产出"看起来打完、实际混合了旧文件"的包。

本项目 2026-10-22 就撞过一次同类现象（打包目录 765 文件 vs 源码 306、入口哈希中途变化）。

## 2. 目标

- **G1**：`bg-v2.mp4` 作为全屏启动动画，真正覆盖侧栏与顶栏。
- **G2**：右上角透明跳过按钮 + Esc 键，键盘可达、读屏可播报。
- **G3**：build.rs 的目录清理/重建失败**不再静默**，改用"明确原因"取代下游的"资源不存在"。

## 3. 非目标

- **NG1**：不改启动动画的视觉设计（极光、HUD、波形、音乐全部保留）。
- **NG2**：不移除 `bootSequence.ts` / `BootOrchestra.vue`（另一套组件，与本视图无关）。
- **NG3**：不把 `set_permissions` 的 `let _ =`（build.rs:506）改成硬失败 —— 它是
  `remove_dir_all_force` 的**前置辅助**，真正判据是其后的 `remove_dir_all` 返回值；
  它失败会体现在那条返回里，不构成静默。**但必须补注释说明，否则下一个人会以为是漏改。**
- **NG4**：不对视频做转码/压缩。4.4 MB 可接受；播放失败自动回退极光（既有逻辑保留）。

## 4. 验收口径

| 编号 | 判据 |
|---|---|
| AC1 | `SplashView` 引用 `/splash/bg-v2.mp4`；构建产物 `dist/splash/bg-v2.mp4` 存在 |
| AC2 | splash 路由下 sidebar / topbar / TitleBarControls **均不渲染** |
| AC3 | 存在 `button` 语义的跳过按钮，`aria-label` 明确，键盘 Tab 可达、Enter/Space 生效 |
| AC4 | Esc 键可跳过 |
| AC5 | 视频 `error` 事件 → 回退极光（`videoOk=false`），不出现破图 |
| AC6 | 单测覆盖：跳过按钮触发跳转、Esc 触发跳转、视频 error 回退 |
| AC7 | build.rs 全部 8 处 `let _ =` 有明确归属：或改为收集失败，或注释说明为何可吞 |
| AC8 | **变异测试**：让目标目录不可删除（设为只读+占用）→ 构建期必须报出"清理失败"而非静默 |
| AC9 | `cargo check -p marguerite-gui` exit 0；webui `typecheck` + 相关单测通过 |

## 5. 影响面

- 消费方：`webui` 路由与 App 外壳、`gui/src-tauri/build.rs`、安装包内容
- 不受影响：core、pyengine、TUI

## 6. 回滚

还原 `SplashView.vue`、`App.vue`、`sharedRoutes.ts`、`build.rs` 四个文件。
