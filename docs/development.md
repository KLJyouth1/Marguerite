# 开发指南

## 环境要求

- **Rust**：stable 工具链（MSRV 满足 edition 2021）
- **系统**：Windows / macOS / Linux
- **注意**：项目已锁定「放弃所有 Web 端」，双原生前端（TUI / 桌面）均为纯 Rust，无需 Node.js。

## 项目结构

```
marguerite/
├── core/          # Rust 核心引擎（工作区成员，含 CLI 引擎与 ACP 薄壳）
├── tui/           # ratatui 终端工作台（工作区成员，bin: marguerite-tui）
├── desktop/       # egui 桌面应用（工作区成员，bin: marguerite-desktop）
├── gui/           # ⚠️ 已弃用 Web 端（Tauri 壳）；非工作区成员（orphan），已冻结
├── skills/        # 内置技能 YAML
├── docs/          # 文档
└── config.yaml    # 默认配置模板
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

| 级别 | 检查 | 命令 |
| --- | --- | --- |
| L1 | 构建零错误零警告 | `cargo build --workspace` |
| L2 | 测试 100% 通过 | `cargo test --workspace` |
| L3 | clippy 零 warning | `cargo clippy --workspace --all-targets` |
| L4 | 依赖零已知漏洞 | `cargo audit` |
| L5 | E2E 核心场景通过 | `cargo test --test e2e` |
| L6 | 调用链路 < 50ms | `cargo test --test perf -- --nocapture` |

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

## 前端架构（双原生，无 Web 端）

项目已锁定「放弃所有 Web 端」，采用双原生前端，二者均为纯 Rust、共享同一个 `core` 引擎，行为完全一致：

| 前端 | 技术 | 成员 | 运行方式 |
|------|------|------|----------|
| 终端工作台 | ratatui | `tui/`（bin: `marguerite-tui`） | `cargo run -p marguerite-tui` |
| 桌面应用 | egui | `desktop/`（bin: `marguerite-desktop`） | `cargo run -p marguerite-desktop` |

> `gui/` 目录是早期 Tauri（Web）壳，**已弃用并冻结**：它不是工作区成员（`Cargo.toml` 的 `members = ["core","desktop","tui"]` 不含它，且无 `Cargo.toml`）。所有新前端能力应落在 `tui/` 或 `desktop/`，不要在 `gui/` 开发。

## 测试策略

- **单元测试**：覆盖核心模块（意图 / 护栏 / 记忆 / 上下文 / 任务队列等），内嵌 `#[cfg(test)]`。
- **E2E 测试**（`tests/e2e.rs`）：5 大闭环——幻觉拦截、记忆固化检索、token 经济、双模型计划、任务队列后台执行。
- **性能基准**（`tests/perf.rs`）：调用链路（意图 → 护栏 → 分页 → 记忆检索）平均 < 50ms，先预热再计时。
