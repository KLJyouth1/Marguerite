# Marguerite MVP 架构文档（ARCH）

> 负责人：高见远（首席架构师）｜状态：方案定稿，已落盘
> 配套文档：`docs/PRD_Marguerite_MVP.md`
> 调研依据：已读 `core/`、`webui/`、`gui/` 现有代码 + crates.io / npm 联网核实版本（检索日 2026-08-18）

## P0 绝对规则落地（贯穿全文）

1. **锁定一套 SVG 图标库 = lucide**（lucide.dev）。`webui/src/utils/icons.ts` 当前已是手移植 lucide SVG path 数据（`IconName` 约 80 个，含 `fileText/fileCode/image/externalLink/window/mouse-*` 等），天然满足「禁 emoji」。新增图标一律从 lucide 官方 path 派生了，不得引入 Material / Ant Design / 其它库。
2. 本文档全程**禁用 emoji**（含 API/RPC 命名、错误码、示例、表格）。
3. 技术栈选型**必须包含锁定图标库依赖**：前端新增 `lucide-static`（npm，SVG 字符串源）作为 `icons.ts` 的单一可信来源与 CI 校验，禁止再手写或混用其它图标来源。

---

## 0. 现有架构速记（已读代码确认）

- **三层**：`marguerite-core`（Rust）+ `marguerite-gui`（Tauri v2 WebView shell）+ `webui`（Vue3 + TS）。
- **产物台账**：`core/src/tools/file_change.rs` 定义 `ArtifactEntry{id,name,path,kind,size,ts}` + 进程级 `Mutex<HashMap<session, Vec<ArtifactEntry>>>`（`ArtifactLedger`）。`record_artifact(session,path,kind)` 用 `format!("artifact:{}:{}", session, path)` 作 id 按 path 去重。**只在 `write_file`/`edit_file` 内被调用。**
- **网关 RPC**：`artifacts.list`（methods.rs:1559，按 sessionKey 读 `list_artifacts`）、`workspace.file_changes.*`；事件总线有 `RouterDecision` 事件，但**无 Artifact 事件**（events.rs 已 grep 确认）。
- **SESSION**：`usage::SESSION` task-local，网关 turn 内 `chat.rs:164` 已 `.scope()` 注入；TUI 路径 `state.rs:448` 注释称「从未 set」（风险点）。
- **Bloom 路由**：`router.rs` 的 `select_model` 基于启动快照 `ModelRoutingConfig`（mode/fusion_model/candidate_models/fixed_model/cross_provider_tiers），运行时 `RouterDecision` 事件已含 strategy/submode/model/decision。
- **桌面操控基础已具备**：`tools/computer_use.rs`（三层 `ScreenPerception`/`InputControl`/`GuiGrounding`，`OsComputerUse` 基于 os 原生命令）、`tools/computer_use_loop.rs`（拟人化闭环 `UiAction`/`HumanizedTiming`/`LoopPolicy`/`RecordingDelayer`）、`automation` 模块（audit/ledger/cron/long_running，审计快照落 `.marguerite/audit-snapshots`）。
- **Tauri 插件**：`tauri-plugin-opener = "2"`（open_path/open_url/reveal）、`tauri-plugin-shell = "2"`（Command spawn）。**core 尚未引入任何 windows/uiautomation/ole/com crate。**

---

## 1. 工作流1：产物注册/同步 + 全格式预览与编辑

### 1.1 根因分析（真实断点）
产物侧边栏不显示，根因是**「登记不全 + 无 live 推送 + 前端未订阅刷新」**三段断链，而非单一 bug：

| # | 断点 | 证据 | 影响 |
|---|------|------|------|
| R1 | 登记覆盖不全 | `record_artifact` 仅接 `write_file`/`edit_file`；`append_file`/`copy_file`/`create_directory` 等其它写工具**未调用** | 追加/复制出来的产物不进台账，永不显示 |
| R2 | 无产物事件 | `events.rs` 无 Artifact 事件；`RightSidebar.vue` 只消费 `props.artifacts`，无 live push | 当前会话新生成产物只能靠 `artifacts.list` 轮询/重加载，无实时出现 |
| R3 | 前端刷新机制缺 | `useSessionArtifacts.ts` 调 `artifacts.list` 拉索引，但 ChatView 未在 turn 结束/文件变更后触发重拉 | 即使台账有数据，UI 也不一定刷新 |
| R4 | SESSION 在 TUI 路径可能 no-op | `state.rs:448` 注释「SESSION 从未被 set」；`try_with` 失败时静默丢弃 | 非网关入口产生的产物无法归属 session |
| R5 | 预览仅覆盖部分类型 | `artifactPreview.ts` 分派 html/markdown/pdf/text/image/unsupported；office/音视频/二进制/代码类外程序预览无统一出口 | 「所有格式预览与编辑」目前不成立 |

### 1.2 统一预览/编辑架构（双轨）

```
[Rust core: ArtifactLedger] --record_artifact(全写工具)--> [EventBus: artifact.updated]
        |                                                          |
        v                                                          v
[Gateway RPC: artifacts.list / artifacts.get / artifacts.open_in_system / artifacts.edit_inline]
        |                                                          |
        v                                                          v
[webui useSessionArtifacts (订阅 artifact.updated 事件, 增量合并)]  [Tauri plugin-opener -> 系统默认程序]
        |
        v
[WorkbenchPreview 分派] --> 内联预览组件 (markdown/html/pdf/text/image/office-iframe/audio/video/hex)
                        --> 内联编辑 (markdown 实时存盘回调 write_file / office 走 open_in_system 回写监听)
```

- **轨一（内联）**：`artifactPreview.ts` 扩展分派，新增 `office`（`<iframe>` 嵌；离线不可行时降级为「下载后用系统程序」）、`audio`/`video`（`<audio>/<video>`）、`binary`（hex 查看 + 元信息）。所有内联组件统一经 `lucide` 图标（如 `fileCode`/`image`/`fileText`）渲染类型标识。
- **轨二（系统程序）**：`artifacts.open_in_system` → Tauri `plugin-opener.open_path(path)`，用户用 WPS/VS Code 等编辑；core 通过 `workspace.file_changes` 监听回写，变更后 emit `artifact.updated` 触发内联刷新。

### 1.3 新增/修正 RPC（统一 `code/message/data` 信封，禁 emoji）

```
artifacts.list          GET  /api/v1/artifacts?session_key=&limit=&cursor=   (已有, 补 cursor 分页)
artifacts.get           GET  /api/v1/artifacts/:id                          (返回完整 entry + mime)
artifacts.register      POST /api/v1/artifacts/register  body{session_key,path,kind}  (供未自动登记的写工具显式补登, 修复 R1)
artifacts.open_in_system  POST /api/v1/artifacts/:id/open  (经 plugin-opener 打开, 修复 R2 的回写闭环)
artifacts.edit_inline   POST /api/v1/artifacts/:id/content  body{content}  (内联编辑落盘, 等价于 write_file 但带 artifact 归属)
event: artifact.updated (Envelope/EventType 新增, payload{id,session_key,kind,path,ts}, 修复 R2 无 live push)
```

- 错误码：`ARTIFACT_NOT_FOUND(1404)`、`ARTIFACT_OPEN_FAILED(1502)`、`SESSION_UNKNOWN(1401)`。
- 一致性修正：把 `append_file`/`copy_file`/`create_directory` 等全部写入口统一调用 `record_artifact`（R1）；并把 `record_artifact` 的 `try_with(SESSION)` 改为「取不到则尝试从调用上下文参数透传 session_key」的 fallback（R4）。

### 1.4 R1–R5 预估工时拆分（供 PM 回填 A1 story 子任务）

| 断点 | 修复动作 | 负责层 | 预估工时 |
|------|----------|--------|----------|
| R1 登记不全 | 枚举 `builtin.rs` 全部写工具（write/edit/append/copy/create_directory/move 等），统一接入 `record_artifact`；补单测 | core | 4 人日 |
| R2 无 live 事件 | `events.rs` 新增 `ArtifactUpdated` 事件（Envelope/EventType 变体）；在 `record_artifact` 成功路径 emit，向后兼容 | core | 3 人日 |
| R3 前端未订阅 | `useSessionArtifacts.ts` 订阅 `artifact.updated`，按 identity 增量合并去重；`RightSidebar` 触发重渲染 | webui | 3 人日 |
| R4 SESSION no-op | `record_artifact` 增加显式 `session_key` 参数 fallback；TUI `state.rs` 路径补 set/透传 | core | 2 人日 |
| R5 预览格式不全 | `artifactPreview.ts` 扩展 office/audio/video/binary 分派；新增内联组件 + 图标 + 净化和相对资源收集 | webui | 5 人日 |
| **核心合计** | | | **17 人日** |
| 联调/回归/QA buffer | 跨端联调、回归测试、边界用例 | 双端 | ~3 人日 |
| **总计（对齐 PRD A1 ≈ 4 人月）** | | | **~20 人日** |

> 说明：PM 已据诊断将 A1 上修为 4 人月（RICE 7.50 居首）。上表 17 人日为核心修复，加 ~3 人日 buffer 合计 ~20 人日 ≈ 4 人月，与 PRD 一致。五个断点必须**整体排期**，不可只修显示层。

---

## 2. 工作流2：Bloom 路由融合可视化与状态

### 2.1 根因分析
- `RouterDecision` 事件已发布（strategy/submode/model/routed_model/source/routing_applied/decision{intent,complexity,needs_llm,subtask_count,rationale}/session_key/task_id/turn_id），但**会话 UI 对这些字段可视化不足**：token 消耗、候选链（candidate_chain 故障转移路径）、并行/串行、融合模型、各子任务路由结果均未呈现。
- `select_model` 基于**启动快照** config（methods.rs 已知），缺运行时埋点，UI 拿不到「当前生效配置 vs 决策结果」的对照。

### 2.2 需暴露的可视化字段（新增到 RouterDecision 或新事件）

```
router_state {
  mode,                         // direct/router/ensemble/bloom
  fusion_model,                 // bloom 融合模型
  candidate_models: [..],       // 候选链
  active_strategy,              // 本次实际策略
  sub_decisions: [ {intent, model, complexity, rationale} ],  // 各子任务路由
  token_estimate,               // 预估/实耗 token
  parallel: bool,               // 并行/串行
  latency_ms,                   // 路由耗时
}
```

### 2.3 新增埋点 / RPC

```
event: router.decision    (扩展现有 RouterDecision, 增 token_estimate/latency_ms/sub_decisions)
event: router.metrics     (累计: 各 model 调用次数/失败次数/平均延迟, 供状态条展示)
rpc:   bloom.status       GET /api/v1/router/status?session_key=   (返回当前生效 ModelRoutingConfig + 最近 N 条决策)
rpc:   router.metrics     GET /api/v1/router/metrics               (聚合指标)
```

- UI：在会话头部/侧栏用 lucide 图标（`gitBranch`/`zap`/`boxes`）呈现 mode 与融合状态，不引入 emoji。

---

## 3. 工作流3：桌面自动化框架（重点）

### 3.1 目标与边界
- 智能体自主启动并操控本机软件（WPS/Office PPT、浏览器、剪映、3D、办公软件），**完全开放边界**，但必须**可回放 / 可审计**。
- 先以 **WPS PPT（Kwpp.Application COM）** 落地，抽象为通用 `DesktopAppDriver` 框架。

### 3.2 分层架构

```
[LLM / Agent Plan]
      |
[Rust: desktop_automation 新 crate/module]   <- 复用 automation/audit (快照) + computer_use_loop (拟人闭环)
  |- AppRegistry      进程发现/启动 (plugin-shell Command / COM ProgID)
  |- Driver trait     统一操控接口: launch/activate/focus/quit/shutdown
  |     |- ComDriver      (WPS/Office: IDispatch 调用 Kwpp.Application)        [Windows]
  |     |- UiaDriver      (UIA 树遍历/模式调用, 通用 GUI app)                  [Windows]
  |     |- InputDriver    (enigo 键鼠模拟, 兜底)                              [跨平台]
  |- Perception       截图 + OCR/VLM 定位 (复用 computer_use ScreenPerception)
  |- Recorder         每步动作快照 -> .marguerite/audit-snapshots (可回放)
  |- Policy           权限闸门 / 敏感操作二次确认
      |
[Tauri Gateway: desktop.* RPC]  <->  webui 控制面板 (lucide 图标)
```

### 3.3 进程启动与发现
- **启动**：`tauri-plugin-shell` 的 `Command::new("kwpp")` 或 COM `CoCreateInstance("Kwpp.Application")`；先查 `HKEY_CLASSES_ROOT\Kwpp.Application\CLSID` 确认注册，否则提示 `wps /regserver`。
- **发现**：枚举运行进程（`windows::Win32::System::Diagnostics::ToolHelp` 或 `uiautomation` 的 `process` feature），按 exe 名 / 窗口类名匹配。
- **COM ProgID 实测清单**（WPS/Office）：
  - WPS 文字 `wps.Application` / `Kwps.Application`；表格 `Ket.Application`；演示 `Kwpp.Application`
  - MS Office：PPT `PowerPoint.Application`、Word `Word.Application`、Excel `Excel.Application`
  - 常见错误：`0x80040154` 类未注册（需 `/regserver`）、`0x80070005` 权限（DCOM 配置）、`0x80080005` 服务器执行失败。

### 3.4 跨进程操控三路径

| 路径 | 适用 | Rust 实现 | 精度 |
|------|------|-----------|------|
| COM/IDispatch | WPS/Office（首选，结构化） | `windows` + `IDispatch` invoke（Variant/SAFEARRAY 手动封送） | 高（直接对象模型） |
| UIA（UI Automation） | 通用 GUI（剪映/3D/浏览器） | `uiautomation` crate（control/pattern/event） | 中（依赖可访问性树） |
| MSAA | 老程序兜底 | `windows::Win32::UI::Accessibility` (IAccessible) | 低 |
| 输入模拟 | 以上皆不可时 | `enigo`（键鼠）、`uiautomation` input feature | 低（坐标/热键） |

- **WPS PPT 落地方案**：`ComDriver` 用 `Kwpp.Application` → `Presentations.Open(path)` → `ActivePresentation.Slides.Add` / `Shapes.AddTextbox` / `ExportAs` 等；结构化生成 PPT，**无需截图兜底**，质量最高。

### 3.5 输入模拟 / OCR / 定位
- **输入**：优先 COM/UIA；兜底用 `enigo`（跨平台键鼠）或 `uiautomation` 自带 `send_keys`/`input` feature（仅 Windows）。
- **OCR / 视觉定位**：复用 `computer_use` 的 `ScreenPerception` 截图 + VLM 定位（`GuiGrounding.locate_by_description`），不强制引入本地 OCR 引擎；如需离线 OCR 可后续接 `tesseract`（leptess）作为可选项。

### 3.6 通用桌面工具抽象（`DesktopAppDriver`）

```rust
trait DesktopAppDriver {
  fn kind(&self) -> AppKind;                 // WpsPpt / Browser / Jianying / ThreeD / Generic
  async fn launch(&self, args: &[String]) -> Result<AppHandle>;
  async fn activate(&self, h: &AppHandle) -> Result<()>;
  async fn run_action(&self, h: &AppHandle, action: UiAction) -> Result<ActionReceipt>; // 每步落 Recorder
  async fn quit(&self, h: &AppHandle) -> Result<()>;
}
// 注册表：AppRegistry::register(WpsPpt, Box::new(ComDriver::new("Kwpp.Application")))
```

- 后续「浏览器/剪映/3D」只需新增 `Driver` 实现 + 在 `AppRegistry` 登记，框架不变。

### 3.7 安全模型（完全开放但可审计/可回放）
- **审计快照**：每步 `UiAction` 经 `Recorder` 写入 `.marguerite/audit-snapshots/<session>/<ts>.json`（动作 + 前后屏快照路径 + 进程上下文），复用现有 `automation/audit.rs`。
- **回放**：快照可重放为时间线，UI 用 lucide `history`/`play` 图标展示。
- **权限闸门**（`Policy`）：① 全局开关「允许桌面自动化」；② 敏感动作（删除文件/发送/付费/退出系统）强制二次确认；③ 白名单应用；④ 单次会话可一键「急停」（发 `desktop.emergency_stop`）。
- **可观测**：`desktop.*` 全部动作 emit 事件 + 审计落盘，无黑盒执行。

### 3.8 Tauri 网关 RPC 集成

```
desktop.list_apps        GET  /api/v1/desktop/apps            (已注册 + 运行中)
desktop.launch           POST /api/v1/desktop/launch  body{app_kind, args}
desktop.action           POST /api/v1/desktop/action  body{handle, action: UiAction}
desktop.snapshots        GET  /api/v1/desktop/snapshots?session_key=
desktop.emergency_stop   POST /api/v1/desktop/stop
event: desktop.action_done   (payload{handle, action, receipt, ts})
```

- 复用 `tauri-plugin-shell` 启动进程、`tauri-plugin-opener` 打开文件，不重复造轮子。

### 3.9 新增 Rust crate 评估（核心结论）
- **新增 `windows = "0.62"`**（Windows API 投影，COM/UIA/进程枚举必需）—— 这是工作流3 的硬依赖。
- **新增 `uiautomation = "0.25"`**（UIA 封装，feature: `process,control,pattern,input,event`）—— 通用 GUI 操控。
- **新增 `enigo = "0.5"`**（输入模拟兜底，feature: 默认即可）—— 坐标/热键最后手段。
- COM `IDispatch` 用 `windows` 原生（无需额外 crate），Variant/SAFEARRAY 手动封送。
- OCR：暂不新增 crate（用 VLM 复用 computer_use）；如离线需求再评估 `leptess`（tesseract 绑定）。

### 3.10 三 crate 接入工期与风险（含 Win/macOS 跨平台等价）

> PM 输入点 2：产品需跨 Win/macOS，故下表给出三 crate 工期/风险，**并明确 windows/uiautomation 在 macOS 的等价方案**。

**Phase 1（Windows，WPS PPT 首落地）**

| 任务 | crate/模块 | 工期 | 主要风险 |
|------|-----------|------|----------|
| windows 接入 + COM IDispatch | `windows 0.62.1` | 8 人日 | 元数据生成重（~9 MiB）、MSRV 1.82、仅 Windows |
| UIA 接入（树遍历/模式/定位） | `uiautomation 0.25.0` | 5 人日 | 依赖 `windows ^0.37`，与我方 0.62 需对齐（见下方风险 R-COMPAT） |
| 输入模拟兜底 | `enigo 0.5.0` | 3 人日 | macOS 需 TCC 辅助功能授权；Linux Wayland 实验性 |
| 审计回放闭环 | `automation/audit`（复用） | 4 人日 | 复用现有，风险低 |
| WPS PPT 端到端联调 | 整合 | 5 人日 | COM 注册/权限错误（0x80040154 等） |
| **Phase 1 合计** | | **~25 人日** | |

**macOS 等价方案（随二期或并行）**

| Windows 能力 | macOS 等价 | 实现方式 / crate | 工期 | 备注 |
|--------------|-----------|-----------------|------|------|
| COM/IDispatch（WPS/Office 结构化） | AppleScript / JXA（应用字典） | 外壳调 `osascript` | 5 人日 | WPS Mac 字典弱；优先 Microsoft PowerPoint Mac（字典完善） |
| UIA 树/模式（通用 GUI） | macOS Accessibility（AXUIElement） | `objc2` + AX 调用 | 8 人日 | `uiautomation` crate 仅 Windows，macOS 需独立 Driver 实现 |
| 输入模拟 | 同（enigo 跨平台） | `enigo 0.5.0` | 共享 | macOS 需辅助功能授权（TCC） |
| 进程枚举 | `sysctl` / AppKit | `objc2` / 标准库 | 2 人日 | |

**跨平台架构结论**：`DesktopAppDriver` trait 平台无关，但必须做平台分支（`cfg(windows)` / `cfg(target_os="macos")`）：
- Windows 后端 = COM（`windows`）+ UIA（`uiautomation`）+ `enigo` 兜底；
- macOS 后端 = AppleScript/JXA + Accessibility（`objc2`）+ `enigo` 兜底；
- 输入模拟 `enigo` 共用；
- **COM 无 macOS 等价** → WPS PPT 自动化以 **Windows 为主战场**；macOS 的 PPT 自动化走 Microsoft PowerPoint AppleScript 字典，弱于 COM 路径，必要时降级为输入模拟。

**构建对齐风险（R-COMPAT）**：`uiautomation 0.25` 传递依赖 `windows ^0.37`，而我方直接依赖 `windows 0.62`。验证策略：① 先尝试直接编译（Cargo 允许同一 crate 多版本共存，仅增大构建体积）；② 若 API 冲突，二选一：(a) 用 `windows 0.62` **直接调用 UIA COM 接口**（不依赖 uiautomation 包装层），或 (b) 将 `uiautomation` patch 到兼容 0.62 的修订版。推荐优先 (a)，减少传递依赖面。

---

## 4. 技术选型对比矩阵（≥3 方案/维度，真实数据）

### 4.1 桌面自动化操控库（Windows 跨进程）

| 方案 | 版本(真实) | 优势 | 劣势 | 适用 | 状态 |
|------|-----------|------|------|------|------|
| `windows` + COM/IDispatch | windows 0.62.1 | 结构化、零封装误差、WPS/Office 完美 | 手写 Variant/SAFEARRAY 封送、仅 Win | WPS/Office/有 COM 的程序 | 官方维护，~190k 累计下载 |
| `uiautomation` (UIA) | 0.25.0 | 通用 GUI 树遍历/模式、进程枚举、事件 | 依赖可访问性树、非结构化 app 弱 | 剪映/浏览器/通用 GUI | 最近更新约 1 月前 |
| `enigo` (输入模拟) | 0.5.0 | 跨平台键鼠、简单 | 坐标/热键、无语义、易碎 | 兜底 | 38k 累计下载 |
| `rdev` | 0.5.3 | 键鼠监听+发送 | 偏监听、操控语义弱 | 不优先 | 485k 累计下载 |

**结论**：WPS PPT 用 `windows`+COM（首选）；通用 GUI 用 `uiautomation`；`enigo` 仅兜底。三者共存于 `desktop_automation`。

### 4.2 产物预览方案

| 方案 | 说明 | 优劣 | 选定 |
|------|------|------|------|
| 内联 Web 组件 + 系统程序双轨（现状扩展） | markdown/html/pdf(text)/image/office-iframe/av/hex | 零外部依赖、可控、可审计 | **选定**（MVP） |
| 第三方预览 SDK（如 MS 在线预览） | iframe 嵌 Office Online | 需联网/授权、数据出域 | 否（违反本地优先/审计） |
| 专用预览服务（OnlyOffice/Collabora） | 本地起服务渲染 | 重、部署复杂 | 否（MVP 过度） |

### 4.3 图标库（P0 锁定）

| 方案 | 说明 | emoji? | 选定 |
|------|------|--------|------|
| **lucide**（SVG，手移植 + `lucide-static` 同步） | webui 现有 `utils/icons.ts` 已是 lucide path；锁定为唯一来源 | 否 | **选定（P0 强制）** |
| Material Icons / Ant Design Icons | 其它 SVG 库 | 否 | 否（禁止混用） |
| emoji 字符 | 系统 emoji | **是** | **禁止（P0）** |

**依赖落地**：`webui/package.json` 增加 `"lucide-static"`（npm，SVG 字符串源）作为 `icons.ts` 的 CI 校验源；架构/API 文档一律禁用 emoji。

---

## 5. 可行性警告与风险清单

| 风险 | 等级 | 说明 | 缓解 |
|------|------|------|------|
| R-3D：3D 软件完全开放操控 | 高 | 3D 软件（Blender/C4D）多数无稳定 COM、UIA 树稀疏、Canvas 渲染不可访问 | 先用 `enigo`+VLM 截图定位落地「打开工程/渲染导出」等有限动作；不承诺语义级操控 |
| R-COM 注册/权限 | 中高 | `0x80040154/0x80070005/0x80080005` 常见 | 启动前查注册表 + DCOM 配置引导 + 错误码友好提示 |
| R-UIA 脆弱 | 中 | 窗口类名/结构随版本变 | Driver 多 matcher + 回退到输入模拟；Recorder 全量快照便于排查 |
| R-安全（完全开放边界） | 高 | 自动操控可触达删除/付费/系统设置 | 强制 `Policy` 闸门 + 敏感动作二次确认 + 急停 + 全量审计回放 |
| R-跨平台 | 中 | `windows`/`uiautomation` 仅 Win；Mac/Linux 仅 `enigo` 兜底 | MVP 以 Windows 为主战场，非 Win 降级为输入模拟 |
| R-COMPAT | 中 | `uiautomation 0.25` 依赖 `windows ^0.37`，与 0.62 可能冲突 | 先编译验证；冲突则用 `windows 0.62` 直调 UIA 或 patch uiautomation |
| R-macOS 权限 | 中 | macOS 输入模拟/辅助功能需 TCC 授权 | 首次运行引导授权；授权失败降级提示 |

---

## 6. 版本锚定（真实 crate / npm 版本，调研核实）

| 类别 | 依赖 | 锚定版本 | 来源 |
|------|------|----------|------|
| 桌面 API | `windows` | `0.62.1`（最新稳定；0.61.2 已 yank） | crates.io（检索日 2026-08-18） |
| UIA | `uiautomation` | `0.25.0`（feature: process,control,pattern,input,event） | crates.io |
| 输入模拟 | `enigo` | `0.5.0`（注：keyword 页见 0.6.1，集成前复核） | crates.io |
| Tauri | `tauri` | `2`（现有） | gui Cargo.toml |
| 打开程序 | `tauri-plugin-opener` | `2`（现有，open_path/open_url/reveal） | gui Cargo.toml |
| 启动进程 | `tauri-plugin-shell` | `2`（现有，Command） | gui Cargo.toml |
| 图标库 | `lucide-static` | 新增 npm 依赖（作 icons.ts 同步源） | npm / lucide.dev |
| OCR（可选） | `leptess`（tesseract） | 待评估，非 MVP 必需 | — |
| macOS（二期） | `objc2` + Accessibility | 集成前锚定最新稳定 | crates.io |

> 注：`windows` 与 `uiautomation` 的 `windows` 传递依赖需对齐（uiautomation 0.25 依赖 `windows ^0.37`，而我方直接依赖 `windows 0.62`；构建时以我方 0.62 为准，必要时 pin `uiautomation` 到兼容 0.62 的修订版或打 patch，详见 §3.10 R-COMPAT）。

---

## 7. 交付与协作要点

- **给 PM**：工作流1 修复含 5 个根因（R1–R5），需整体排期（§1.4 工时拆分已给）；工作流3 建议分两期（WPS PPT 一期 ~25 人日 / 通用框架+浏览器剪映二期），macOS 跨平台等价方案见 §3.10。
- **给 Designer**：图标一律走 lucide（`IconName`），新增产物类型/路由状态/自动化面板 UI 请复用既有 lucide 图标，**严禁 emoji**；预览双轨（内联 + 系统程序）的交互需明确「应用内查看 / 用系统程序打开」两按钮。
- **给 team-lead**：以上为架构方案，已落盘 `docs/ARCH_Marguerite_MVP.md`，与 `docs/PRD_Marguerite_MVP.md` 配对。待 PM 排期、Designer 出 UI 后进入实现。版本锚定已用真实数据。

---

## 附录 A：OpenAPI 3.0 草案（新增/修正 RPC）

```yaml
openapi: 3.0.3
info:
  title: Marguerite API (MVP)
  version: 1.0.0
paths:
  /api/v1/artifacts:
    get:
      summary: 列出会话产物（补 cursor 分页）
      parameters:
        - name: session_key
          in: query
          required: true
          schema: { type: string }
        - name: limit
          in: query
          schema: { type: integer, default: 200 }
        - name: cursor
          in: query
          schema: { type: string }
      responses:
        '200':
          description: 成功
          content:
            application/json:
              schema:
                type: object
                properties:
                  code: { type: integer, example: 0 }
                  data:
                    type: object
                    properties:
                      items:
                        type: array
                        items: { $ref: '#/components/schemas/ArtifactEntry' }
                      has_more: { type: boolean }
                      next_cursor: { type: string }
                  message: { type: string }
  /api/v1/artifacts/register:
    post:
      summary: 显式补登产物（修复 R1）
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              required: [session_key, path, kind]
              properties:
                session_key: { type: string }
                path: { type: string }
                kind: { type: string, enum: [created, modified, copied] }
      responses:
        '201': { description: 已登记 }
        '1401': { description: SESSION_UNKNOWN }
  /api/v1/artifacts/{id}/open:
    post:
      summary: 用系统默认程序打开（修复 R2 回写闭环）
      parameters:
        - name: id
          in: path
          required: true
          schema: { type: string }
      responses:
        '200': { description: 已打开 }
        '1404': { description: ARTIFACT_NOT_FOUND }
        '1502': { description: ARTIFACT_OPEN_FAILED }
  /api/v1/artifacts/{id}/content:
    post:
      summary: 内联编辑落盘（等价于 write_file 但带 artifact 归属）
      parameters:
        - name: id
          in: path
          required: true
          schema: { type: string }
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              required: [content]
              properties:
                content: { type: string }
      responses:
        '200': { description: 已保存 }
  /api/v1/desktop/launch:
    post:
      summary: 启动本机应用（桌面自动化）
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              required: [app_kind]
              properties:
                app_kind: { type: string, enum: [WpsPpt, Browser, Jianying, ThreeD, Generic] }
                args: { type: array, items: { type: string } }
      responses:
        '200': { description: 已启动, 返回 handle }
  /api/v1/desktop/action:
    post:
      summary: 对应用执行一步动作
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              required: [handle, action]
              properties:
                handle: { type: string }
                action: { $ref: '#/components/schemas/UiAction' }
      responses:
        '200': { description: 动作完成, 返回 receipt }
  /api/v1/desktop/stop:
    post:
      summary: 急停（安全闸门）
      responses:
        '200': { description: 已急停 }
  /api/v1/router/status:
    get:
      summary: 当前路由状态（Bloom 可视化）
      parameters:
        - name: session_key
          in: query
          schema: { type: string }
      responses:
        '200': { description: 成功, 返回 router_state }
components:
  schemas:
    ArtifactEntry:
      type: object
      properties:
        id: { type: string }
        name: { type: string }
        path: { type: string }
        kind: { type: string }
        size: { type: integer }
        ts: { type: integer }
        mime: { type: string }
    UiAction:
      type: object
      properties:
        type: { type: string, enum: [Click, DoubleClick, RightClick, TypeText, Key, Scroll, Move, Locate] }
        x: { type: integer }
        y: { type: integer }
        text: { type: string }
        key: { type: string }
        dx: { type: integer }
        dy: { type: integer }
        description: { type: string }
        auto_click: { type: boolean }
    RouterState:
      type: object
      properties:
        mode: { type: string }
        fusion_model: { type: string }
        candidate_models: { type: array, items: { type: string } }
        active_strategy: { type: string }
        sub_decisions: { type: array, items: { type: object } }
        token_estimate: { type: integer }
        parallel: { type: boolean }
        latency_ms: { type: integer }
```

（文档完）
