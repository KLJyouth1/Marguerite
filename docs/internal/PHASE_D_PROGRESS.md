# Phase D — GUI 前端与内核深度整合 进展

> 状态：Phase D 已完成并验证通过。Rust `cargo check -p marguerite-gui` 通过；前端 `npm run build` + `npx tsc --noEmit` 零错误；core 回归 614 passed / 0 failed。
> 全程未做任何 git 提交（用户要求）。
> 前置：A4.4（真实引擎埋点）已完成；Phase C（托盘/全局快捷键/菜单 + 实时文件监听）已完成。

## 一、目标

把 `core` 的对话引擎（`RealEngine` / `AgentRuntime`）真正接进 Tauri GUI，让 React 界面成为引擎的第一公民 UI（与 CLI / TUI 三态并列，复用同一内核）。

## 二、打通架构

```
React 界面
  └─ invoke("run_turn", { text, session, mode })
        ↓
Rust 命令 run_turn (gui/src-tauri/src/lib.rs)
  ├─ ModeKind::from_name(mode)        // "yolo"/"agent"/"plan" → ModeKind
  ├─ let bus = EventBus::default()
  ├─ let rx = bus.subscribe("**")     // 全量订阅
  ├─ tokio::spawn: while rx.recv() → app.emit("mgt:{session}", Envelope→JSON)
  └─ state.engine.run_turn(text, mode, session, bus).await   // RealEngine
        ↓
RealEngine::run_turn → AgentRuntime::process_input
  └─ 发布 Envelope: Thinking / Done(summary=回复) / Error
        ↓ (经 bus.publish)
  Tauri 全局事件 "mgt:{session}"
        ↓
React listen("mgt:{session}", handler) → 按 env.type 分流渲染
  ├─ thinking / tool_call / tool_result / progress / status → 右侧事件流
  └─ done(summary) → 左侧助手气泡；error → 错误事件
```

会话隔离：事件名含 `session`（前端 `crypto.randomUUID()` 生成），多会话不串台；`trace_id` 去重防 StrictMode 双监听重复渲染。

## 三、落地文件

| 文件 | 改动 |
|------|------|
| `gui/src-tauri/src/lib.rs` | ① import `EventBus`/`Envelope`/`ModeKind`/`Config`/`EngineRunner`/`RealEngine`；② `AppState` 增 `engine: Arc<dyn EngineRunner>`（`RealEngine::from_config(Config::default())`）；③ 新增 `run_turn` 命令（构造总线 + spawn 转发任务 + 调引擎）；④ `invoke_handler` 注册 `run_turn` |
| `gui/src/App.tsx` | 重写为对话工作台：会话/模式 state、消息流、事件流、`invoke("run_turn")` + `listen("mgt:{session}")`，按 `type` 分流；`Done.summary` 渲染助手回复；`Ctrl/Cmd+Enter` 发送 |
| `gui/src/App.css` | 浅色蓝主题（`--primary: #3B82F6`），两栏布局，事件流分类色（indigo 纯色 `#6366F1` 用于 tool，非紫粉渐变） |
| `gui/vite.config.ts` | 增 `base: "./"`（Tauri 自定义协议加载 dist 需相对路径） |

## 四、P0 合规核对

- 图标：UI 无 emoji 功能图标，发送用文字「发送」；事件类型用文字标签 + 色块边框。✅
- 主视觉：蓝 `#3B82F6` 为主色；无 `Indigo→Pink` 渐变、无发光边框、无毛玻璃模板套路。✅
- 文案：务实中文（「思考中」「调用工具」「工具结果」「进度」「处理中」），无 `Welcome`/`Lorem`/`Sign up` 等占位。✅
- 颜色：全部经 CSS 变量，无硬编码紫粉色值。✅

## 五、验证结果（已全部通过）

1. `cargo check -p marguerite-gui` ✅ —— Rust 命令通道类型正确（`EngineRunner`/`RealEngine`/`EventBus`/`Emitter` 接线无误）。
2. 前端 ✅ —— `npm run build` 产出 `dist/`（index.html + 147KB JS + 3.46KB CSS）；`npx tsc --noEmit` 严格类型零错误（`UnlistenFn`/`crypto.randomUUID`/事件分流类型均通过）。
3. 全量回归 ✅ —— `cargo test -p marguerite-core --lib` → **614 passed / 0 failed**（A4.4 + C1 未因 `tui::workbench` 可见性改动回归）。

## 六、环境门槛（非代码问题）

- 完整 `cargo build` / `npm run tauri dev` 需 MSVC + WebView2 工具链，sandbox 仅到 `cargo check` + `vite build` 级别。
- 引擎真实运行需 `MARGUERITE_API_KEY`（LLM 调用）；无 key 时 `AgentRuntime` 走 `ModelRouter` 失败路径，发布 `Error` 事件，前端可见错误条目——通道本身不依赖 key 即可验证。

## 七、下一步

- 验证全绿后，Phase D 标记完成。
- 后续：Phase E（自进化：记忆台账/经验沉淀）、Phase F（发布：CloudStudio/安装包）。
