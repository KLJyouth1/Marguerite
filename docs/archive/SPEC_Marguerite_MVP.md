# Spec - Marguerite MVP v1.0

> 生成日期：2026-08-18
> 基于：PRD_Marguerite_MVP.md（v1，PM 许清楚） + ARCH_Marguerite_MVP.md（架构师 高见远） + UIUX-Redesign.md（设计师 颜好看）
> 状态：已确认（用户确认「全部 P0 进入实现」）
> 范围裁决：B1（Bloom 可视化）维持 P0（用户显式诉求优先于 RICE 排序）；Epic C 全 P0，WPS 先验证、再抽象通用框架仅作实现顺序，不削减范围。

---

## 1. 产品定义
- **一句话描述**：本地优先的桌面 AI 工作台——会话中产出的任意格式文件可预览/编辑，Bloom 路由融合过程可视化，且智能体可自主操控本机软件（WPS/浏览器/剪映/3D/办公）并全程可回放可审计。
- **目标用户**：重度效率用户 / 个体创作者 / 小微企业主 / 研究员，日常在本机重度使用多款桌面软件，愿授予「完全开放」权限但要求可回放可审计，不信任文件上云。
- **核心问题**：① 已生成产物不在侧边栏显示；② 产物预览/编辑能力不全；③ Bloom 路由状态对使用者不可见；④ 缺少让 AI 真正操控本机软件做跨应用任务的能力。

## 2. MVP 范围（锁定——不在此列表的功能一律不做）

| 优先级 | Epic | 功能 | 验收标准摘要 |
|--------|------|------|-------------|
| P0 | A | A1 产物登记/同步整体修复（R1–R5 五段断链） | 会话生成任意格式文件即进台账并实时显示 |
| P0 | A | A3 轻量内联编辑（.md/.txt/.csv） | 应用内直接改并落盘 + 版本 |
| P0 | A | A4 系统程序打开（.docx/.pptx/.xlsx → WPS/Office） | 调起系统默认程序并回写登记 |
| P0 | A | A2 全格式预览（简版：支持格式内联 + 不支持则系统打开兜底） | 空状态消除，无「只显示不支持」 |
| P0 | B | B1 Bloom 路由融合可视化 | 候选链/并行串行/token/融合归属可见 |
| P0 | B | B2 气泡/消息 redesign | 用户/助手/系统分层、引用高亮、有意义空态 |
| P0 | B | B3 thinking 过程可视化 | 可折叠推理 + 路由/模型标注 + token |
| P0 | B | B4 工具调用卡片 | 工具名+参数摘要+状态+产物引用脚注 |
| P0 | B | B5 产物引用 inline | 消息内引用可点击跳转预览/编辑 |
| P0 | C | C1 通用桌面自动化框架 DesktopAppDriver | 启动/发现/操控/录制/闸门统一抽象 |
| P0 | C | C2 WPS PPT 自动化（Kwpp.Application COM） | 自然语言→调起 WPS 演示→生成可编辑 .pptx |
| P0 | C | C3 可回放/可审计 | 每步快照 + 时间轴回放 + 导出日志（复用 automation/audit） |
| P0 | C | C4 安全边界（完全开放） | kill-switch + 敏感二次确认 + 注入扫描 + ledger |
| P1 | A | A5 产物列表管理（筛选/重命名/删除/下载） | 列表增删改查 |
| P1 | B | （B 系列在 P0 已含，无独立 P1 项） | — |
| P1 | C | C5 浏览器/剪映/3D 适配器 | 框架成熟后接入 |

## 3. 明确不做（Out-of-Scope — 锁定）

| 不做的功能 | 原因 | 何时考虑 |
|------------|------|----------|
| 云端多端同步/多人协作编辑 | 本地优先定位 | 独立 Epic |
| 自研大模型 | 用 Bloom 路由融合现有模型 | 永不（除非战略调整） |
| 移动端 | Tauri 桌面优先 | v2 |
| 影刀式可视化低代码设计器 | 用自然语言 + 审计回放 | 永不 |
| Office 协同编辑/评论 | 只做生成+预览+轻编辑 | v2 |
| 长尾软件专用适配器 | 先抽象通用框架 | C5 之后 |
| 无审计黑盒自动执行 | 必须可回放 | 永不 |
| 3D 软件语义级操控 | 缺稳定 COM/UIA 树稀疏 | 仅有限动作 |
| 引入紫→粉渐变或 emoji 图标 | 违反绝对规则 | 永不 |

## 4. 技术架构（锁定 — 含版本锚定）

| 层 | 技术 | 实际版本 | 锁定原因 |
|----|------|----------|----------|
| 核心引擎 | marguerite-core (Rust) | 现有 | 产物台账/网关/路由基座 |
| 桌面壳 | marguerite-gui (Tauri v2) | 2 | 现有 WebView + plugin-opener/shell |
| 前端 | webui (Vue3 + TS) | 现有 | 现有产物/会话 UI |
| 图标库 | lucide（SVG，手移植 + lucide-static 同步源） | 锁定 | P0 禁 emoji，统一描边矢量 |
| 桌面 API | windows (Rust COM/UIA/进程) | 0.62.1 | 工作流3 硬依赖 |
| UIA | uiautomation | 0.25.0（feature: process,control,pattern,input,event） | 通用 GUI 操控 |
| 输入模拟 | enigo | 0.5.0 | 兜底坐标/热键 |
| 打开程序 | tauri-plugin-opener | 2（现有） | open_path/open_url/reveal |
| 启动进程 | tauri-plugin-shell | 2（现有） | Command spawn |
| OCR（可选） | leptess（tesseract） | 待评估，非 MVP 必需 | 离线 OCR 选项 |

> 版本风险：uiautomation 0.25 传递依赖 windows ^0.37，我方直接依赖 windows 0.62.1。构建时以 0.62 为准，需验证 uiautomation 在其上编译通过，必要时 pin 修订版或打 patch。

## 5. API 端点清单（锁定——开发时以此为唯一依据）

### 产物（Epic A）
| Method | Path | 功能 | 认证 | 请求体 | 响应 |
|--------|------|------|------|--------|------|
| GET | /api/v1/artifacts?session_key=&limit=&cursor= | 列出产物（补 cursor 分页） | session | — | {items:[ArtifactEntry], next_cursor} |
| GET | /api/v1/artifacts/:id | 取单个产物完整 entry + mime | session | — | ArtifactEntry+content_url |
| POST | /api/v1/artifacts/register | 显式补登（修复 R1） | session | {session_key,path,kind} | {ok,id} |
| POST | /api/v1/artifacts/:id/open | 系统默认程序打开（修复 A4） | session | — | {ok, launched} |
| POST | /api/v1/artifacts/:id/content | 内联编辑落盘（修复 A3） | session | {content} | {ok} |
| EVENT | artifact.updated | 产物新增/变更推送（修复 R2） | session | payload{id,session_key,kind,path,ts} | — |

### 路由（Epic B）
| Method | Path | 功能 | 认证 | 请求体 | 响应 |
|--------|------|------|------|--------|------|
| GET | /api/v1/router/status?session_key= | 当前生效 ModelRoutingConfig + 最近 N 决策 | session | — | RouterState |
| GET | /api/v1/router/metrics | 聚合指标 | session | — | Metrics |
| EVENT | router.decision | 扩展 RouterDecision（token_estimate/latency_ms/sub_decisions） | session | RouterDecision+ | — |

### 桌面自动化（Epic C）
| Method | Path | 功能 | 认证 | 请求体 | 响应 |
|--------|------|------|------|--------|------|
| GET | /api/v1/desktop/apps | 已注册 + 运行中应用 | session | — | [AppInfo] |
| POST | /api/v1/desktop/launch | 启动应用 | session | {app_kind,args} | {handle} |
| POST | /api/v1/desktop/action | 执行动作（每步落 Recorder） | session | {handle,action:UiAction} | ActionReceipt |
| GET | /api/v1/desktop/snapshots?session_key= | 审计快照列表 | session | — | [Snapshot] |
| POST | /api/v1/desktop/stop | 急停 | session | — | {ok} |
| EVENT | desktop.action_done | 动作完成 | session | {handle,action,receipt,ts} | — |

错误码：ARTIFACT_NOT_FOUND(1404)、ARTIFACT_OPEN_FAILED(1502)、SESSION_UNKNOWN(1401)、DESKTOP_APP_UNKNOWN(2401)、DESKTOP_ACTION_DENIED(2403)、DESKTOP_COM_ERROR(2505)。

## 6. 数据/状态结构（锁定）

### ArtifactEntry（现有扩展）
```
ArtifactEntry { id, name, path, kind, size, ts, mime, session_key }
ArtifactLedger: Mutex<HashMap<session_key, Vec<ArtifactEntry>>>  // 进程级
```
- 修复 R1：所有写工具（write_file/edit_file/append_file/copy_file/create_directory/rename_file）统一调 record_artifact。
- 修复 R4：record_artifact 的 try_with(SESSION) 失败时，从调用上下文参数透传 session_key 作 fallback。

### RouterState（新增）
```
router_state { mode, fusion_model, candidate_models[], active_strategy, sub_decisions[], token_estimate, parallel, latency_ms }
```

### DesktopAppDriver（新增 trait）
```
trait DesktopAppDriver {
  fn kind()->AppKind;                       // WpsPpt/Browser/Jianying/ThreeD/Generic
  async fn launch(args)->Result<AppHandle>;
  async fn activate(h)->Result<()>;
  async fn run_action(h, action:UiAction)->Result<ActionReceipt>;  // 每步落 Recorder
  async fn quit(h)->Result<()>;
}
// AppRegistry::register(WpsPpt, Box::new(ComDriver::new("Kwpp.Application")))
```

### 审计快照
复用 `core/src/automation/audit.rs` + `ledger.rs`，落 `.marguerite/audit-snapshots/<session>/<ts>.json`（动作 + 前后屏快照路径 + 进程上下文）。

## 7. 页面清单（锁定）

| 页面/面板 | 路由/位置 | 核心组件 | 对应 API | 设计 Token 主题 |
|-----------|-----------|----------|----------|-----------------|
| 产物工作台（侧边栏 Tab） | RightSidebar「产物」 | 列表/网格切换、类型 chips、空态引导、重试条 | artifacts.* | 蓝橙青，lucide 图标 |
| 产物预览器 | 分屏弹层 | 左预览/右大纲，预览/编辑双 Tab | artifacts.get/:id/content/open | §8 token |
| 会话消息区 | ChatView | 气泡分层、thinking、工具卡、产物引用 | router.status, artifact.updated | 蓝橙青 |
| Bloom 路由条 | 消息头/侧栏 | 候选链/并行串行/token/融合归属 | router.decision/status | 蓝主/青融合/橙聚合 |
| 桌面自动化面板 | 会话内 live strip + 审计时间线 | app 图标、动作动词、回放控制、ApprovalCard | desktop.* | 状态色阶 |

## 8. 设计 Token（锁定）
- **主色**：--accent #60A5FA（主模型/胜者）；--accent-secondary #F97316（聚合器/确认动作）；--bloom-accent #21c8c8（Bloom 融合）
- **中性**：--bg #18181A / --bg-surface #202022 / --bg-elevated #2D2D30 / --text #F5F5F7 / --text-muted #B0B0B6 / --text-dim #87878E / --border #303034
- **状态**：--ok #39D7A2 / --warn #E8B23A / --danger #FF6B6B / --info #56C2E6
- **产物类型色（仅小点/角标）**：doc #8AB4F8 / sheet #39D7A2 / deck #F2A65A / image #56C2E6 / code #93C5FD / audio #B0B0B6 / video #7AA2F7 / pdf #FF6B6B / unknown #87878E
- **字体**：--font-sans "Inter",-apple-system,"PingFang SC","Noto Sans SC",sans-serif；--font-mono "IBM Plex Mono"
- **间距**：--sp-1..8 = 4/8/12/18/24/30/40px；**圆角**：control10/card14/panel18/modal24/pill999；**阴影**：--elev-1..3；**焦点环**：3px accent 40%
- **动效**：≤300ms，reduced-motion 降级；**图标库**：lucide（描边 2px、16/20/24px）
- **对标**：Notion 密度分屏骨架 + Linear 暗色克制皮肤 + Marguerite 自家蓝橙青
- ⛔ 禁紫→粉渐变；每屏强调 ≤2；零 emoji；颜色全走 token

## 9. 验收标准（锁定——EARS 格式，QA 以此为准）

| 编号 | 功能 | EARS 验收标准 | 优先级 |
|------|------|--------------|--------|
| AC-A1 | 产物登记 | When 任意写工具落盘文件，系统必须将其登记到所属 session 的台账并向 UI 推送 artifact.updated | P0 |
| AC-A1b | 空状态修复 | If 会话已生成「本周计划.md」，系统必须在侧边栏「产物」Tab 显示该条目（不再空状态） | P0 |
| AC-A3 | 内联编辑 | While 用户在内联编辑器修改 .md/.txt/.csv，系统必须将 content 落盘且不丢失格式 | P0 |
| AC-A4 | 系统打开 | When 用户点击 .pptx/.docx/.xlsx 的「用系统程序打开」，系统必须调起 WPS/Office 并在变更后回写登记 | P0 |
| AC-B1 | Bloom 可视化 | When 一次路由决策完成，系统必须在会话 UI 展示 mode/候选链/并行串行/token/融合归属 | P0 |
| AC-B2 | 消息 redesign | While 渲染会话，系统必须按用户/助手/系统分层并高亮产物引用 | P0 |
| AC-C2 | WPS PPT | When 用户说「做成一周计划 PPT」，系统必须调起 WPS 演示并生成可编辑 .pptx 且登记为产物 | P0 |
| AC-C3 | 回放审计 | While 桌面自动化执行，系统必须为每步写审计快照，且用户可逐帧回放并导出日志 | P0 |
| AC-C4 | 安全闸门 | If 敏感动作（删除/发送/付费）将执行，系统必须弹二次确认；当检测到屏幕注入必须中断告警 | P0 |

## 10. 边界与约束
- 不支持 IE；响应式断点 desktop-first（Tauri 窗口）。
- 性能：产物列表 <500ms、预览首屏 <1s、自动化单步 <800ms。
- 安全：完全开放模式仍保留 kill-switch + 二次确认 + 注入扫描 + 审计 ledger。
- 兼容：Win 主战场（WPS2019+/Office2016+）；Mac/Linux 降级为 enigo 输入模拟。
- 不承诺 100% computer-use 成功率（行业 OSWorld 上限 ~72%），需降级 + 人工接管。

## 11. 内嵌已知坑（从项目记忆拉取）

| 坑 | 技术栈指纹 | 根因 | 修法 |
|----|------------|------|------|
| 产物不在侧边栏 | marguerite-core + webui | 写工具未全量调 record_artifact；events 无 Artifact 事件；前端未订阅刷新 | R1–R5 整体修复（见 §2/§5） |
| COM 类未注册 0x80040154 | windows + WPS COM | WPS 未 regserver | 启动前查注册表 + 提示 /regserver |
| COM 权限 0x80070005 | windows DCOM | DCOM 配置 | 引导 DCOM 配置 |
| uiautomation 与 windows 版本错配 | uiautomation 0.25 + windows 0.62 | 传递依赖 ^0.37 | 以我方 0.62 为准验证编译，必要时 pin/patch |

## 12. 端到端验证步骤

```bash
# 1. 构建核心
cargo check -p marguerite-core
# 2. 启动网关（Tauri）
cargo tauri dev   # 或已装 exe
# 3. 核心成功流（产物）
curl -X POST http://localhost:18790/api/v1/artifacts/register -d '{"session_key":"s1","path":"/.../本周计划.md","kind":"markdown"}'
# 断言：返回 {ok:true,id:...}；侧边栏「产物」出现该条目
# 4. 核心成功流（WPS PPT）
# 在会话说「把本周计划做成 PPT」→ 断言：WPS 启动并生成 .pptx，侧边栏出现且可打开
# 5. 关键错误流（COM 未注册）
# 未 regserver 时执行 WPS 动作 → 断言：返回 DESKTOP_COM_ERROR(2505) 且提示 /regserver
```

## 13. 变更记录
| 日期 | 变更内容 | 原因 | 影响范围 |
|------|----------|------|----------|
| 2026-08-18 | 初版 Spec | 用户确认三文档 + 全部 P0 | 全量 |
| 2026-08-18 | B1 维持 P0 | 用户显式诉求优先于 RICE | Epic B |
