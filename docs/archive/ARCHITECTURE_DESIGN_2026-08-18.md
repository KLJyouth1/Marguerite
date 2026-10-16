# Marguerite 架构设计文档 — MVP 三工作流

- 作者：首席架构师 高见远（mvp-dev-expert-team-architect）
- 日期：2026-08-18
- 范围：工作流1（产物同步与统一预览/编辑）、工作流2（Bloom 路由融合可视化）、工作流3（桌面自动化框架）
- 配套文档：PRD_Marguerite_MVP.md（PM）、设计文档（Designer）

## P0 绝对规则合规声明

| 规则 | 落地方式 |
|------|----------|
| Spec 必须锁定一套 SVG 图标库 | 锁定 **lucide**（SVG-first，零 emoji）。源数据来自 `lucide-static@0.577.0`，经 `scripts/sync-icons.mjs` 生成 `webui/src/utils/icons.ts` 的 `ICONS`/`IconName`，与现有手移植实现一致。 |
| 禁止任何 emoji 图标 | 全文档、API 契约、前端组件统一使用 lucide SVG path。新增 RPC/事件/字段命名不使用 emoji。 |
| 技术栈选型必须包含锁定图标库的依赖 | 在 `webui/package.json` 中显式钉入 `lucide-static`（`dependencies`/`devDependencies`），见第 6 节版本锚定。 |

> 现状校验：代码阅读确认 `webui/src/utils/icons.ts` 已是约 80 个图标的手移植 lucide SVG path（`chat`/`home`/`fileText`/`fileCode`/`image`/`externalLink`/`cursor`/`window`/`mouse-*` 等），天然满足“锁定 SVG、禁 emoji”。本 Spec 将其正式固化为工程约束并补一个同步脚本以防漂移。

---

## 0. 现有架构基线（来自代码阅读）

| 区域 | 文件 | 关键事实 |
|------|------|----------|
| 产物台账 | `core/src/tools/file_change.rs` | `ArtifactEntry { id, name, path, kind, size, ts }`；进程级 `Mutex<HashMap<session, Vec<ArtifactEntry>>>`；`record_artifact(session, path, kind)` 用 `format!("artifact:{}:{}", session, path)` 作 id，按 path 去重。 |
| 登记调用点 | `core/src/tools/builtin.rs` | **仅** `write_file`（~L311）与 `edit_file`（~L654）调用 `record_artifact`；`append_file`/`copy_file`/图像与媒体保存工具**未**登记。 |
| RPC 实现 | `core/src/gateway/methods.rs` | `artifacts_list`（~L1559）读 `sessionKey`/`session_key`/`session`，返回 `{artifacts, has_more:false}`（`has_more` 硬编码 false）。`computer_use` 分发在 L315。 |
| SESSION | `core/src/usage/mod.rs` + `core/src/gateway/chat.rs` | `task_local! SESSION`；仅在网关 `chat.rs` turn 内 `.scope()` 注入。`core/src/tui/workbench/state.rs:448` 注释称 TUI 路径从未 set。 |
| 路由事件 | `core/src/gateway/chat.rs` | 已发布 `RouterDecision`（strategy/submode/model/routed_model/source/routing_applied/decision{intent,complexity,needs_llm,subtask_count,rationale}）。 |
| 路由配置 | `core/src/persistence/config.rs` | `ModelRoutingConfig { mode, fusion_model, candidate_models, fixed_model, cross_provider_tiers }`，默认 `mode="direct"`。 |
| 前端产物源 | `webui/src/composables/chat/useSessionArtifacts.ts` | 调 `artifacts.list`（分页 limit≤200），合并 indexed/history/stream。 |
| 前端侧栏 | `webui/src/components/chat/RightSidebar.vue` | 渲染 `props.artifacts`（`RsArtifact{name,path,openable}`），`v-if="a.openable !== false"` 显“应用内查看”，三 tab：tasks/artifacts/changes。无 live 订阅。 |
| 预览分派 | `webui/src/utils/workbench/artifactPreview.ts` | `artifactWorkbenchPreviewKind` → html/markdown/pdf/text/image/unsupported；已含 markdown sanitize、HTML 离线 CSP、相对资源收集。 |
| 事件总线 | `core/src/events.rs` | **无任何 Artifact 事件**（无 server→client 推送）。 |
| 计算机操控 | `core/src/tools/computer_use.rs` | 三层：`ScreenPerception`/`InputControl`/`GuiGrounding`；`OsComputerUse` 基于 os 命令。 |
| 操控闭环 | `core/src/tools/computer_use_loop.rs` | `UiAction`/`HumanizedTiming`/`LoopPolicy`/`Delayer`(Real/Recording)/`VisualLoop`。 |
| 自动化模块 | `core/src/automation/*` | `audit.rs`（快照至 `.marguerite/audit-snapshots`）、`ledger`、`cron`、`long_running`。 |
| Tauri 依赖 | `gui/src-tauri/Cargo.toml` | `tauri=2`、`tauri-plugin-opener=2`、`tauri-plugin-shell=2`、`tokio=1.44.2(features=full)`、`marguerite-core=path`。**尚未引入任何 windows/uiautomation/com crate**。 |
| 图标库 | `webui/src/utils/icons.ts` | 手移植 lucide SVG（见 P0 声明）。 |

---

## 1. 工作流1：产物同步与统一预览/编辑

### 1.1 根因分析（为什么 .md 生成了但侧栏不显示）

根因分三类，按置信度排序：

1. **登记覆盖不全（高置信，主因）**
   `record_artifact` 仅在 `write_file`/`edit_file` 接入口调用。任何经由 `append_file`、`copy_file`、媒体/图像保存工具、或 Agent 旁路写入（如直接 `std::fs`）产出的文件都不入台账。用户看到的 `.md` 若由非 `write_file` 路径落盘，便天然不在 `ArtifactLedger` 中 → 侧栏永远查不到。

2. **无 live 推送 + 索引轮询（高置信）**
   `events.rs` 无 `Artifact*` 事件。前端 `RightSidebar` 数据来自 `useSessionArtifacts` → `artifacts.list` 的**主动轮询/重载**，而非服务端推送。即使某产物被正确登记，在当前 turn 进行中也不会刷新；需在 turn 结束后重新拉取索引才出现。表现即“不显示/延迟显示”。

3. **SESSION 注入路径不完整（中置信，触发条件类）**
   `usage::SESSION` 仅在网关 `chat.rs` turn 内 `scope`。若工具经 TUI 路径（`state.rs:448` 注释“从未 set”）或某些后台任务执行，`SESSION.try_with(...)` 返回 `None`，`record_artifact` 静默 no-op → 漏登记。另：`artifacts.list` 以 RPC body 的 `sessionKey` 反查，若与登记时所用 session 字符串不一致，lookup 落空。

4. **openable 过滤与“所有格式”缺口（中置信）**
   `RightSidebar` 仅当 `openable !== false` 显“应用内查看”；对 image/unsupported 类型不渲染工作区内预览，且 `open_in_system` RPC **尚不存在**（虽 `tauri-plugin-opener` 已就位却未接线到产物）。`artifactPreview` 目前对 docx/pptx/xlsx/psd 等返回 `unsupported` → 既不内联也不提供系统打开入口。

### 1.2 统一预览/编辑架构

目标：所有格式产物“可见”（侧栏出现）+ “可预览”（内联或系统打开）+ “可编辑”（内联编辑 + 系统默认程序双轨）。

**数据面修复（登记统一化）**
- 新增 `core/src/tools/file_change.rs::write_bytes(session, path, bytes, existed)` 作为**唯一**落盘入口，内部调用 `record_artifact` 并发 `ArtifactCreated/Updated` 事件。`write_file`/`edit_file`/`append_file`/`copy_file`/媒体保存工具全部改走 `write_bytes`，消除“按工具遗漏”整类缺陷。
- `SESSION` 不再作为登记唯一来源：工具上下文（registry/workspace）显式携带 `session`，调用点**强制传参**；`try_with` 仅作兜底，`None` 时降级为 workspace 级 session 并打 warn 日志。

**事件面修复（live 推送）**
- `events.rs` 新增 `EventType::ArtifactCreated` / `ArtifactUpdated` / `ArtifactRemoved`，载荷 `{ session_key, id, name, path, kind, size, ts }`。
- `record_artifact`/`write_bytes` 发出事件；网关经现有 `Envelope/EventType` 总线推前端。
- 前端 `useSessionArtifacts` 增加事件订阅分支（复用现有 `streamArtifacts` 合并逻辑），`RightSidebar` 据此 `v-if` 实时增删，无需整页重载。

**预览/编辑双轨**
- `PreviewKind` 扩展：`markdown | html | pdf | text | image | code | office | csv | video | audio | unsupported`。
- 内联可预览：`markdown`(已 sanitize)、`html`(离线 CSP)、`pdf`(pdf.js)、`text`/`code`(CodeMirror 6 内联编辑)、`image`(灯箱)、`csv`(表格)。
- 非 Web 可预览（office/video/audio/unsupported）：工作区内渲染**元数据卡**（lucide 图标 + 文件名 + 大小 + 类型 + 修改时间），提供两个动作：`open_in_system`（系统默认程序打开）与 `reveal`（在文件管理器定位）。不内联渲染二进制。
- **内联编辑**：`text`/`code`/`markdown`/`csv`/`json`/`yaml` 在预览卡内提供“编辑”态，保存 → `artifacts.save` → 回写并重新登记 + 发 `ArtifactUpdated`。
- **系统打开**：`artifacts.open_in_system` → `tauri-plugin-opener.open_path`（已就位，仅接线）。

### 1.3 新增 RPC / 事件（API 草案）

```
# 现有（修正）
GET  /api/v1/artifacts.list        { session_key }  -> { artifacts[], has_more, cursor }   # 真实游标分页，去掉 has_more 硬编码
# 新增
POST /api/v1/artifacts.register     { session_key, path, kind? }        -> { artifact }
POST /api/v1/artifacts.get          { session_key, id }                  -> { id, name, path, kind, size, ts, content? }
POST /api/v1/artifacts.save          { session_key, id, content }        -> { artifact, updated:true }
POST /api/v1/artifacts.open_in_system { session_key, path }             -> { ok }
POST /api/v1/artifacts.reveal        { session_key, path }             -> { ok }
POST /api/v1/artifacts.preview_kind  { session_key, path }             -> { kind, mime, editable, openable }
```
事件：`artifact.created` / `artifact.updated` / `artifact.removed`（server→client，经网关事件总线）。

统一响应包络（沿用现有）：
```yaml
{ code: 0, data: {}, message: "" }
```
错误码：1001 会话未找到；1002 产物不存在；1003 路径越界（非 workspace）；1004 系统打开失败（无关联应用）。

---

## 2. 工作流2：Bloom 路由融合可视化

### 2.1 根因分析（为什么融合状态看不清）

1. **UI 字段暴露不全（高置信）**
   `RouterDecision` 已含 strategy/decision，但会话界面未渲染：每候选模型 **token 消耗**、**候选链顺序**、**并行/串行扇出数**、**融合模型**、**各候选得分/延迟**、**失败转移**。用户只看到一个“router 选了 X”。

2. **无运行时埋点（高置信）**
   `select_model` 基于**启动快照** config（`methods.rs` 已知），运行时改配置不生效；无 RPC 查询当前路由态（存活模型、tier、cross_provider），也无累计指标（延迟/token/成本/回退次数）。

3. **Bloom 融合可视化缺失（高置信）**
   `bloom` 模式 = 多候选模型 + 融合模型组合。UI 没有“扇出→融合”图；执行期**逐候选流式进度**未发事件，无法实时展示 fan-out。

4. **事件粒度不足（中置信）**
   `RouterDecision` 仅决策时一次；执行期候选 start/done/fallback/fusion 无事件 → UI 无法做 live fan-out。

### 2.2 可视化字段与状态模型

新增 `RouterTrace` 状态对象（前端 `BloomRoutingCard` 渲染）：
```
RouterTrace {
  session_key, turn_id, task_id,
  mode,                       # direct | router | ensemble | bloom
  submode,                    # 子策略
  fusion_model,               # 融合模型（bloom/ensemble）
  candidates: [               # 扇出候选
    { index, model, provider, status: pending|streaming|done|error|fallback,
      prompt_tokens, completion_tokens, latency_ms, score?, error? }
  ],
  parallelism,               # 同时并行数
  routing_applied,           # 是否实际走路由
  decision: { intent, complexity, needs_llm, subtask_count, rationale },
  fallback_index,            # 当前回退到第几个候选
  totals: { prompt_tokens, completion_tokens, cost_usd, duration_ms }
}
```
前端组件（lucide 图标，无 emoji）：`BloomRoutingCard`（gitBranch/split/merge/cpu/zap）、会话头状态条 `RoutingStatusStrip`、侧栏 `ModelUsagePanel`。

### 2.3 新增 RPC / 事件（API 草案）

```
GET  /api/v1/bloom.status     { session_key?, turn_id? } -> RouterTrace       # 当前/最近路由态
GET  /api/v1/router.metrics   { session_key?, window? } -> { per_model:[{model,latency_p50,latency_p99,tokens,cost,fallbacks}] }
POST /api/v1/routing.set_mode { mode, fusion_model?, candidate_models?, cross_provider_tiers? } -> { ok, config }  # 运行时切换并持久化
```
事件（server→client）：`router.decision`（扩 `RouterTrace`）、`router.candidate_start {model,index}`、`router.candidate_chunk {model,index,delta}`、`router.candidate_done {model,index,tokens,latency,status}`、`router.fusion {fusion_model,strategy}`、`router.fallback {from,to,reason}`。

> 运行时配置：将 `methods.rs` 的启动快照改为读 `persistence` 中可重载的 `ModelRoutingConfig`；`routing.set_mode` 写回 config 文件并热更新，避免重启生效。

---

## 3. 工作流3：桌面自动化框架（重点）

### 3.1 目标与范围

智能体自主启动并操控本机软件（先从 **WPS PPT** 落地），完全开放边界但**可回放/可审计**。抽象为通用 `DesktopTool` 框架，后续扩浏览器/剪映/3D/办公软件。

### 3.2 分层架构

```
┌─────────────────────────────────────────────────────────────┐
│  WebUI (Vue3)  DesktopAutomation 面板 / 会话内 action 卡片      │
│  lucide 图标: monitor / appWindow / play / square / shield /  │
│  history                                                    │
└───────────────┬─────────────────────────────────────────────┘
                │ Tauri invoke (desktop.* RPC)
┌───────────────▼─────────────────────────────────────────────┐
│  Tauri 网关 (core/src/gateway/methods.rs)  desktop.* 分发      │
└───────────────┬─────────────────────────────────────────────┘
                │
┌───────────────▼─────────────────────────────────────────────┐
│  core/src/desktop (新模块)                                    │
│  ├─ registry.rs   DesktopToolRegistry (app_id -> Adapter)     │
│  ├─ tool.rs      DesktopApp trait + DesktopAction enum        │
│  ├─ adapter/                                                  │
│  │   ├─ com.rs      ComAdapter   (WPS/Office via IDispatch)   │
│  │   ├─ uia.rs      UiaAdapter    (windows 0.62 IUIAutomation) │
│  │   ├─ input.rs    InputAdapter  (复用 computer_use InputControl)│
│  │   └─ hybrid.rs   HybridAdapter (COM miss -> UIA -> input)   │
│  ├─ launch.rs    进程启动/发现 (COM CoCreate / exe / registry) │
│  ├─ audit.rs     动作日志 -> .marguerite/audit-snapshots        │
│  └─ policy.rs    安全策略 (allowlist / 需确认动词)              │
└───────────────┬─────────────────────────────────────────────┘
        ┌───────┼───────────────────────┐
        ▼       ▼                       ▼
   windows 0.62   uiautomation 0.25   tauri-plugin-shell
   (COM/UIA/     (可选高层 UIA)        (exe spawn 兜底)
    process/
    registry)
        │
   ┌────▼────────────────────────────────────┐
   │ 本机应用：WPS PPT(Kwpp) / 浏览器 / 剪映 / 3D │
   └───────────────────────────────────────────┘
```

### 3.3 进程启动 / 发现

- **启动**：优先 COM `CoCreateInstance(ProgID)`（应用自起，无需可见窗口）；否则 `std::process::Command` / `tauri-plugin-shell` 按 exe 路径启动。
- **发现**：`windows::Win32::System::Toolhelp`（`Process32First`/`EnumProcesses`）或 `uiautomation::Process` 枚举进程；按 exe 名 / 窗口类 / 标题匹配。已装路径经注册表解析（`HKEY_LOCAL_MACHINE\SOFTWARE\...`）。
- **WPS PPT 落地**：`CoCreateInstance("Kwpp.Application")` → `Presentations.Add` → `Slides.Add` → 设文本 → `Export`/`SaveAs`。纯 COM，可无头。

### 3.4 跨进程操控（四路）

| 通路 | 适用 | 实现 |
|------|------|------|
| **COM（首选）** | WPS/Office（结构化模型） | `windows::Win32_System_Com` + `Win32_System_Ole` + `Win32_System_Variant`；绑定 `Kwpp.Application` IDispatch。可靠、可脚本。 |
| **UIA / MSAA** | 剪映/浏览器/无 COM 应用 | `windows::Win32_UI_Accessibility`（`IUIAutomation`）；`oleacc` `AccessibleObjectFromWindow` 作 MSAA 兜底。元素树/名称/类匹配 + `send_keys` + Control Pattern（Invoke/Value/Selection）。 |
| **输入模拟** | 3D 视口/游戏/无可达树 | 复用 `computer_use::InputControl`（click/drag/type/key/combo）。 |
| **OCR/定位** | 像素级定位 | 复用现有 `vision_cap` + `GuiGrounding::locate_by_description`。 |

已知 COM 错误与处置：`0x80040154`（类未注册→提示 `wps /regserver`）、`0x80070005`（DCOM 权限）、`0x80080005`（服务器启动失败）。`ComAdapter` 捕获并转可读错误，失败自动降级 UIA/输入。

### 3.5 通用桌面工具抽象

```rust
trait DesktopApp {
    fn capabilities(&self) -> AppCaps;          // com | uia | input | hybrid
    fn launch(&self, args: &[String]) -> Result<AppHandle>;
    fn terminate(&self, h: &AppHandle) -> Result<()>;
    fn list_windows(&self, h: &AppHandle) -> Vec<WindowInfo>;
    fn root_element(&self, w: &WindowInfo) -> Option<UiElement>;
    fn exec(&self, h: &AppHandle, action: &DesktopAction) -> Result<ActionResult>;
    fn com_object(&self, h: &AppHandle) -> Option<ComHandle>;   // COM 专用
}
```
- `DesktopToolRegistry`：`app_id`（wps_ppt/wps_writer/excel/powerpoint/browser/jianying/blender…）→ Adapter。
- 适配器选型：ProgID 已注册且 typelib 已知 → `ComAdapter`；否则 `UiaAdapter`；否则 `InputAdapter`；`HybridAdapter` 串联降级。
- `DesktopAction` 枚举：`UiAction` 超集 + 应用动词（`ppt.add_slide`/`ppt.set_text`/`ppt.export_pdf` 等），编译到 COM 调用或 UIA/输入序列。
- 复用 `computer_use_loop::{VisualLoop, RecordingDelayer}` 走输入模拟路径；复用 `automation::audit` 快照。

### 3.6 安全模型（可回放/可审计）

- **审计日志**：每 `DesktopAction` 写 `.marguerite/audit-snapshots`，含 `ts, app, action, params(脱敏), pre/post 快照(窗口树/缩略图), result, duration`。
- **回放**：审计即命令日志；`desktop.replay(session, from_seq, to_seq)` 重放。`RecordingDelayer` 保证确定性时序。
- **审批闸门（开放但安全）**：`desktop.policy` 默认对**破坏性强动词**（delete/format/send/exec）`require_approval`， benign（type/add_slide）`auto`；用户可切 `auto_all`（信任）/ `deny`。高危动作在 `ChatView` 弹确认。
- **能力约束**：文件写限定 workspace；应用启动限 allowlist（可配）；所有外部进程 spawn 入审计。

### 3.7 Tauri 网关 RPC 集成

```
POST /api/v1/desktop.discover   { hint }                     -> { apps:[{app_id,name,running,capabilities}] }
POST /api/v1/desktop.launch     { app_id, args? }            -> { handle }
POST /api/v1/desktop.describe   { app_id }                   -> { capabilities, windows[] }
POST /api/v1/desktop.action     { session_key, app_id, action } -> { result, audit_seq }   # 经适配器，入审计
POST /api/v1/desktop.action_batch { session_key, app_id, actions[] } -> { results[], audit_seq[] }  # 扇出/串行
POST /api/v1/desktop.snapshot   { app_id, window? }          -> { tree?, screenshot? }     # grounding/校验
POST /api/v1/desktop.replay     { session_key, from_seq, to_seq } -> { ok, replayed }
POST /api/v1/desktop.policy     { op: get|set, policy? }     -> { policy }
POST /api/v1/desktop.audit      { session_key, cursor? }     -> { entries[], has_more }
GET  /api/v1/desktop.apps       -> { registry:[{app_id, adapter, com_progid?}] }
```
实现：新增 `core/src/desktop/*`；`methods.rs` 增加 `desktop.*` 分发；前端 `DesktopAutomation` 面板 + lucide 图标。
**WPS PPT 纵向切片**：`registry` 注册 `wps_ppt → ComAdapter("Kwpp.Application")`；动词 `ppt.create_presentation`/`ppt.add_slide(title)`/`ppt.set_body(text)`/`ppt.export(format, path)` 全部 COM 支撑并审计，端到端验证框架。

### 3.8 新增 Rust crate 评估

- **`windows = "0.62.1"`（必须，仅 `cfg(target_os="windows")`）**：COM + UIA + 进程 + 注册表。`features`：`Win32_System_Com`、`Win32_System_Ole`、`Win32_System_Variant`、`Win32_UI_Accessibility`、`Win32_UI_WindowsAndMessaging`、`Win32_System_Toolhelp`、`Win32_System_Threading`、`Win32_Foundation`。
- **`uiautomation = "0.25.0"`（可选，高层 UIA）**：⚠ 依赖 `windows ^0.37` → 与直接 `windows 0.62` 形成**双 windows 版本**（增加编译体量，无运行时风险）。MVP 建议先用 `windows 0.62` 直接写薄封装 `core/src/desktop/uia.rs`（控 IUIAutomation），待 `uiautomation` 升 windows 后再引入提效。
- **MSAA**：`windows` 直接绑 `oleacc.dll` `AccessibleObjectFromWindow`，无额外 crate。
- **已就位**：`tauri-plugin-opener`(open_in_system)、`tauri-plugin-shell`(exe 兜底)、`tokio 1.44.2`、`serde`、`image`（截图）。
- **非 Windows**：`desktop` 模块以 `feature="desktop"` + `cfg(windows)` 门控，其它平台优雅降级（RPC 返回 `unsupported_platform`）。

---

## 4. 技术选型对比矩阵

### 4.1 桌面自动化操控库（≥3 方案）

| 方案 | 操控力 | 易用性 | 依赖整洁度 | 学习成本 | 风险 | 结论 |
|------|--------|--------|------------|----------|------|------|
| A. `windows` 0.62 直写（COM+UIA+MSAA） | ★★★★★ | ★★ | ★★★★★（单 windows） | ★★★ | 低 | **MVP 首选** |
| B. `uiautomation` 0.25.0 | ★★★★ | ★★★★ | ★（windows 0.37 双版本） | ★★ | 中 | 后续提效可选 |
| C. 仅输入模拟（复用 `computer_use`） | ★★ | ★★★★★ | ★★★★★（零新增） | ★ | 中 | 仅视觉/输入类应用 |
| D. Python pywin32 子进程 | ★★★★ | ★★★ | ✗（打破 Tauri 单二进制） | ★★ | 高（IPC） | 否决 |

**推荐**：A 为主；WPS PPT 仅用 A 的 COM 路径即可，无需 B。

### 4.2 产物预览方案（≥3 方案）

| 方案 | 覆盖格式 | 内联编辑 | 引入成本 | 风险 | 结论 |
|------|----------|----------|----------|------|------|
| A. 现有 `artifactPreview` + open_in_system（补 CodeMirror/pdf.js） | md/html/pdf/text/image/code + 其余系统打开 | 文本类原生 | 低 | 低 | **MVP 首选** |
| B. LibreOffice 无头转 PDF 再预览 | docx/pptx/xlsx 内联 | 否（转后只读） | 高（LO 依赖+延迟） | 中 | 后续可选 |
| C. 专用 Web 组件（pdf.js+CodeMirror+markdoc） | 富编辑 | 强 | 中 | 低 | 与 A 合并采用 |
| D. 仅系统打开、无内联 | 全部（系统） | 否 | 极低 | 不满足需求 | 否决 |

**推荐**：A + C（pdf.js 预览 PDF、CodeMirror 6 编辑代码/文本；office 类走 open_in_system，或后续接 B）。双轨满足。

### 4.3 图标库（≥3 方案，P0 锁定）

| 方案 | 格式 | emoji | 可摇树 | 与现状契合 | 结论 |
|------|------|-------|--------|------------|------|
| **lucide（lucide-static 0.577.0）** | SVG | 无 | ★★★★★ | ★★★★★（已手移植） | **锁定** |
| Material Symbols / @mdi/svg | SVG | 无 | ★★★★ | ★★ | 备选 |
| Heroicons | SVG | 无 | ★★★★ | ★★★ | 备选 |
| Feather（lucide 上游） | SVG | 无 | ★★★ | ★★★ | 备选 |
| Font Awesome（字体） | 字体 | 无 | ★★ | ★ | 不优先 |

**锁定**：lucide。源 `lucide-static@0.577.0`，由 `scripts/sync-icons.mjs` 生成 `icons.ts`；`webui/package.json` 钉入依赖（见第 6 节）。全文档/组件零 emoji。

---

## 5. 可行性警告与风险清单

| 编号 | 风险 | 影响 | 缓解 |
|------|------|------|------|
| R1 | 3D 软件（Blender/剪映/C4D）完全开放操控 | 无 COM、UIA 树常为空（视口即单画布），仅输入模拟+OCR，可靠性低、无结构化模型 | 范围先锁 WPS PPT + 浏览器；3D 标 experimental + 人在环；`policy=require_approval` |
| R2 | WPS 未注册 `Kwpp.Application` ProgID | `0x80040154` | 检测并提示 `wps /regserver`；降级 UIA/输入 |
| R3 | DCOM 权限（`0x80070005`）/ 服务器启动（`0x80080005`） | COM 起不来 | 可读错误 + 降级；文档化 DCOM 配置步骤 |
| R4 | `uiautomation` 引 windows 0.37 双版本 | 编译体量/潜在 API 错配 | MVP 用 windows 0.62 直写；B 延后 |
| R5 | UAC/跨完整性级别 | 非提权进程无法驱动提权窗口（UIA 跨 IL 阻断） | 文档化；必要时 Marguerite 提权运行 |
| R6 | 回放非确定性 | 输入时序/UI 变化致 replay 偏离 | 快照校验 + `RecordingDelayer` 确定性时序 |
| R7 | “完全开放”+auto 策略 | 误删/外发风险 | 默认 `require_approval` 破坏性强动词；allowlist 启动；全量审计 |
| R8 | 跨平台 | desktop 模块 Windows-only | `cfg(windows)` + feature 门控，其它平台降级返回 `unsupported_platform` |
| R9 | 产物 live 推送改造回归 | 事件总线改动影响其它订阅方 | 新增事件用新 `EventType` 枚举值，向后兼容；加网关冒烟测试 |

---

## 6. 版本锚定（真实，调研于 2026-08-18）

### Rust（core / gui）
| crate | 版本 | 来源 | 说明 |
|-------|------|------|------|
| `windows` | `=0.62.1` | crates.io（最新稳定；0.61.2 已 yank） | COM/UIA/MSAA/进程/注册表，仅 Windows |
| `uiautomation` | `=0.25.0` | crates.io（约 1 月前发布） | 可选高层 UIA；注意 windows 0.37 双版本 |
| `tauri` | `2` | 已就位 | GUI shell |
| `tauri-plugin-opener` | `2` | 已就位 | `open_in_system`/`reveal` |
| `tauri-plugin-shell` | `2` | 已就位 | exe spawn 兜底 |
| `tokio` | `1.44.2` | 已就位 | `features=full` |
| `serde` / `image` | 现有 | 已就位 | 序列化/截图 |

### 前端（webui）
| 包 | 版本 | 来源 | 说明 |
|----|------|------|------|
| **`lucide-static`** | `=0.577.0` | npm（SVG 源；vite-plugin-lucide-sprite 实测基准） | **P0 锁定图标库依赖**，供 `sync-icons.mjs` 生成 `icons.ts` |
| `@lucide/vue` | `=0.546.0` | npm（备选组件式） | 若改组件式引入可用 |
| `pdfjs-dist` | 最新 2.16.x | npm | PDF 内联预览 |
| `codemirror` 6（`@codemirror/state`/`view`/`lang-*`） | 6.x | npm | 代码/文本内联编辑 |
| `@types/...` | 配套 | npm | 类型 |

> 锚定策略：Rust 用精确 `=` 锁版避免 0.63/0.26 破坏性；npm 用 `=` 锁 lucide-static 防图标漂移，其余 caret 跟随补丁。

---

## 附录：ADR 索引（建议落地时补 MADR）

- ADR-001：产物登记统一走 `write_bytes` 单一入口（+ live 事件）
- ADR-002：图标库锁定 lucide（lucide-static 0.577.0）+ sync 脚本
- ADR-003：桌面自动化首选 `windows` 0.62 直写 COM/UIA，uiautomation 延后
- ADR-004：桌面安全默认 `require_approval` 破坏性强动词 + allowlist
- ADR-005：Bloom 路由运行时可重载 config + `RouterTrace` 事件族
