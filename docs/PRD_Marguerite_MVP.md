# Marguerite MVP 产品需求文档（PRD）

> 范围：三需求合并为一个 MVP 的三 Epic 规划
> - Epic A：产物注册修复 + 全格式预览/编辑
> - Epic B：Bloom 路由融合可视化 + 会话界面改进
> - Epic C：桌面自动化（Agent 自主操控本机软件）
>
> ⛔ 绝对规则（全程遵守）：(1) 功能图标一律用文字描述，禁止 emoji 图标；(2) 配色沿用产品蓝橙语义令牌，禁止紫色→粉色渐变；(3) 禁止 "Welcome to" / "Lorem ipsum" 类占位文案。
> 调研依据：联网检索 Claude Artifacts / ChatGPT Canvas / Manus / OpenAI Operator·ChatGPT Agent / Claude Computer Use·Cowork / 影刀RPA / UiPath / WPS 灵犀 Pro(Lingxi Pro) / harness-anything / WPS AI 企业版合规规范 等公开资料（2025-2026）。

---

## 0. 现状诊断（供架构师定位，非需求本身）

| 现象 | 代码位置 | 初步判断 |
|------|----------|----------|
| 已生成 `本周计划.md` 但右侧「产物」空状态 | `core/src/tools/file_change.rs` 的 `ArtifactLedger`（进程内存 HashMap<session, Vec>）；`core/src/gateway/methods.rs` 的 `artifacts.list` 仅读该台账 | 文件落盘路径未触发 `record_artifact(session, path, kind)`，或 `.md` 的 kind 未被识别、或前后端 session key 不一致，导致数据源为空 |
| 仅 html/markdown/text/image/pdf 可预览 | `webui/src/utils/workbench/artifactPreview.ts` 的 `artifactWorkbenchPreviewKind` | docx/xlsx/pptx/csv/json/svg 等返回 `unsupported`，无兜底"系统打开" |
| 侧栏图标与打开行为单一 | `webui/src/components/chat/RightSidebar.vue` 的 `fileIcon`（docx/xlsx/pptx/pdf 全映射 `fileText`）、`openArtifact` 仅走应用内预览 | 多格式无差异化图标，二进制格式无法调起 WPS/Office |
| 设计令牌 | `AGENTS.md` §3 不变量：三端共享"蓝橙语义令牌" | PRD 设计约束据此锁定，禁止紫粉渐变 |
| 自动化/审计底座已存在 | `core/src/automation/audit.rs`+`ledger.rs`、`core/src/tools/computer_use.rs`（ScreenPerception/InputControl/GuiGrounding） | Epic C 可直接复用为审计/回放基座与窗口操控基座 |
| Bloom 路由可视化 | `core/src/tui/bloom/`（widget/theme/spectral/capabilities） | Epic B 需在 Web/Tauri 端补齐同等能力的可视化 |

---

## 1. 竞品分析

### 1.1 直接竞品（AI 助手 + 产物/自动化，与 Marguerite 最可比）

| 产品 | 产物/文件管理范式 | 权限/安全模型 | 办公文档 AI 生成 UX | 公认短板（来自差评） | 对我们的启示 |
|------|------------------|--------------|---------------------|----------------------|--------------|
| **ChatGPT（Canvas + ChatGPT Agent/Operator）** | Canvas 双栏文档/代码编辑器，段落级编辑、滑块调速；Agent 模式含终端/文本浏览器/连接器(Gmail/Drive/GitHub) | 云托管虚拟浏览器，登录/支付前确认；不触本机文件系统 | 能生成文档/代码，但不可直接产出原生 Office 文件 | Canvas 早期无实时代码预览；Agent 仅浏览器、不碰本机；Operator 浏览器自动化在改版网站/CAPTCHA 前频繁卡死 | 我们差异化在"本机优先、可触达任意软件" |
| **Claude（Artifacts + Computer Use / Cowork）** | Artifacts 独立面板，代码/文档/图表实时预览、可分享链接、版本管理；2025-10 起支持内联编辑 | Computer Use 原始截图+键鼠原语，需开发者自建 VM 护栏；2026-03 Cowork 在真实 Mac 运行，按应用授权、敏感应用默认拦截、破坏性行为需确认、屏幕内容做 prompt-injection 扫描 | 不直接生成原生 PPT/Word，偏代码与网页 | 早期 Artifacts "只看不能改"；Computer Use 有被 prompt-injection 诱导下载恶意程序的真实漏洞；远程 VM 模式攻击面大 | 我们应保留"可审计回放 + 敏感操作二次确认 + 注入扫描"三件套 |
| **Manus（通用自主智能体，已被 Meta 收购）** | 云端 VM 执行，产出研究报告/代码/网页；GAIA 自报 86.5%(L1) | 云端执行，数据经第三方服务器；信用点计费 | 生成 PPT 时**无法调用 Office 软件**，任务常失败 | 速度慢、token 消耗惊人、上下文窗口短、付费墙/CAPTCHA 卡死、易陷入死循环、幻觉严重、无本地文件/无回放审计 | 我们"本地执行 + 可审计 + 能调 Office"正好填补其最大短板 |
| **WPS 灵犀 Pro（Lingxi Pro，金山）** | 对话即入口，技能产出**可编辑原生 Word/PPT**；云文档闭环（678M MAU） | 云端 SaaS，依赖平台侧权限与加密；企业版支持私有化部署+RBAC+全生命周期审计 | 直接输出带公式表格、可审阅文档、成品 PPT | 不能在对话中直接引用本地/云文件；按"会话"而非"项目"组织；依赖云端 | 我们用"本地优先 + 项目级产物台账 + 不依赖云"切其软肋 |

### 1.2 替代方案（非同类但覆盖同一用户任务）

| 产品 | 形态 | 价值 | 短板 | 启示 |
|------|------|------|------|------|
| **影刀 RPA / UiPath** | 可视化/编码 RPA，元素捕获驱动 | 电商/财务流程成熟模板，免代码拖拽（影刀） | 元素捕获在 UI 改版后极不稳定；UiPath 重、贵、学习曲线陡；数据存云端；本地 EXE 调用受限；常驻进程吃资源 | 我们走"自然语言驱动 + 通用 computer-use"，不重造低代码设计器 |
| **harness-anything（开源）** | 把 WPS/Office/Photoshop/Zotero 的 COM 接口封装成 AI 可调用的 CLI（47 条 WPS 命令，JSON 驱动 PPT 生成） | 证明"COM/CLI 封装 → Agent 直接驱动桌面软件"路径可行 | 仅 Windows + pywin32；需自装环境；非产品化 | Epic C 可借鉴其"统一 CLI/接口抽象 + JSON 数据驱动"思路，但做成产品内原生能力 |

### 1.3 补充参考（形态参照）
- **Gemini Canvas**（2025-10 上线侧栏，与 Workspace 打通）、**OpenAI Operator**（浏览器专用，87% WebVoyager 但 OSWorld 仅 38.1%）、**UiPath**（企业审计强但个人过重）——共同印证：本地优先、轻量、可审计是当前市场空白。

---

## 2. 用户画像与核心场景

### 2.1 主用户画像
- **身份**：重度效率用户 / 个体创作者 / 小微企业主 / 研究员，日常在**本机**重度使用多款桌面软件。
- **痛点**：重复跨软件搬运数据；想让 AI "直接把活干完"而不是"给段文字让我自己排版"；不信任把机密文件上传云端。
- **技术水平**：会用命令行但不想写脚本；愿意授予"完全开放"权限以换取自动化，但要求**可回放、可审计**。
- **核心诉求**：AI 在本地电脑自主启动并操控 WPS/浏览器/剪映/3D 软件，串联出成品，且每一步可追溯。

### 2.2 五个真实场景
1. **WPS 做一周计划 PPT**：用户说"把刚生成的那份本周计划做成 12 页汇报 PPT"，Agent 调起 WPS 演示，套用 VI 模板，产出可编辑 `.pptx`，并在侧栏登记、可预览/再编辑。
2. **浏览器自动填表**：Agent 打开指定网页，识别表单字段，填入从本地 Excel 读取的数据并提交，全过程录屏+事件流可回放。
3. **剪映自动剪辑**：Agent 启动剪映，导入本地素材，套用模板，批量导出短视频。
4. **3D 软件批量出图**：Agent 操控建模软件，加载参数表，批量渲染并导出 PNG/glTF，汇总到产物侧栏。
5. **跨软件数据汇总周报**：从浏览器抓数据 → 写入本地 Excel → 调 WPS 生成带图表的周报 PPT → 全部产物自动登记、可审计。

---

## 3. 市场空白分析（来自竞品差评的机会点）

| 竞品差评信号 | 暴露的空白 | Marguerite 的切入点 |
|--------------|-----------|---------------------|
| Claude Artifacts 长期"只看不能改"；Canvas 无实时代码预览 | 本地文件级、可内联编辑的产物管理弱 | 全格式产物"轻量内联编辑 + 系统打开"双轨 |
| Manus 无法调 Office 做 PPT、云端、无回放、慢、贵 | 本地原生软件不可达 + 不可审计 | 本机执行 + computer-use + 审计 ledger |
| 影刀/UiPath 元素捕获脆弱、重、数据上云 | 自然语言驱动、轻量、数据不出域的自动化缺位 | 通用桌面自动化框架（自然语言 + GUI grounding） |
| WPS 灵犀不能对话引用本地文件、按会话而非项目 | 本地优先 + 项目级产物台账缺位 | 会话/项目级产物注册与同步 |
| Operator 仅浏览器、不碰本机文件系统 | 跨软件、触达本机全部窗口的 agent 稀缺 | "完全开放"安全边界 + 可回放审计 |
| 所有云端方案的数据合规焦虑 | 数据不出域、可审计的强需求 | 本地 Tauri 桌面 + RBAC/审计（复用 WPS 企业版合规思路） |

**一句话定位**：本地优先、全格式产物可预览可编辑、Agent 可自主操控任意本机软件且全程可回放可审计的桌面 AI 工作台——填补"云端 agent 够聪明但够不着本机、RPA 够本地但太重太脆"之间的空白。

---

## 4. 功能清单 + RICE 评分

RICE = (Reach × Impact × Confidence) / Effort。Reach 1-10（每季度受影响用户比例），Impact {0.25,0.5,1,2,3}，Confidence {50%,80%,100%}，Effort 1-10（人月）。

### Epic A — 产物修复 + 预览/编辑

| ID | 功能 | Reach | Impact | Conf | Effort | RICE | MVP |
|----|------|-------|--------|------|--------|------|-----|
| A1 | 修复产物同步（R1-R5 五段断链整体修复：登记不全/无 live 推送/前端未订阅/TUI SESSION no-op/预览格式不全） | 10 | 3 | 100% | 4 | **7.50** | P0 |
| A4 | 系统程序打开：.docx/.pptx/.xlsx 等调起 WPS/Office（或系统默认程序），打开后回写登记 | 8 | 1.5 | 90% | 2 | **5.40** | P0 |
| A3 | 轻量内联编辑：.md/.txt/.csv 直接改并回写文件，保留版本 | 8 | 2 | 80% | 3 | **4.27** | P0 |
| A2 | 全格式预览器：image/pdf/html/md 已支持；扩展 docx/xlsx/pptx/csv/json/svg 内联或转换预览，未识别格式用"系统打开"兜底（消除空状态） | 10 | 2 | 80% | 5 | **3.20** | P1 |
| A5 | 产物列表管理：按类型/时间筛选、重命名、删除、下载 | 6 | 1 | 90% | 2 | **2.70** | P1 |

### Epic B — Bloom 路由 + 会话 UI

| ID | 功能 | Reach | Impact | Conf | Effort | RICE | MVP |
|----|------|-------|--------|------|--------|------|-----|
| B2 | 气泡/消息 redesign：用户/助手/系统分层，蓝橙令牌，引用高亮，空状态有意义引导文案 | 9 | 2 | 90% | 3 | **5.40** | P0 |
| B4 | 工具调用卡片：工具名 + 参数摘要 + 状态（运行/完成/失败）+ 关联产物引用 | 9 | 2 | 90% | 3 | **5.40** | P0 |
| B5 | 产物引用 inline：消息中引用产物可点击跳转预览/编辑 | 8 | 1.5 | 85% | 2 | **5.10** | P0 |
| B3 | thinking 过程可视化：可折叠推理步骤、路由/模型标注、token 用量 | 8 | 1.5 | 85% | 3 | **3.40** | P0 |
| B1 | Bloom 路由融合可视化：能力光谱 + 当前模型/路由状态展示（对齐 TUI bloom 模块） | 7 | 2 | 80% | 4 | **2.80** | P1 |

### Epic C — 桌面自动化

| ID | 功能 | Reach | Impact | Conf | Effort | RICE | MVP |
|----|------|-------|--------|------|--------|------|-----|
| C2 | WPS PPT 自动化：先用 computer-use / COM-CLI 落地"一周计划 PPT"，JSON 数据驱动，产出可编辑 .pptx | 8 | 3 | 80% | 5 | **3.84** | P0 |
| C3 | 可回放/可审计：事件流 + 时间轴回放 + 导出审计日志（复用 automation/audit.rs+ledger.rs） | 7 | 2.5 | 85% | 4 | **3.72** | P0 |
| C4 | 安全边界"完全开放"模式：用户已选；含兜底 kill-switch、敏感操作二次确认、prompt-injection 扫描（复用 scanner_l2/l3） | 6 | 2 | 85% | 3 | **3.40** | P0 |
| C1 | 通用桌面自动化框架（基座）：启动/操控任意窗口，截图+键鼠+GUI grounding，统一 action 抽象，供 C2/C3/C4/C5 复用 | 9 | 3 | 80% | 8 | **2.70** | P0（前置） |
| C5 | 浏览器/剪映/3D 软件适配器：框架成熟后接入（抽象通用，非专用） | 5 | 2 | 60% | 6 | **1.00** | P1 |

> 依赖说明：C1 通用框架 RICE 2.70，但按架构侧两期建议——**一期先用 WPS PPT(C2) 落地验证 windows 0.62.1 / uiautomation 0.25.0 / enigo 0.5.0 三 crate 接入与审计回放闭环，二期再抽 `DesktopAppDriver` 通用框架(C1) 并接入浏览器/剪映(C5)、落地安全闸门(C4)**；C3 审计回放为两期共用闭环验证。A2 是 A3/A4 体验完整度补充，MVP 以"双轨（内联编辑 + 系统打开）兜底无空状态"为 P0，完整内联渲染列 P1。

---

## 8. 排期对齐（架构侧输入，2026-08-18）

架构文档已落盘 `docs/ARCH_Marguerite_MVP.md`（6 大节 + OpenAPI 草案附录），与本文档配对。以下为回填的工期与跨平台风险。

### 8.1 A1 子任务工时（R1-R5，整体排期，禁止只修显示层）
| 断点 | 修复动作 | 层 | 工时 |
|------|----------|----|------|
| R1 登记不全 | 全写工具(write/edit/append/copy/create_directory/move) 统一接 `record_artifact` | core | 4 人日 |
| R2 无 live 事件 | `events.rs` 新增 `artifact.updated` 事件 + record 点 emit | core | 3 人日 |
| R3 前端未订阅 | `useSessionArtifacts` 订阅事件 + 增量合并去重 | webui | 3 人日 |
| R4 SESSION no-op | `record_artifact` 增显式 `session_key` 参数 fallback + TUI 补传 | core | 2 人日 |
| R5 预览格式不全 | `artifactPreview` 扩展 office/audio/video/binary 分派 + 组件 | webui | 5 人日 |
| 核心合计 | | | 17 人日 |
| + 联调/回归/QA buffer | | 双端 | ~3 人日 |
| **总计** | | | **~20 人日 ≈ 4 人月（对齐 A1 effort 上修）** |

五个断点必须整体排期；只修显示层会复发。

### 8.2 Epic C 三 crate 工期与跨平台风险（架构师实测版本）
**Phase 1（Windows，WPS PPT 首落地）**：windows 0.62.1 接入+COM = 8 人日；uiautomation 0.25.0 UIA = 5 人日；enigo 0.5.0 输入兜底 = 3 人日；审计回放闭环(复用 `automation/audit`) = 4 人日；WPS PPT 端到端联调 = 5 人日；**Phase1 合计 ~25 人日**。

**macOS 等价（产品跨 Win/macOS，关键）**：
- COM/IDispatch（Windows 专有，无 macOS 等价）→ macOS 走 AppleScript/JXA（`osascript` 外壳），WPS Mac 字典弱、优先 Microsoft PowerPoint Mac（字典完善）。
- UIA（uiautomation 仅 Win）→ macOS 走 Accessibility API（AXUIElement）经 `objc2` + core-foundation。
- 输入模拟 enigo 跨平台共用（macOS 需 TCC 辅助功能授权）。
- 结论：`DesktopAppDriver` trait 平台无关，但须 `cfg(windows)`/`cfg(target_os="macos")` 分支——Windows 后端=COM+UIA+enigo；macOS 后端=AppleScript/AX+enigo。**WPS PPT 自动化以 Windows 为主战场**。

**构建对齐风险（R-COMPAT）**：uiautomation 0.25 传递依赖 windows ^0.37，与我方 0.62 可能冲突。验证策略：先直编（Cargo 允许多版本共存，仅增大构建体积）；若 API 冲突，推荐优先「windows 0.62 直调 UIA COM 接口」（去 uiautomation 包装层）或 patch uiautomation。

### 8.3 图标库锁定（P0 规则落地）
架构师在 ARCH 明确：图标库锁定 **lucide**（lucide-static 作 `webui/src/utils/icons.ts` 单一可信源，CI 校验禁混用 Material/Ant Design 等其它库；新增图标从 lucide 官方 path 派生）。本文档第 6 节设计约束已同步为 lucide，与 ARCH 一致。

（保留两期结论：一期 WPS PPT 验证三 crate + 审计回放闭环；二期抽 `DesktopAppDriver` 通用框架 + 浏览器/剪映 + 安全闸门。）

---

## 5. 明确不做（Out-of-Scope）

| 不做项 | 原因 |
|--------|------|
| 云端多端同步 / 多人协作编辑 | 产品本地优先、数据不出域；云同步属独立 Epic |
| 自研大模型 | 用 Bloom 路由融合现有模型即可，不自建训练 |
| 移动端（iOS/Android） | Tauri 桌面优先，范围收敛 |
| 影刀式可视化低代码流程设计器 | 本阶段用"自然语言 + 审计回放"，不重造 RPA 设计器 |
| Office 文档协同编辑/评论 | 只做生成 + 预览 + 轻编辑 |
| 长尾软件专用适配器（非 WPS/Office/浏览器/剪映/3D） | 先抽象通用框架，长尾由后续迭代/社区补齐 |
| 无审计的"黑盒"自动执行 | 任何执行必须可回放（安全合规底线） |
| 引入紫→粉渐变或 emoji 图标 | 违反设计令牌与绝对规则 |
| 承诺 100% computer-use 成功率 | 行业 OSWorld 上限约 72%（人类基线），需明确降级与人工接管策略 |

---

## 6. MVP 范围建议

### P0（必须随 MVP 交付）
- **Epic A**：A1（注册修复，阻塞项）、A3（内联编辑）、A4（系统打开）；预览以"已支持格式 + 系统打开兜底"消除空状态。
- **Epic B**：B2（气泡）、B3（thinking）、B4（工具卡片）、B5（产物引用）——需求 #2 全部明确要求。
- **Epic C**：C1（通用框架基座）、C2（WPS PPT 自动化）、C3（可回放/可审计）、C4（完全开放安全模式）。

### P1（快速跟进）
- **Epic A**：A2（全格式内联渲染）、A5（产物列表管理）。
- **Epic B**：B1（Bloom 路由可视化）。
- **Epic C**：C5（浏览器/剪映/3D 适配器）。

### 非功能需求（PRD 必含）
| 类别 | 要求 | 优先级 |
|------|------|--------|
| 性能 | 产物列表加载 < 500ms（本地台账）；预览首屏 < 1s；自动化单步延迟 < 800ms | P0 |
| 安全 | "完全开放"模式仍保留 kill-switch、敏感操作二次确认、prompt-injection 扫描（复用 scanner_l2/l3）、完整审计 ledger | P0 |
| 兼容 | Windows/macOS（Tauri）；WPS 2019+/Office 2016+；Chrome/Edge/Safari 最新版 | P0 |
| 可访问性 | 键盘可达、对比度达标（蓝橙令牌已满足基础） | P2 |
| 数据埋点 | 见下 | P1 |

### 设计约束（嵌入 P0 规则）
- **图标**：图标库锁定 **lucide**（lucide-static 作 `webui/src/utils/icons.ts` 单一可信源，CI 校验禁混用 Material/Ant Design 等其它库；新增图标从 lucide 官方 path 派生）；PRD 内功能图标一律文字描述（如"文件图标""播放图标""警告图标"），禁止 emoji。
- **配色**：沿用产品蓝橙语义令牌，禁止紫色→粉色渐变。
- **文案**：禁止 "Welcome to" / "Lorem ipsum" 等占位；空状态改为有意义引导，如"本次会话还没有生成文件，发送任务让 Marguerite 为你产出"。

### 数据埋点方案（MVP 必埋）
| 事件 | 说明 |
|------|------|
| `artifact_registered` | 产物登记成功（验证 A1 修复） |
| `artifact_viewed` / `artifact_edited` / `artifact_opened_external` | 预览 / 内联编辑 / 系统打开（验证 A3/A4） |
| `automation_started` / `automation_step` / `automation_completed` | 自动化启停与单步（验证 C1/C2） |
| `automation_replay_viewed` | 审计回放查看（验证 C3） |
| `bloom_route_shown` / `tool_card_expanded` / `artifact_reference_clicked` | 路由可视化 / 工具卡片 / 产物引用（验证 B 系列） |

### 关键验收标准（Given/When/Then 选摘）
- **A1**：Given 会话中 Agent 生成 `本周计划.md`，When 用户打开右侧「产物」标签，Then 列表中出现该文件且可点击预览（不再空状态）。
- **A4**：Given 列表中的 `.pptx` 产物，When 用户点击"系统打开"，Then 本机 WPS/Office 启动并打开该文件，且操作被登记。
- **C2**：Given 用户指令"把本周计划做成 PPT"，When Agent 完成，Then 产出可编辑 `.pptx` 并出现在产物侧栏。
- **C3**：Given 一次自动化执行，When 用户打开回放，Then 可逐帧查看事件流并导出审计日志。
- **C4**：Given "完全开放"模式，When Agent 触发敏感操作（如删除/支付/发送），Then 弹窗二次确认；When 检测到屏幕 prompt-injection，Then 中断并告警。

### 边界条件
- 空状态：无产物时显示引导文案；未识别格式点击后降级为"系统打开"。
- 错误状态：产物登记失败不影响会话；自动化单步失败进入"人工接管"提示。
- 并发：同一会话多产物并发登记需去重（已有同路径替换逻辑，保留）。
- 离线：无 API Key 时降级为离线回显（架构不变量 §3-3），自动化框架仍可在本地执行。
- 权限拒绝：系统打开被 OS 拒绝时提示"未找到默认程序"。

---

## 7. 调研来源（摘要）
- Claude Artifacts vs ChatGPT Canvas 对比（编辑范式、实时预览差异，2025）
- Anthropic Computer Use / OpenAI Operator·ChatGPT Agent / Claude Cowork 权限模型与 OSWorld 基准（2024-2026）
- Manus 用户差评（PPT 无法调 Office、慢、上下文短、无回放；Meta 收购，2025）
- 影刀RPA / UiPath 用户踩坑（元素捕获脆弱、重、上云、许可限制，2026）
- WPS 灵犀 Pro（Lingxi Pro）发布与局限、WPS AI 企业版合规规范（RBAC/审计/数据不出域）
- harness-anything 开源项目（COM→CLI 封装驱动 WPS/Office/Photoshop，JSON 驱动 PPT）
