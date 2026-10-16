# Marguerite 统一工作台合并方案 v1.0

> 日期: 2026-09-02
> 目标: 将 D:\Downloads\Marguerite 与 D:\Downloads\Marguerite\marguerite 合并为单一项目根，CLI/TUI/GUI/Web 四端统一为单一工作台
> 设计原则: 诚实工程、零能力删减、状态单一来源、契约N同步

---

## 1. 现状诊断

### 1.1 目录结构问题
```
D:\Downloads\Marguerite\              ← 外层包装目录（冗余）
├── marguerite\                       ← 实际项目根（唯一真实内容）
│   ├── core/      (Rust引擎 2412 tests)
│   ├── gui/       (Tauri+Vue3 桌面端)
│   ├── tui/       (ratatui 终端)
│   ├── webui/     (Vue3 控制台 3148 tests)
│   ├── pyengine/  (Python 侧车)
│   ├── workbench/ (文件智能+LLM实验台)
│   ├── skills/    (内置技能 YAML)
│   └── ...
├── _archive\      (旧项目归档 ≈2.8GB)
└── deliverables\  (交付物)
```

**问题**: 外层 `Marguerite` 目录是纯包装层，增加路径深度，构建脚本需 `%~dp0..` 多跳一层；`_archive/` 含旧 Electron 桌面（≈2.8GB）污染仓库。

### 1.2 四端现状（已天然统一于核心引擎）

| 端 | 入口 | 技术栈 | 与核心关系 | 状态 |
|---|---|---|---|---|
| CLI | `cargo run -p marguerite-core` | Rust lib.rs + argv.rs | 直接调用 | 健康 |
| TUI | `cargo run -p marguerite-tui` | ratatui | 直接调用 core API | 健康 |
| GUI(桌面) | `cargo run -p marguerite-gui` | Tauri + Vue3 WebUI | WebSocket → 统一网关 | 健康 |
| Web | `http://127.0.0.1:18790` | Vue3 (同 GUI) | WebSocket/HTTP → 统一网关 | 健康 |

**关键发现**: GUI 与 Web 已共享同一套 Vue3 WebUI 代码（`webui/dist` 嵌入 Tauri），并非两个独立前端。四端的真正差异只是**容器层**（终端 ratatui vs Tauri WebView vs 浏览器），业务逻辑全在 Rust 核心。

---

## 2. 合并目标架构

### 2.1 合并后目录结构
```
D:\Marguerite\                        ← 合并后单一项目根（建议路径）
├── Cargo.toml                        ← workspace = ["core","gui/src-tauri","tui"]
├── config.yaml                       ← 默认配置模板
├── README.md                         ← 统一工作台入口文档
├── CHANGELOG.md
├── AGENTS.md                         ← AI 智能体指南（已存在，保留）
│
├── core/                             ← Rust 核心引擎（不变）
│   ├── src/cli/                      ← CLI 路由 + 19 子命令
│   ├── src/agent/                    ← Agent 运行时 + 工具循环
│   ├── src/model/                    ← LLM 封装 + 双模型路由
│   ├── src/mcp/                      ← MCP 客户端/服务端/注册表
│   ├── src/memory/                   ← 三层记忆
│   ├── src/guard/                    ← 四层护栏 + 自动修复
│   ├── src/context/                  ← 分页/缓存/预算
│   ├── src/skills/                   ← 技能引擎
│   ├── src/plugins/                  ← 插件系统 + 18 harness 清单
│   ├── src/gateway/                  ← 统一网关（WebSocket + HTTP JSON-RPC）
│   ├── src/orchestration/            ← 编排器（模式切换/蜂群/团队）
│   ├── src/tui/                      ← ratatui 终端界面
│   └── src/mode/                     ← 模式系统（plan/agent/ensemble/bloom/omni）
│
├── gui/                              ← Tauri 桌面壳（不变）
│   └── src-tauri/
│       ├── src/lib.rs                ← Rust 后端：runtime_status + onboarding.status
│       ├── tauri.conf.json           ← 前端 dist = webui/dist
│       └── webui/dist/               ← 构建产物嵌入点
│
├── tui/                              ← ratatui 终端工作台（不变）
│
├── webui/                            ← Vue3 统一控制台（不变）
│   ├── src/
│   │   ├── App.vue                   ← 统一起点：OnboardingOverlay + ModeBadge + GatewayReadiness
│   │   ├── components/               ← 共享组件库
│   │   ├── views/                    ← 12+ 功能视图
│   │   ├── router/                   ← 分组导航（Work/Operate/Observe/Settings）
│   │   ├── stores/                   ← Pinia 状态（useActiveMode / useGatewayReadiness）
│   │   └── i18n/                     ← 6 语言 full parity
│   └── dist/                         ← Vite 构建产物
│
├── pyengine/                         ← Python 侧车（OpenSquilla 微内核，不变）
│
├── workbench/                        ← 文件智能 + LLM 实验台（子项目，不变）
│   ├── backend/                      ← FastAPI (port 8011)
│   └── frontend/                     ← React SPA
│
├── skills/                           ← 内置技能 YAML（不变）
├── third_party/                      ← 第三方集成 + 27 仓库清单（不变）
├── docs/                             ← 文档（新增本合并方案）
├── scripts/                          ← 构建/验证/打包脚本
└── dist/                             ← 构建产物（MSI/NSIS/便携包）
```

### 2.2 统一工作台架构图

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           统一入口层 (4 容器)                                  │
├──────────────┬──────────────┬──────────────────────────────┬─────────────────┤
│   CLI 终端   │   TUI 终端   │      Tauri 桌面窗口          │   浏览器 Web    │
│  marguerite  │  marguerite  │   Marguerite.exe (MSI)       │ 127.0.0.1:18790 │
│   -p/--pipe  │   ratatui    │   内嵌 Vue3 webui/dist       │   (回环 only)   │
└──────┬───────┴──────┬───────┴──────────────┬───────────────┴────────┬────────┘
       │              │                      │                        │
       ▼              ▼                      ▼                        ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         统一网关层 (core/src/gateway/)                        │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐                │
│  │  WebSocket RPC  │  │  HTTP JSON-RPC  │  │  ACP 薄壳        │                │
│  │  (实时双向)      │  │  (请求-响应)     │  │  (127.0.0.1)    │                │
│  └────────┬────────┘  └────────┬────────┘  └────────┬────────┘                │
│           └─────────────────────┴─────────────────────┘                        │
│                              │                                                │
└──────────────────────────────┼────────────────────────────────────────────────┘
                               ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                        Rust 核心引擎 (core/src/)                              │
│  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐   │
│  │  CLI    │ │ 意图    │ │ Agent   │ │ 模型    │ │ MCP     │ │ 记忆    │   │
│  │ 路由    │ │ 分类    │ │ 运行时  │ │ 路由    │ │ 注册表  │ │ 三层    │   │
│  └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘   │
│       └───────────┴───────────┴───────────┴───────────┴───────────┘          │
│                              │                                                │
│  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐              │
│  │ 护栏    │ │ 上下文  │ │ 技能    │ │ 插件    │ │ 编排    │              │
│  │ 四层    │ │ 引擎    │ │ 注册表  │ │ 系统    │ │ 器      │              │
│  └─────────┘ └─────────┘ └─────────┘ └─────────┘ └─────────┘              │
│                              │                                                │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ 模式系统 (plan/agent/ensemble/bloom/omni_transcend)                   │   │
│  │  + 蜂群控制 + 团队持久化 + BloomSpec + 思考链 S1-S5                  │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
└──────────────────────────────┬────────────────────────────────────────────────┘
                               │ core_bridge (物理桥接)
                               ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                      Python 侧车层 (pyengine/ + workbench/)                  │
│  ┌─────────────────────────┐  ┌─────────────────────────────────────────┐   │
│  │ OpenSquilla 微内核       │  │ File Intelligence + LLM Lab + Agent Loop │   │
│  │ (技能执行 / 工具封装)     │  │ (FastAPI 8011 / 注意力热力图 / BPE / TTC) │   │
│  └─────────────────────────┘  └─────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. 合并策略（四阶段）

### Phase 1: 目录归一（零代码改动）
1. **上移项目根**: `D:\Downloads\Marguerite\marguerite\*` → `D:\Downloads\Marguerite\*`（删除包装层）
2. **归档清理**: `_archive/` 中旧 Electron 桌面（≈2.8GB）移至外部备份位置，仓库内只保留清单
3. **路径修正**: 所有构建脚本中的 `%~dp0..` 改为 `%~dp0`（少跳一层）

### Phase 2: 构建系统统一
1. **Cargo workspace**: 确认 `Cargo.toml` 包含 `["core","gui/src-tauri","tui"]`
2. **Tauri 配置**: `tauri.conf.json` 的 `frontendDist` 保持 `"webui/dist"`，`beforeBuildCommand` 保持外部构建
3. **打包脚本**: `scripts/package_app.cmd` 的 `REPO_ROOT` 从 `%~dp0..` 改为 `%~dp0`
4. **WebUI 构建**: `npm run build` 产物同步到 `gui/src-tauri/webui/dist`

### Phase 3: 入口统一（品牌 + 启动）
1. **单一 README**: 顶层 README.md 描述统一工作台四端入口
2. **启动器**: `dist/` 下产物统一命名 — `Marguerite_0.2.0_x64_en-US.msi`（桌面）、便携 zip（CLI+TUI+WebUI）
3. **首次启动**: OnboardingOverlay 已在 WebUI 实现（API Key 配置向导），TUI/CLI 在 `run_turn` 中已有 `api_key_missing` 结构化引导

### Phase 4: 验证门禁
1. `bash scripts/verify.sh --clean --all` — 7/7 全绿
2. core: 2412 passed · webui: 3148 passed · clippy: 零 warning
3. MSI 构建 + OLE 头验证

---

## 4. 智能体调用逻辑统一设计

### 4.1 调用协议分层

| 层级 | 协议 | 端 | 说明 |
|---|---|---|---|
| L0 引擎层 | Rust 函数调用 | CLI/TUI | 直接 `lib.rs` API |
| L1 网关层 | WebSocket RPC | GUI/Web | `session.*` / `onboarding.*` / `mode.*` |
| L2 HTTP层 | JSON-RPC | Workbench/外部 | `POST /api/*` |
| L3 MCP层 | MCP 协议 | 外部工具 | `tools/list` / `tools/call` |

### 4.2 模式状态单一来源

```
useActiveMode (shared store)
    ├─ init() → mode.get (水合)
    ├─ 订阅 session.event.mode.changed (实时广播)
    └─ switchActiveMode() → 乐观更新 + RPC

所有视图订阅同一实例:
    - ModeBadge (顶栏徽标)
    - ModeHintBar (对话页提示)
    - ControlCenterView (智能中枢面板)
    - EngineHealthView (引擎健康)
    - CommandPalette (模式切换命令)
```

### 4.3 就绪态统一

```
useGatewayReadiness
    ├─ WS 连接态: disconnected | connecting | connected
    └─ 业务就绪态: not_ready (缺API Key) | ready
    
conn-pill 显示:
    disconnected → 灰色 "离线"
    connecting   → 蓝色 "连接中..."
    not_ready    → 黄色 "需配置" ← 点击打开 OnboardingOverlay
    ready        → 绿色 "就绪"
```

---

## 5. 功能页面统一设计

### 5.1 导航分组（已落地于 webui/src/router/nav.ts）

| 分组 | 路由 | 覆盖能力 |
|---|---|---|
| **Work** | `/chat` `/editor` `/browser-cockpit` `/terminal` | 核心工作区 |
| **Operate** | `/skills` `/plugins` `/cron` `/dispatch` | 执行与调度 |
| **Observe** | `/engine-health` `/overview` `/logs` | 观测与诊断 |
| **Settings** | `/settings` `/control/control-center` `/local-models` | 系统与配置 |

### 5.2 跨端功能映射

| 功能 | CLI/TUI | GUI/Web |
|---|---|---|
| 聊天对话 | TUI 消息面板 | `/chat` ChatView |
| 文件编辑 | `-f` 参数 + 工作区 | `/editor` WorkspaceEditorView |
| 浏览器 | 外部浏览器 + `--browser` | `/browser-cockpit` 内置 iframe |
| 终端 | 内嵌终端 | `/terminal` xterm.js 沙箱 |
| 模型管理 | `model list/add/remove` | `/local-models` LocalModelsView |
| 模式切换 | `/model` slash 命令 | ControlCenter + ModeBadge |
| 插件市场 | `skills list/install` | `/plugins` PluginsView |
| 专家/蜂群 | `team run` | `/expert-center` ExpertCenterView |
| 设置 | 配置文件 | `/settings` SettingsModal |

---

## 6. 后端服务网关统一设计

### 6.1 双网关共存（不合并，桥接）

```
统一网关 (core/src/gateway/)          Workbench 网关 (workbench/backend/)
    port: 18790                          port: 8011
    proto: WebSocket + HTTP              proto: HTTP (FastAPI)
    auth: session token                  auth: none (回环 only)
    scope: 对话/模式/缓存/会话              scope: 文件/LLM实验/Agent循环
    
两者通过 core_bridge 共享 CoreSqlite:
    core/src/gateway/methods.rs ──► core_bridge ──► workbench/backend/api.py
```

### 6.2 不合并的理由
- Workbench 是**实验台**（LLM 原理可视化 + 文件智能），与核心引擎**互补非重复**
- FastAPI 的同步路由适合 CPU 密集型实验（BPE/注意力矩阵/GPT前向）
- WebSocket 实时推送适合对话场景
- 两者均只监听回环，安全边界一致

---

## 7. UI/UX 设计统一规范

### 7.1 设计令牌（跨端共享）

```css
/* 品牌色 — 蓝主 + 橙强 */
--brand: #1A56DB;           /* light */
--brand-dark: #60A5FA;      /* dark */
--accent-2: #C2410C;        /* light */
--accent-2-dark: #F97316;   /* dark */

/* 从 webui/src/themes/{light,dark}/tokens.css 同步到 workbench/frontend/src/styles/tokens.css */
```

### 7.2 响应式断点
- Desktop: ≥1280px（Tauri 窗口默认尺寸）
- Tablet: 768–1279px
- Mobile: ≤767px（底部标签栏切换）

### 7.3 无障碍（已落地）
- Icon-only 按钮带 `aria-label`
- 状态变化用 `aria-live="polite"`
- 焦点管理（Modal 关闭后返回触发器）
- 颜色对比度通过 10 主题守卫

---

## 8. 合并执行清单

| # | 任务 | 影响面 | 验证方式 |
|---|---|---|---|
| 1 | 目录上移（删除包装层） | 所有脚本路径 | `ls D:\Downloads\Marguerite\Cargo.toml` |
| 2 | `_archive` 移出仓库 | 体积 -2.8GB | `du -sh .git` |
| 3 | 修正脚本路径 | `scripts/*.cmd` | 构建成功 |
| 4 | 更新 README | 入口文档 | 人类可读 |
| 5 | verify.sh --clean --all | 全仓库 | 7/7 绿 |
| 6 | cargo tauri build | MSI 产物 | OLE 头验证 |

---

## 9. 风险与回滚

| 风险 | 缓解 |
|---|---|
| 路径变更破坏 CI | 先本地验证全绿再提交 |
| 脚本路径多跳/少跳 | 每个脚本打印 `REPO_ROOT` 回显确认 |
| 旧归档丢失 | 外部备份后再删除 |
| Tauri 嵌入路径失效 | `frontendDist` 用相对路径 `webui/dist`，不受根目录移动影响 |

**回滚**: `git revert` 单提交；归档从外部备份恢复。

---

*本方案由 agent-engineering-core 技能约束下产出，遵循 R1-R10 铁律。*
