# Marguerite 统一工作台 — 前后端开发与架构 + UI/UX 设计方案

> 版本: 1.0 | 日期: 2026-09-02
> 状态: 已执行（目录合并 + 架构确认 + 构建验证）
> 约束: 诚实工程 R1-R10 · 架构不变量 INV-1~9 · Bloom 红线 RED-1~2

---

## 一、执行摘要

本文档是 Marguerite 统一工作台的完整设计方案。目录合并已执行：外层 `D:\Downloads\Marguerite\marguerite\` 内 179 个条目提升到外层 `D:\Downloads\Marguerite\`，123 个临时文件清理，`.ruff_cache/` 和空壳 `deliverables/` 删除。`cargo check` 通过（19.99s），路径验证全绿。

**四端统一架构**：CLI（`marguerite-core` bin，19 子命令）+ TUI（ratatui）+ 桌面（Tauri v2 + Vue3 WebUI，MSI 304MB）+ Web（`127.0.0.1:18790` 回环）。四端共享同一 Rust 核心引擎，通过统一网关（WebSocket + HTTP JSON-RPC）触达。Python 侧车（OpenSquilla + Workbench FastAPI 8011）经 `core_bridge` 桥接。

**测试基线**：core 2412 passed + 0 failed + 17 ignored · webui 3148 passed / 305 files · clippy L3 零警告 · i18n 6 语言 full parity。

---

## 二、前端架构

### 2.1 技术栈

| 层 | 技术 | 版本 | 职责 |
|---|---|---|---|
| 框架 | Vue 3 | ^3.5 | 响应式 UI |
| 状态 | Pinia | ^2.1 | 全局 store（useActiveMode / useGatewayReadiness / useRpcStore） |
| 路由 | Vue Router | ^4.4 | SPA 路由 + 分组导航 |
| i18n | vue-i18n | ^9.14 | 6 语言（zh-Hans/en/ja/fr/de/es）full parity |
| 构建 | Vite | ^5.4 | 产物到 `webui/dist/` |
| 测试 | Vitest | ^2.0 | 3148 passed / 305 files |
| 类型 | vue-tsc | --noEmit | 零类型错误 |
| 守卫 | 10 项架构守卫 | colors/motion/radius/theme-contract/... | 零 hex/零渐变/零 emoji |

### 2.2 目录结构

```
webui/
├── src/
│   ├── App.vue              # 统一起点：OnboardingOverlay + ModeBadge + GatewayReadiness
│   ├── components/           # 共享组件库
│   │   ├── OnboardingOverlay.vue       # 首次启动向导（3步：欢迎→选择→Key）
│   │   ├── ModeBadge.vue               # 模式徽标（顶栏）
│   │   ├── ModeHintBar.vue             # 模式提示（对话页）
│   │   ├── conn-pill.vue               # 连接状态指示器
│   │   └── settings/                   # 设置面板组件
│   ├── views/               # 12+ 功能视图
│   │   ├── ChatView.vue               # 聊天主界面
│   │   ├── WorkspaceEditorView.vue    # 工作区编辑器
│   │   ├── BrowserCockpitView.vue     # 浏览器驾驶舱
│   │   ├── TerminalView.vue           # 终端沙箱（xterm.js）
│   │   ├── SkillsView.vue             # 技能管理
│   │   ├── PluginsView.vue            # 插件市场
│   │   ├── CronView.vue               # 定时任务
│   │   ├── DispatchView.vue           # 任务调度
│   │   ├── EngineHealthView.vue       # 引擎健康
│   │   ├── OverviewView.vue           # 总览仪表盘
│   │   ├── ControlCenterView.vue      # 智能中枢（模式/缓存/能力）
│   │   ├── LocalModelsView.vue        # 本地模型工作台
│   │   ├── ExpertCenterView.vue       # 专家中心
│   │   └── SettingsModal.vue          # 设置弹窗
│   ├── router/
│   │   └── nav.ts            # 分组导航（Work/Operate/Observe/Settings）
│   ├── stores/               # Pinia 状态
│   │   ├── useActiveMode.ts  # 模式状态单一来源
│   │   └── useRpcStore.ts    # RPC 通信层
│   ├── composables/          # 组合式函数
│   │   ├── useGatewayReadiness.ts  # 就绪态统一（WS + 业务）
│   │   └── useRpcCall.ts    # RPC 调用封装
│   ├── lib/
│   │   └── rpc.ts            # WebSocket 协议实现
│   └── i18n/                 # 6 语言
│       ├── zh-Hans.json
│       ├── en.json
│       ├── ja.json
│       ├── fr.json
│       ├── de.json
│       └── es.json
├── dist/                     # Vite 构建产物 → gui/src-tauri/webui/dist
├── package.json
├── vite.config.ts
└── tsconfig.json
```

### 2.3 状态管理

**模式状态单一来源**（`useActiveMode`）：
```
init() → mode.get (水合)
  ↓
订阅 session.event.mode.changed (实时广播)
  ↓
switchActiveMode() → 乐观更新 + RPC
  ↓
所有视图订阅同一实例:
  ModeBadge · ModeHintBar · ControlCenterView · EngineHealthView · CommandPalette
```

**就绪态统一**（`useGatewayReadiness`）：
```
输出: 'disconnected' | 'connecting' | 'not_ready' | 'ready'

disconnected → 灰色"离线"
connecting   → 蓝色"连接中..."
not_ready    → 黄色"需配置" ← 点击打开 OnboardingOverlay
ready        → 绿色"就绪"
```

### 2.4 导航分组

| 分组 | 路由 | 覆盖能力 |
|---|---|---|
| **Work** | `/chat` `/editor` `/browser-cockpit` `/terminal` | 核心工作区 |
| **Operate** | `/skills` `/plugins` `/cron` `/dispatch` | 执行与调度 |
| **Observe** | `/engine-health` `/overview` `/logs` | 观测与诊断 |
| **Settings** | `/settings` `/control-center` `/local-models` | 系统与配置 |

侧栏由 `getWorkNavigationSection()`（平铺核心 rail）与 `getGroupedNavigation()`（可折叠四组）共同渲染。新增深链能力必须设 `meta.group`。

---

## 三、后端架构

### 3.1 Rust 核心引擎（core/）

```
core/src/
├── cli/
│   ├── argv.rs          # CLI 路由（19 子命令）
│   └── program.rs       # 路由注册表
├── agent/
│   ├── runtime.rs       # Agent 运行时（tool_cycle · stream_loop · build_model_messages）
│   └── team.rs          # P2P 团队持久化
├── model/
│   ├── byok.rs          # AES-256-GCM 凭据加密
│   ├── prefetch.rs      # 预热去重
│   └── routing.rs       # direct/router/ensemble/bloom
├── mcp/                 # MCP 客户端/服务端/注册表
├── memory/              # 三层记忆（感知/短期/长期）
├── guard/               # 四层护栏 + 自动修复
├── context/             # 上下文引擎（分页/缓存/预算）
├── skills/              # 技能引擎（registry.rs · loader.rs）
├── plugins/             # 插件系统（di.rs · manifest.rs）
├── gateway/
│   ├── server.rs        # WebSocket 服务端
│   ├── methods.rs       # RPC 方法 dispatch
│   ├── chat.rs          # chat.send 契约
│   ├── security_seam.rs # 安全接缝
│   └── rate_limit.rs    # 限流
├── orchestration/       # 编排器（模式切换/蜂群/团队）
├── mode/
│   ├── anchoring.rs     # Bloom 三段锚定（Anchored→Ptc→Full）
│   └── modes/bloom.yaml # Bloom 模式配置 + anti_patterns
├── cache/               # 三层缓存（Tier 0/1/2）
├── rbac/                # 角色访问控制
├── audit_store/         # 审计存储
└── credentials/         # 凭据管理
```

### 3.2 统一网关协议

**WebSocket RPC**：
```
握手: connect.challenge → connect(req: {auth:{token,guestSessionKey}, minProtocol:3, maxProtocol:3, client:{name}}) → HelloOk(protocol:3)

帧格式:
  请求: {type:'req', id, method, params}
  响应: {type:'res', id, ok, payload|error}
  事件: {type:'event', event:'session.event.<type>', payload, meta, seq}

关键方法:
  chat.send          → {message, sessionKey, thinking?, workspaceId?}
  onboarding.status   → ReadinessStatus {first_run, gateway_ready}
  onboarding.catalog  → provider 列表
  onboarding.provider.configure → 保存凭据
  onboarding.provider.probe     → 验证凭据
  mode.get / mode.switch / mode.spec
  cache.status → {tiers, hit_rate}
  usage.status → {total_tokens, total_input_tokens, total_output_tokens}
  usage.query  → {items[].prompt_tokens, completion_tokens, total_tokens, cache_read_tokens}
```

**事件映射**（`envelope_to_frame`）：
```
tools         → session.event.tool_use_start
sessions_changed → sessions.changed（顶层，非 session.event 命名空间）
其余           → session.event.*
```

### 3.3 Python 侧车

```
pyengine/    — OpenSquilla 微内核（技能执行/工具封装/迁移）
workbench/   — 文件智能 + LLM 实验台
  backend/   — FastAPI (port 8011, 回环 only)
  frontend/  — React SPA (9 Panel: Overview/Orchestrate/Model Lab/Files/Memory/Experts/Knowledge/Harness/Research)
```

**双网关桥接**：`core/src/gateway/methods.rs` → `core_bridge` → `workbench/backend/api.py`，共享 `CoreSqlite`。

### 3.4 智能体调用逻辑

```
用户输入
  ↓
CLI 路由 (argv.rs) → 19 子命令 dispatch
  ↓
意图分类 (Orchestrator::analyze) — 离线模式，不调 LLM
  ↓
模式路由:
  plan      → 计划 + 审查（PlanGate C1-C4）
  agent     → 单 Agent + 工具循环
  ensemble  → 多 Agent 协作
  bloom     → 三段锚定（Anchored→Ptc→Full）
  omni      → 全模式融合
  ↓
Agent 运行时 (runtime.rs):
  build_model_messages → 静态区(mode_section→tool_guide) → 插话 → 历史 → 动态区(stable_prefix→experience_hint→route_decision)
  ↓
模型路由 (routing.rs): direct / router / ensemble / bloom
  ↓
工具循环 (tool_cycle):
  MCP tools/call → 四层护栏 → 自动修复 → 上下文引擎 → 记忆沉淀
  ↓
产出 (CompletionGate: 四检查 + 自报文件落盘核验)
```

**KV 缓存优化**（M2 三刀落地）：
- 刀 A: `runtime.rs` — 静态区前置，动态区后置末尾
- 刀 B: `optimize.rs` — `reorder_stable_prefix` 只保留 leading run
- 刀 C: `chat.rs` — PSI 分桶（psi_band/existence_band），不再输出浮点

---

## 四、UI/UX 设计规范

### 4.1 设计令牌（跨端共享）

```css
/* 品牌色 — 蓝主 + 橙强 */
--brand: #1A56DB;           /* light */
--brand-dark: #60A5FA;      /* dark */
--accent-2: #C2410C;        /* light */
--accent-2-dark: #F97316;   /* dark */

/* 语义色 */
--danger: #E5484D;
--success: #3DD68C;
--warning: #F5A524;

/* 主题守卫: CSS 中 var() 不可带 hex fallback */
/* 正确: var(--danger) */
/* 错误: var(--danger, #e5484d) — 架构守卫会报错 */
```

### 4.2 响应式断点

| 断点 | 宽度 | 布局 |
|---|---|---|
| Desktop | ≥1280px | Tauri 窗口默认，侧栏 + 主区域 |
| Tablet | 768–1279px | 可折叠侧栏 |
| Mobile | ≤767px | 底部标签栏切换 |

### 4.3 无障碍

- Icon-only 按钮带 `aria-label`
- 状态变化用 `aria-live="polite"`
- 焦点管理（Modal 关闭后返回触发器）
- 颜色对比度通过 10 主题守卫
- 键盘导航支持（Tab/Enter/Esc）

### 4.4 首次启动引导（OnboardingOverlay）

```
Step 1: 欢迎 → 项目介绍
Step 2: 提供商选择 → onboarding.catalog RPC
Step 3: API Key 输入验证 → onboarding.provider.configure + onboarding.provider.probe
  ↓
就绪态更新: not_ready → ready
  ↓
conn-pill 变绿，ModeHintBar 消失
```

### 4.5 跨端功能映射

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

## 五、安全设计

### 5.1 M1 安全底座（已落地）

| 模块 | 路径 | 测试 | 状态 |
|---|---|---|---|
| RBAC | `core/src/rbac/` (mod 18 / policy 98 / roles 53) | 6 passed | ✅ |
| 审计存储 | `core/src/audit_store/` (214 + tests 91) | 5 passed | ✅ |
| 凭据加密 | `core/src/credentials/` (633 + authorization 371) | 23 passed | ✅ |
| 安全接缝 | `gateway/security_seam.rs` (258) | 5 passed | ✅ |
| 限流 | `gateway/rate_limit.rs` (276) | 8 passed | ✅ |
| 权限边界告知 | `webui/src/components/settings/SetupConnectionPanel.vue` | i18n 6 语言 | ✅ |

### 5.2 安全约束

- 密钥**绝不**写入文件（.env / credentials.json 等不提交）
- API Key 经 AES-256-GCM 加密落盘（`model/byok.rs`），磁盘无明文
- 网关 token 存储在 `$LOCALAPPDATA/Marguerite/gateway-token`（43 字符）
- 所有网络监听仅 `127.0.0.1`，禁止 `0.0.0.0` / 公网

---

## 六、性能优化

### 6.1 KV 缓存前缀稳定（M2 三刀落地）

| 修复 | 文件 | 效果 |
|---|---|---|
| 刀 A | `runtime.rs` | 静态区前置 + 动态区后置 → 服务端 KV 前缀匹配 |
| 刀 B | `optimize.rs` | `reorder_stable_prefix` 只保留 leading run |
| 刀 C | `chat.rs` | PSI 分桶（psi_band/existence_band），字节稳定 |

### 6.2 三层缓存

| 层 | 策略 | 说明 |
|---|---|---|
| Tier 0 | 精确匹配 | 与网关共享 `Arc<PrefixCache>`（`Arc::ptr_eq` 为真） |
| Tier 1 | TTL 过期 | 常规缓存 |
| Tier 2 | 语义近似 | 默认关闭（`semantic_enabled=false`） |

**工具路径永远走 ExactOnly** — 近似命中会静默丢弃工具调用，是正确性缺陷。

### 6.3 Bench-30 测量闭环

- `bench-30.json`：30 条 / 25 编程 / 10 场景类别
- `bench-30-runner.cjs`：`--validate` / `--record` / `--summarize`
- 测量协议：per-task 记录 `{task_id, variant, run, prompt/completion/total/cache_read_tokens, success}`
- pilot 实测通过（真实 DeepSeek 调用，零 mock）

---

## 七、质量门禁

### 7.1 统一验证闸门

```bash
bash scripts/verify.sh --clean --all  # 完成声明用此档
```

| 检查项 | 基线 | 守卫 |
|---|---|---|
| core lib 测试 | 2412 passed + 0 failed + 17 ignored | 不得回退 |
| webui vitest | 3148 passed / 305 files | 不得回退 |
| clippy | L3 零警告（`-D warnings`） | workspace 全量 |
| 架构守卫 | 10 项全绿 | colors/motion/radius/theme-contract |
| i18n | 6 语言 full parity | 无未翻译键 |
| 文档/CLI 漂移 | 0 | check-doc-cli-drift.py |
| 第三方集成 | manifest 自洽 + 18/18 清单 + 5 技能 | validate_integration.py |

### 7.2 验证铁律

```
无新鲜验证证据，不得声称完成 / 通过 / 已修复 / 已构建。
NO COMPLETION CLAIMS WITHOUT FRESH VERIFICATION EVIDENCE
```

- 完成声明前必须运行 `verify.sh --clean --all`，取得全绿证据
- 产出文件用 `assert-deliverables.sh` 确认存在且非空
- 权威完成验证须带 `--clean`（破除增量缓存遮蔽）

---

## 八、合并执行结果

### 8.1 目录合并

| 操作 | 数量 | 状态 |
|---|---|---|
| 内层条目提升到外层 | 179 | ✅ 完成 |
| 临时文件清理 | 123 | ✅ 完成 |
| 空壳目录删除 | 5（deliverables/.ruff_cache/.dsh-home/.cargo_home/.tmp_build） | ✅ 完成 |
| `dist/` 空目录 | 1 | ⚠️ Windows 文件锁延迟，重启后可删 |
| `marguerite/` 空目录 | 1 | ⚠️ 因 dist/ 锁未完全删除 |

**残留处理**：`dist/` 是空目录（0 字节），不影响任何功能。重启后执行 `rd /s /q D:\Downloads\Marguerite\marguerite` 即可完全清除。

### 8.2 路径验证

| 路径引用 | 类型 | 合并后 | 状态 |
|---|---|---|---|
| `Cargo.toml` workspace members | 相对 | `["core","gui/src-tauri","tui"]` | ✅ |
| `verify.sh` ROOT | 相对 | `SCRIPT_DIR/..` | ✅ |
| `package_app.cmd` REPO_ROOT | 相对 | `%~dp0..` | ✅ |
| `tauri.conf.json` frontendDist | 相对 | `webui/dist` | ✅ |
| `.gitignore` | 相对 | 无 `marguerite/` 前缀 | ✅ |
| `cargo check -p marguerite-core` | 编译 | 19.99s 通过 | ✅ |

### 8.3 产物清单

| 产物 | 路径 | 大小 |
|---|---|---|
| 合并方案文档 | `docs/UNIFIED_WORKBENCH_MERGE_PLAN.md` | 8.2KB |
| 架构可视化图 | `docs/unified-workbench-architecture.html` | 6.8KB |
| 前后端设计方方案 | `docs/UNIFIED_WORKBENCH_DESIGN.md`（本文档） | 12KB |
| README | `README.md`（已更新为统一工作台） | 4.5KB |
| MSI 安装包 | `dist/Marguerite_0.2.0_x64_en-US.msi` | 318MB |
| NSIS 安装包 | `dist/Marguerite_0.2.0_x64-setup.exe` | 302MB |

---

## 九、后续路线

| 优先级 | 任务 | 依赖 |
|---|---|---|
| P0 | `verify.sh --clean --all` 全量验证 | 当前合并完成后 |
| P0 | 重启后删除 `marguerite/` 残留 | Windows 文件锁释放 |
| P1 | marguerite-agent-web 21 审查改进点修复 | C-01~L-04 |
| P1 | marguerite-desktop 全量视觉重设计 | Editorial Tech → 东方草木 |
| P2 | M2 降耗 optimized 变体实现 + bench-30 A/B | 测量闭环已就绪 |
| P2 | M3 真实 SSH/WS 传输 + 主机池 | 外部前置 |
| P3 | memory + distill 自动蒸馏 | P2 完成后 |
| P3 | tool_cycle + stream_loop DAG 并发 | P2 完成后 |

---

*本方案在 agent-engineering-core 技能 R1-R10 铁律约束下产出。*
*所有架构不变量（INV-1~9）和 Bloom 红线（RED-1~2）均已对照确认。*
