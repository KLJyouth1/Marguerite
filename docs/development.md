# 开发指南

## 环境要求

- **Rust**：stable 工具链（MSRV 满足 edition 2021）

- **Node.js**：≥20（用于 WebUI：Vue3 + Vite + Pinia）

- **Python**：≥3.10（漂移门禁 / 文档校验）

- **系统**：Windows / macOS / Linux

## 总体架构 ASCII 图（V2.1 权威）

```
┌──────────┬────────────┬──────────────────────────────────┬──────────────────────┐
│  CLI     │  TUI       │  GUI (Tauri React 桌面)          │  Web 控制台         │
│          │            │  ┌─主窗─┐  ┌─Pet Overlay─┐      │  127.0.0.1:18790    │
│ bin:     │ bin:       │  │      │  │ transparent  │      │  回环 only, 禁公网  │
│ marguerite│ marguerite│  │Main  │  │ (/pet-overlay)│      │  ┌control/pet┐     │
│ -core     │ -tui      │  │Window│  │ 拖拽/抚摸/菜单│      │  ├control-center┤   │
│ 19 subcmd │ ratatui   │  └──┬───┘  └──────┬───────┘      │  └...────────┘     │
└────┬─────┴──────┬─────┴─────┼───────────────┼──────────────┴──────────┬─────────┘
     │            │           │               │                          │
     ▼            ▼           ▼               ▼                          ▼
     └────────────┴───────────┴───────────────┴──────────────────────────┘
                           统一网关 (core/src/gateway)
                      HTTP JSON-RPC + WebSocket (127.0.0.1)
                     ┌─────────────────────────────────────────┐
                     │         三层缓存 MultiTierCache          │
                     │  T0 PrefixCache(共享Arc)·T1 LRU·T2 语义 │
                     │  tools 路径强制 ExactOnly / 诚实命中率  │
                     └─────────────────────────────────────────┘
                                       ▼
                         Rust 核心引擎 (core/)
          CLI路由 · 意图分类 · Agent运行时 · 模型路由 · Pet RPC
          MCP · 三层记忆 · 四层护栏 · 上下文 · 技能/插件
          模式系统(plan/agent/ensemble/bloom/omni) · 编排器
          Bloom 三段锚定 Anchored→PTC→Full（单调扩展，不回退）
                                       ▼
                          Python 侧车 (pyengine/)
                    OpenSquilla 微内核 · migrations · tests
```

## 项目结构（V2.1 权威）

```
Marguerite/
├── Cargo.toml            # workspace members = ["core","gui/src-tauri","tui"]
├── core/                 # Rust 核心 + CLI + TUI 引擎 + ACP 薄壳 + Pet RPC
│   └── src/
│       ├── cli/{argv.rs,program.rs}   # 19 路由三处同步（SUBCOMMANDS / register / 测试断言=19）
│       ├── gateway/                   # 统一网关 + Pet 类型(pet.rs) + 5 方法
│       ├── mode/anchoring.rs          # Bloom 三段锚定 Anchored/PTC/Full
│       ├── cache/                     # MultiTierCache(T0共享前缀计量统一)
│       └── plugins/manifest.rs        # PluginManifest 接入 harness
├── tui/                  # ratatui 终端工作台 bin (marguerite-tui)
├── gui/                  # Tauri 桌面壳（唯一原生桌面入口 bin: marguerite-gui）
│   └── src-tauri/src/lib.rs           # 4 个 pet_overlay_* 命令 + 托盘菜单
├── webui/                # Vue3 控制台（构建产物由统一网关回环托管）
│   ├── src/rpc/petContract.ts         # Pet 前端契约（枚举/DTO/RPC 封装）
│   ├── src/views/PetView.vue          # /control/pet（3D 舞台 + 设置）
│   ├── src/views/PetOverlayView.vue   # /pet-overlay（悬浮窗透明画布）
│   ├── src/views/ControlCenterView.vue# /control/control-center（锚定/缓存/心跳/矩阵）
│   └── src/router/{webRoutes,desktopRoutes,index,nav}.ts
├── pyengine/             # Python 引擎侧车（经网关代理方法组）
├── third_party/          # 18 harness manifests + 5 skills YAML + validate_integration.py
├── skills/               # 内置技能 YAML（JIT 按意图触发）
├── docs/                 # cli-reference.md / mcp.md / development.md 等
└── scripts/
    ├── verify.sh                       # 统一闸门（--fast/--core/--webui/--all/--clean/--release）
    ├── check-doc-cli-drift.py          # 文档/CLI 漂移 CI 门禁
    └── assert-deliverables.sh          # 交付物落盘校验
```

## 常用命令

```bash
# 构建（工作区）
cargo build

# 运行 CLI（交互模式 / 单指令 / 子命令）
cargo run -p marguerite-core --
cargo run -p marguerite-core -- -p "你的指令"
cargo run -p marguerite-core -- doctor

# 测试
cargo test                                # 全部测试（单元 + E2E + 性能门禁）
cargo test -p marguerite-core --lib       # 单元测试
cargo test -p marguerite-core --test e2e  # E2E 测试
cargo test -p marguerite-core --test perf -- --nocapture   # 性能基准

# 质量门禁
cargo clippy --workspace --all-targets    # L3: 零 warning
cargo audit                               # L4: 零已知漏洞
cargo build --workspace                   # L1: 零错误零警告

# 覆盖率
cargo llvm-cov --workspace --html         # 生成覆盖率报告（需安装 cargo-llvm-cov）
```

## 质量门禁（提交前必须通过）

| 级别 | 检查               | 命令                                       |
| -- | ---------------- | ---------------------------------------- |
| L1 | 构建零错误零警告         | `cargo build --workspace`                |
| L2 | 测试 100% 通过       | `cargo test --workspace`                 |
| L3 | clippy 零 warning | `cargo clippy --workspace --all-targets` |
| L4 | 依赖零已知漏洞          | `cargo audit`                            |
| L5 | E2E 核心场景通过       | `cargo test --test e2e`                  |
| L6 | 调用链路 < 50ms      | `cargo test --test perf -- --nocapture`  |

## 代码规范

- **精简**：避免冗余嵌套（如 if 层层包裹），追求简单高效、易维护。

- **注释**：仅在逻辑不自明处添加；公共 API 使用 doc 注释。

- **错误处理**：内部代码信任框架保证，仅在系统边界（用户输入、外部 API）做校验。

- **新增依赖**：优先自研轻量实现，避免引入重量级库。

## 添加新意图

1. 在 [`core/src/intent/classifier.rs`](../core/src/intent/classifier.rs) 的 `IntentType` 中新增枚举项。
2. 在 `default_rules()` 中注册关键词与正则规则。
3. 在 [`core/src/intent/router.rs`](../core/src/intent/router.rs) 中注册 `IntentHandler` 并映射到技能。
4. 补充单元测试，验证分类准确率与置信度。

## 添加新护栏层

1. 在 [`core/src/guard/`](../core/src/guard/) 下新建模块（参考 `syntax.rs` 结构）。
2. 实现检查函数，返回 `Vec<GuardIssue>`（含 `GuardLayer` / `Severity` / 修复建议）。
3. 在 [`core/src/guard/mod.rs`](../core/src/guard/mod.rs) 的 `GuardGateway` 中接入新层。
4. 如涉及自动修复，在 `repair.rs` 中注册修复规则。

## 添加新技能

1. 创建 `skills/<name>.yaml`（格式见 [skills.md](skills.md)）。
2. 在 `intent` 中加入触发关键词。
3. 运行测试验证加载与执行链路。

## 添加新插件

1. 实现 [`core/src/plugins/mod.rs`](../core/src/plugins/mod.rs) 中的 `Plugin` trait（`meta()` / `on_load()` / `capabilities()`）。
2. 通过 `PluginRegistry` 注册，支持热插拔。

## 前端架构（四端统一：CLI / TUI / Tauri 桌面 / Web 控制台）

自 V2 起统一工作台采用「单 Rust 引擎 + 三前端（ratatui TUI / Tauri React 桌面 / Vue3 Web 控制台）」架构；
Web 端仅由回环网关（127.0.0.1）托管，不开放公网。四端共享蓝橙语义令牌、同一 Pet RPC、同一三层缓存与模式系统。

| 前端      | 技术                           | 工作区成员                                    | 说明                                                |
| ------- | ---------------------------- | ---------------------------------------- | ------------------------------------------------- |
| CLI     | Rust                         | `core/` (bin: `marguerite-core`)         | 19 子命令；结构化错误 + `--json` 契约；ASCII 状态标记             |
| 终端工作台   | ratatui                      | `tui/` (bin: `marguerite-tui`)           | 五区全帧 · Bloom 阶段 · 实时心跳                            |
| 桌面应用    | Tauri v2 (React 壳 + Rust 后端) | `gui/src-tauri/` (bin: `marguerite-gui`) | 主窗 + Pet 悬浮窗（透明置顶，4 条 invoke 命令）                  |
| Web 控制台 | Vue3 + Vite + Pinia          | 构建产物由网关托管                                | 回环 127.0.0.1:18790；PetView / ControlCenter / 四组导航 |

> 旧 `desktop/`（egui）已从工作区移除并删除；旧冻结声明作废。所有桌面新能力统一落在 `gui/src-tauri/` 与 `webui/`。

### 桌宠悬浮窗（V2.1-P2 + V2.2）与 Tauri 联调

**命令契约**（`gui/src-tauri/src/lib.rs`，已注册进 `generate_handler!`）：

| 命令                   | 行为                                   | 校验                                    |
| -------------------- | ------------------------------------ | ------------------------------------- |
| `pet_overlay_show`   | 创建/显示悬浮窗（幂等；透明/无边框/置顶/跳过任务栏，缺省屏幕右下角） | 纯逻辑 `pet_overlay_default_position` 单测 |
| `pet_overlay_hide`   | 隐藏（幂等）                               | —                                     |
| `pet_overlay_toggle` | 托盘快捷切换                               | —                                     |
| `pet_overlay_move`   | 拖拽到新坐标 `(x,y)`（前端 dragend 上报）        | —                                     |

**自动门禁**（无头 CI 可跑）：

- `cargo check -p marguerite-gui`：命令 + 托盘 + handler 编译门禁（0 warning）

- `cargo test -p marguerite-gui --lib pet_overlay`：悬浮窗纯逻辑 4 单测（缺省位置 / 契约常量）

**人工冒烟清单（`cargo tauri dev`，需桌面会话 + WebView2）**：

1. 启动后托盘菜单「桌宠悬浮窗 → 显示」→ 右下角出现透明宠物（主窗不遮挡）
2. 顶部细条拖拽 → 窗口跟随移动（`pet_overlay_move` 生效）
3. 双击宠物 → 心情胶囊/好感度变化（`pet.triggerAction` 经网关往返）
4. 右键 → 菜单：换肤（5 款）/ 投喂 / 打开设置 / 隐藏
5. 托盘「隐藏」→ 悬浮窗消失且主进程不退出
6. Web 端 `http://127.0.0.1:18790/control/pet` 与悬浮窗共享同一 `pet.*` RPC 状态

> 说明：`cargo tauri dev` 依赖真实桌面会话，无法在无头 CI 自动断言；以上为可重复的人工验收路径，
> 自动化覆盖由「纯逻辑单测 + GUI 编译门禁 + Web 端 Playwright 握手（`e2e/pet.spec.ts`）」三档补齐。

## 测试策略

- **单元测试**：覆盖核心模块（意图 / 护栏 / 记忆 / 上下文 / 任务队列等），内嵌 `#[cfg(test)]`。

- **E2E 测试**（`tests/e2e.rs`）：5 大闭环——幻觉拦截、记忆固化检索、token 经济、双模型计划、任务队列后台执行。

- **性能基准**（`tests/perf.rs`）：调用链路（意图 → 护栏 → 分页 → 记忆检索）平均 < 50ms，先预热再计时。

- **Web 端 E2E（Playwright）**：`webui/e2e/pet.spec.ts`（V2.2）覆盖 `/control/pet` 与 `/pet-overlay` 的
  RPC 握手（`pet.list` 5 皮肤 / 3D 画布挂载 / `pet.status` / `pet.triggerAction` 往返）。运行方式：
  先构建 `target/debug/marguerite-core.exe` 并 `gateway --port <N>` 启动回环网关（托管 `webui/dist` + `pet.*` RPC），
  再 `OPENSQUILLA_WEBUI_BASE_URL=http://127.0.0.1:<N> npx playwright test e2e/pet.spec.ts`。

