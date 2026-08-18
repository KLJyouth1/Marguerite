# Phase C — 桌面能力 进展与方案

> 状态：A4.4 已完成；C1 已升级为 `notify` 实时监听（core 614 passed）；C2/C3/C4 已在 Tauri GUI 壳中编码落地并通过 `cargo check -p marguerite-gui` 类型验证（托盘 + 全局快捷键 + 原生菜单）。
> 全程未做任何 git 提交（用户要求）。

## 一、A4.4 真实引擎埋点（已完成 ✅）

| 项 | 改动 | 验证 |
|----|------|------|
| `EventBus` | 加 `#[derive(Debug, Clone)]`，允许被 `AgentRuntime` 持有 | 编译通过 |
| `AgentRuntime` | 新增 `events: Option<Arc<EventBus>>` + `event_session` + `set_event_bus` + `emit`；在 `process_input` 的 思考开始 / 完成 / 错误 三处发布 `Envelope` | `test_agent_runtime_publishes_thinking_and_done` ✅ |
| `RealEngine` | 新适配器实现 `EngineRunner`：`from_config(config)` + `run_turn`（包装 `AgentRuntime` 并注入总线与 session） | `test_real_engine_publishes_events` ✅ |
| `WorkbenchApp::from_config` | `MockEngine` → `RealEngine::from_config(config.clone())`（保留 `MockEngine` 供单测） | `test_from_config_builds_ok` ✅ |
| **TUI 订阅双前缀 bug** | 订阅由 `ws.{session}.*`（session 已是 `ws.<uuid>` → `ws.ws.<uuid>.*`）修正为 `{session}.*`，使真实事件真正回流 UI | 全量 615 passed |

> 核心收益：工作台不再是假事件流，而是真实 LLM 调用经 `AgentRuntime` 发布的 `Thinking → Done/Error` 事件链。

## 二、Phase C 能力清单（用户已选 B：放开 `Cargo.toml` 约束）

| # | 能力 | 当前状态 | 落地位置 / 依赖 |
|---|------|----------|------------------------|
| C1 | 工作区文件监听（变更经事件总线回流 TUI） | ✅ 已完成（实时，`notify` 递归 watch） | `core/src/tui/workbench/workspace_watcher.rs` + `notify` |
| C2 | 全局快捷键（Cmd/Ctrl+Shift+Space 切换窗口） | ✅ 已编码 | Tauri GUI：`tauri-plugin-global-shortcut`，`cargo check` 验证中 |
| C3 | 系统托盘图标 + 右键原生菜单 | ✅ 已编码 | Tauri GUI：`tauri` `tray-icon` feature + `TrayIconBuilder` |
| C4 | 原生菜单（托盘菜单：显示/隐藏/退出） | ✅ 已编码 | Tauri GUI：`tauri::menu` |

> 决策依据：托盘/全局快捷键/原生菜单本质上属于 GUI 壳能力，而 `gui/src-tauri` 脚手架已存在（Phase D 的 Tauri GUI）。故 C2/C3/C4 直接落在 Tauri 后端 `lib.rs`，而非 CLI/TUI 形态下另起独立托盘进程——更省依赖、更贴近最终架构。

## 三、C1 实现要点（已升级为 `notify` 实时）

- 文件：`core/src/tui/workbench/workspace_watcher.rs`（已在 `mod.rs` 注册 `pub mod workspace_watcher`）。
- 行为：`WorkspaceWatcher::new(root).with_interval(2s)` 后台 tokio 线程运行，`notify::RecursiveWatch` 监听整个工作区（忽略 `.git`/`target`/`node_modules`/`.workbuddy`），事件去重后映射为 `Created/Modified/Deleted`，经 `EventBus` 发布 `Status{kind:"workspace.changed"}`。
- 接线：`WorkbenchApp::new` 中 `rt.spawn(watcher.run(bus, session))`，事件路由键 `ws.<uuid>.status` 命中 TUI 订阅。
- 测试：`test_ignores_noise_dirs` / `test_publish_on_create` / `test_publish_on_modify` 通过（core 全量 614 passed）。
- 改进：相比早期 mtime 轮询，`notify` 对「同秒改写」不再漏检。

## 四、C2/C3/C4 实现要点（Tauri GUI 壳）

- 文件：`gui/src-tauri/src/lib.rs`（重写），`gui/src-tauri/Cargo.toml`（加 `tauri-plugin-global-shortcut`，`tauri` 开 `image-png` feature，`tokio` 锁定 `1.44.2`），`gui/src-tauri/capabilities/default.json`（加 `global-shortcut:default`）。
- 托盘：`TrayIconBuilder::with_id("main-tray")` 经 `Image::from_path(resource_dir/icons/32x32.png)` 加载图标（需 `image-png` feature 解码 PNG），右键菜单含「显示工作台 / 隐藏到托盘 / 退出」（`tauri::menu`）；左键单击切换窗口显隐。
- 全局快捷键：`setup` 中 `app.global_shortcut().on_shortcut("CmdOrCtrl+Shift+Space", handler)` 注册，`ShortcutState::Pressed` 时调用 `toggle_window`。
- 命令：`toggle_window` / `show_window` / `hide_window` 暴露给前端，经 `invoke_handler` 注册。
- 健壮性：`setup` 中托盘/快捷键初始化失败仅 `eprintln!` 不影响主窗口启动。
- 验证：`cargo check -p marguerite-gui` 通过（仅 `core` 既有 dead-code 警告）；完整 `cargo build`/运行需 MSVC + WebView2 工具链，属环境门槛，代码已就绪。

## 五、下一步

`cargo check -p marguerite-gui` 验证通过后，Phase C 全部能力落地；随后转入 Phase D（Tauri GUI 前端与壳深度整合）/ Phase E（自进化）/ Phase F（发布）。
若 GUI 因环境缺 MSVC/WebView2 工具链而无法完整构建，代码已就绪，待具备 Tauri 工具链时一键 `npm run tauri dev` 验证。
