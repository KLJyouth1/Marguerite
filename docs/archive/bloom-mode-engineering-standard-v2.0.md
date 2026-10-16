# Bloom Mode 工程标准 v2.0（UnivAgent 统一质量门禁）

> 文档类型：技术规范 / 活文档（Quality Gate）
> 受众：UnivAgent 全体研发（Rust TUI / TS GUI / Py API / 文档 / QA / DevOps）
> 生效日期：2026-08-05
> 作者：大湾区靓仔（项目总监）| 基于 `后端开发规范_v1.0.md` + `前端开发规范_v1.0.md` 升级、竞品调研（Cline/OpenHands/Hermes/OpenClaw/Aural-Pro/shader-playground）与 Phase J 视觉门禁固化
> 状态：v2.0 草案（待确认后取代两份 v1.0 作为唯一质量门禁来源；两份 v1.0 降级为语言专属细则参考）

---

## 0 Bloom Mode 定义

**Bloom Mode 是 UnivAgent 的工程纪律 + 视觉身份的统一体。** 它要求：任何产物（一行代码、一个组件、一份文档、一段特效）在交付前必须穿过五维决策框架与五级质量门禁；同时产品的视觉表达必须遵循统一的 Bloom 视觉语言（粒子化、品牌蓝、零模板味）。

Bloom Mode 不是一套新流程，而是把既有的两套 v1.0 规范**收口为一份跨语言门禁**，并补上三块 v1.0 缺失的能力：

1. **视觉 P0 门禁**（来自 Phase J 实机复核固化）：禁止 emoji 作功能图标、禁止紫粉渐变、品牌蓝锚定、Design Token 驱动、五源对齐像素复核。
2. **竞品 adopt/avoid**（来自 K4-deep + 全球调研）：把 Cline 的 Plan/Act + 审批门、OpenHands 的沙箱事件流、Aural-Pro 的 GPU 卸载等可复用范式写进门禁。
3. **内置工具路线图**（terminal/browser/sandbox/RPA）：把"更高质产物"所需的自验证闭环（agent 自己跑测试/浏览器验证）写入标准。

> **单一目的**：让 UnivAgent 每产出一个交付物，都比上一次质量更高、可回归、可审计。

---

## 1 五维决策拍板框架（跨语言统一）

所有技术/设计决策必须在五维加权评分，任一维 < 3/5 不得推进。权重按形态微调：

| 维度 | 默认权重 | 评估标准 | 及格 |
|------|---------|---------|------|
| 实际情况匹配度 | 30% | 贴合业务量级/团队技能/部署环境/终端形态；不盲目追新 | ≥3/5 |
| 功能/模块完整性 | 25% | 覆盖需求+边界+异常+交互态（loading/空/错/禁用/hover/focus/active） | ≥3/5 |
| 可维护性 | 20% | 新人定位成本、变更影响面、零知识债 | ≥3/5 |
| 可扩展性 | 15% | 新增同类=加组件/加配置，非改 N 个 if 分支 | ≥3/5 |
| 先进性 | 10% | 生态向上、3 年内不被淘汰 | ≥3/5 |

> 性能敏感：实际情况↑40%、先进性↓5%。探索型：先进性↑20%。C 端大众：实际情况↑45%。

---

## 2 五级质量门禁（跨语言统一，上一级不过不进下一级）

```
L5 发布门禁    回滚就绪 + 灰度无退化 + 应急预案 + 无障碍 0 P0/P1
L4 设计/架构评审 还原度/像素偏差≤2px + 五维评分 + ADR（L2/L3 必过）
L3 自动化测试   单测100% + 接口≥99% + E2E 关键路径 + 性能回归≤基线10%
L2 CI 静态检查  TS strict / Sonar / ESLint+Stylelint / Lighthouse / Gitleaks / 依赖审计
L1 本地门禁     pre-commit 格式化 + 类型检查 + 变更文件单测
```

豁免任何一级必须经 §14 拍板 + 记 ADR。

---

## 3 代码质量红线（统一索引，P0 一票否决）

| # | 红线 | 等级 | 来源 |
|---|------|------|------|
| R1 | if-else/switch 堆叠 > 3 层（改策略/状态/表驱动） | P0 | 后端§1.3-1 / 前端§4.3 |
| R2 | 3 行以上相同代码重复 ≥ 2 次 | P0 | 后端§1.3-2 |
| R3 | catch 后吞异常（空 catch / 仅打印继续） | P0 | 后端§1.3-3 |
| R4 | `SELECT *` / 持久层绕过 Service | P1 | 后端§1.3-4 / §3.1 |
| R5 | 硬编码魔法值（数字/字符串无定义） | P1 | 后端§1.3-5 |
| R6 | `<div onClick>` 替 `<button>`/`<Link>`（无障碍） | P0 | 前端§1.3-1 |
| R7 | render 中同步读 DOM 尺寸（强制同步布局） | P0 | 前端§1.3-2 |
| R8 | `transition: all` | P1 | 前端§1.3-3 |
| R9 | 禁缩放 `user-scalable=no` | P0 | 前端§1.3-4 |
| R10 | 拦截 `onPaste`（尤其密码/验证码） | P0 | 前端§1.3-5 |
| R11 | TS strict 下用 `any` | P1 | 前端§1.3-6 |
| R12 | 硬编码 16 进制色/裸 px 间距字号 | P1 | 前端§1.3-7 |
| R13 | `<img>` 缺 width/height 或 aspect-ratio（CLS） | P1 | 前端§1.3-8 |
| R14 | `dangerouslySetInnerHTML` 未过 DOMPurify | P0 | 前端§6.2 |
| R15 | GET 执行副作用 / 写请求无 CSRF | P0 | 前端§6.2 |
| R16 | 生产残留 `debugger`/`console.log(敏感)` | P1 | 前端§6.2 |
| R17 | 图标用 emoji 替代 SVG 功能图标 | P0 | **Bloom 视觉门禁 V1** |
| R18 | 紫→粉渐变主视觉（Indigo→Pink 三位一体 AI 模板味） | P0 | **Bloom 视觉门禁 V2** |
| R19 | 硬编码颜色值（唯一例外 `#fff`/`#000`） | P0 | **Bloom 视觉门禁 V3** |
| R20 | 弹跳缓动 `cubic-bezier(0.68,-0.55,...)` 等 AI 模板味 | P0 | **Bloom 视觉门禁 V4** |

---

## 4 架构分层（三语言统一约束）

依赖方向**只能向下**，禁止反向引用。

```
[Rust TUI 引擎]  core/src/tui  →  WorkbenchState + EventBus + SlotId 契约
        ↑ bridge (文档 + 共享类型，禁止逻辑耦合)
[TS 桌面 GUI]    nexustar-ui: pages→layouts→features(按域)→shared(ui/hooks/utils/store/theme)→lib(第三方适配)
        ↑ Tauri IPC
[Rust 桌面壳]    desktop/src-tauri  →  sidecar / 文件网关 / UIA 桥
        ↑ HTTP/gRPC/OpenAPI
[Py API 后端]    nexustar-api: controller→service→mapper/repo→domain→infra
[官网/面板]       nexustar-website(-next) / admin-backend
```

- `shared/components/ui` 不得反向依赖 `features/`；`utils/` 纯函数不得引 React/DOM/API。
- TUI 与 GUI 通过 **WorkbenchState + EventBus + 36 槽 SlotId** 跨形态唯一来源对接；GUI 直接消费，无需返工（Phase J 已锁定）。

---

## 5 类型安全与状态管理

- **API 契约**：后端 OpenAPI 3.0 → `openapi-typescript`/`Orval` 生成前端类型（漂移零可能）。响应统一信封 `{code,message,data,traceId,timestamp}`，前端拦截器解包、非零抛 `BusinessError`。
- **状态分层**：组件内 `useState` → 跨层 `props/Context` → 单 feature `Zustand slice` → 跨 feature 全局 `Zustand root` → 服务端缓存 **必须走 React Query/SWR**（禁止手写 useState+useEffect 取数）。
- **表单**：Zod + React Hook Form（schema first，类型/校验/默认值三处复用）。
- **高频信号（音频/鼠标/帧）禁止走 React state**：用 `ref` 直送 shader uniform / 原生事件，避免 re-render 风暴 `[Research-backed]`（shader-playground / 7g3n starter 实证）。

---

## 6 异常 / 安全 / 可访问性

- 异常分类：参数(400)/业务(200+码)/第三方(502)/系统(500)/兜底(500)；Service 不得 catch 后返 null 伪装成功。
- XSS：`dangerouslySetInnerHTML`/`v-html` 必经 DOMPurify。CSRF：写请求带 Token。JWT 优先 HttpOnly Cookie。CSP 生产启用。
- a11y P0：语义标签、`aria-label`、键盘可达、`:focus-visible` 焦点环、表单 `type/inputmode/autocomplete`、`aria-live` 通知、**绝不拦截 paste**。axe-core 0 严重+0 中度。

---

## 7 性能与降级

- 前端 CWV 预算：LCP<2.5s / INP<200ms / CLS<0.1 / TTFB<800ms；CI 卡点 LCP<3s、INP<300ms、首屏 JS<200KB gzip。
- 分级降级 L1~L4：弱网切缩略图/关动画/`prefers-reduced-motion`；API 降级去抖/降频；离线 Service Worker + IndexedDB 离线队列（带 `Idempotency-Key`）。
- 粒子/WebGL：80K–200K 粒子需 GPU 卸载（逻辑在 shader，CPU 只传 uniform）`[Research-backed]`（Aural-Pro）。低端设备降级 Canvas2D 或关 Bloom。

---

## 8 可扩展性（开闭原则落地）

- 后端：策略模式+Spring 注入 / 事件驱动 / SPI / 表驱动，按优先级。
- 前端：组件注册表 `Record<Type, ComponentType>` / 插槽 Children / HOC 装饰 / Module Federation（仅 50+ 人）。
- 主题：Design Token 驱动（CSS 变量 / `@theme`），新主题=新增一份变量覆盖，不改组件。
- i18n 预埋：`t('ns:key')`，日期/数字走 `Intl.*`，专有名词 `translate="no"`。

---

## 9 Bloom 视觉门禁（产品级 P0，Phase J 固化）

UnivAgent 所有可见界面（TUI/ GUI / 官网 / 面板 / 特效）必须遵守：

1. **V1 图标**：功能图标一律 SVG 统一描边、可缩放、语义明确；全项目锁定一套图标库，禁止 emoji 作功能图标（UGC/IM 例外）。
2. **V2 配色**：禁止 Indigo→Pink 渐变 + 发光边框 + 毛玻璃三位一体 AI 模板套路；品牌蓝锚定 `#3B82F6`（纯色 Indigo `#6366F1`/Slate `#4F46E5` 允许）。
3. **V3 Token**：颜色/间距/字号一律 Design Token 引用，禁止硬编码（例外 `#fff`/`#000`）。
4. **V4 文案/动效**：禁止 Lorem ipsum / Welcome to / Sign up today 等占位；禁止弹跳缓动；Hero 展示真实内容非千篇一律。
5. **V5 像素复核**：GUI 尺寸锁 48/236/288/24（px）↔ TUI 列宽 2/22/34/1（半角）；五源对齐（设计变量+元数据+截图+代码+渲染截图）交叉校验。

> 门禁工具：emoji 正则扫描 + 紫粉渐变色值断言 + 品牌蓝锚点断言，CI 卡点（Phase J 已实现 J5 测试套件，可复用）。

---

## 10 竞品 Adopt / Avoid（来自 K4-deep + 全球调研）

| 来源 | Adopt（写入 UnivAgent） | Avoid |
|------|------------------------|-------|
| **Cline** (61K⭐) | Plan/Act 分离 + 逐步审批门 + `.rules` 文件承载编码标准 + MCP 市场扩展 + 内置浏览器/终端沙箱 | 纯审批流在超大多文件任务偏慢（用并行子 agent 缓解） |
| **OpenHands** (72K⭐) | Docker 沙箱运行时 + 事件流架构 + 插件系统 + 多 agent 协作 | 循环/回退敏感（需 Plan 门禁兜底） |
| **Hermes Agent** | 五层记忆 + 自进化闭环（技能自动生成） | 单循环无沙箱（UnivAgent 用本地优先+沙箱补） |
| **OpenClaw** (346K⭐) | Gateway+多 Runtime 插件化 + ClawHub 技能生态思路 | 仓库过重（UnivAgent 走轻量 bridge） |
| **Aural-Pro** | GPU 卸载粒子（80K–200K）+ UnrealBloom + Nuclear Boost 低频算法 | 仅 Wallpaper 形态（需抽成 React 组件） |
| **shader-playground** | R3F + Vite + Tailwind + Zustand + GLSL 技术栈（与 nexustar-ui 同构）+ 音频 bypass React state | — |
| **Cymatica** | 双核 FFT：512 瞬时（鼓点）+ 32768 谐波（纹理）并行 | — |
| **DeepSeek-TUI** | TUI 流式渲染 + 终端 UI 管理范式（直接竞品参照） | 无 GUI/无技能生态 |

---

## 11 内置工具与应用路线图（闭环自验证）

为"更高质产物"，UnivAgent 内置工具必须支持 agent 自己验证产出：

| 工具 | 形态 | 参考 | 优先级 | 状态 |
|------|------|------|--------|------|
| 沙箱终端 | 内嵌 xterm.js / 自研，跑测试/构建/命令并读 stderr | Cline/Cursor | P0 | 规划 |
| 内置浏览器 | Headless(Puppeteer)+有头双模，截图验证 UI 变更 | Cline | P0 | 规划 |
| 自动化引擎 | 无视觉鼠标操控（坐标/语义定位）+ 第三方 RPA 软件桥 | OpenHands/UIA | P1 | 规划(C 阶段) |
| 文件网关 | 本地文件读写/检索/收藏（歌曲/文档） | — | P1 | 规划(C 阶段) |
| 粒子音乐 | 音频解析+歌词获取+FFT 驱动 Bloom 背景 | K1/Aural-Pro | P0(差异化) | **K1.2 原型进行中** |

> 所有工具走 MCP 或 Tauri IPC，权限按需申请、操作可审计（沿用 Cline 审批门）。

---

## 12 跨仓集成规范（三仓不合并，建统一门禁索引）

| 仓库 | 角色 | 门禁接入点 |
|------|------|-----------|
| **Marguerite** | TUI 引擎（Rust） | `core/src/tui/workbench` + J5 测试套件（已落地 P0 视觉门禁） |
| **GUI_Agent** | 主仓库（TS/Py/Rust，有 git） | nexustar-ui(前端门禁) / nexustar-api(后端门禁) / desktop(壳) |
| **_GUI_Agent_archived** | 只读归档（研究参考） | 仅 external/ 竞品源码供 K4 分析，不并入构建 |

- 跨仓对接靠 `docs/integration/` 下的 bridge 文档 + 共享类型，不靠代码耦合。
- 任一仓的 PR 必须过 §2 五级门禁；violations 触发打回。

---

## 13 反模式速查（统一，节选自两份 v1.0 + Bloom 新增）

见 §3 红线表（R1–R20）。新增 Bloom 专属：emoji 图标(R17)、紫粉渐变(R18)、硬编码色(R19)、弹跳缓动(R20)。

---

## 14 ADR 与多轮复核

- **L2/L3 决策必记 ADR**：`docs/adr/NNNN-{简述}.md`（背景/备选/决策/后果），Accept 后不可改，推翻建 Superseded。
- **四轮复核**：①作者自审（对照 §3 红线+门禁清单）②Code Reviewer（代码层）③架构组（五维+一致性）④设计还原/发布终审（体验+运维）。
- 评审意见必须可落地（"此处 div onClick 违反 R6，请改 button+aria-label='删除'"），禁止"这里不太好"。

---

## 15 版本迭代

| 版本 | 日期 | 变更 | 人 |
|------|------|------|----|
| v2.0 | 2026-08-05 | 统一后端/前端 v1.0 为跨语言门禁；新增 Bloom 视觉门禁(V1–V5)、竞品 Adopt/Avoid、内置工具路线图、跨仓集成规范 | 项目总监 |

> 本标准为活文档，季度回顾。所有 UnivAgent 交付物以本标准为唯一质量门禁来源；两份 v1.0 保留为 Java/TS 专属细则参考。
