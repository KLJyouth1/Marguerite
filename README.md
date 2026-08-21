# Marguerite CLI Workbench

极致精简的 Agent 工作台，CLI 为第一公民。

- **Rust 核心引擎**（`core/`）：CLI 路由、意图系统、Agent 运行时、MCP、三层记忆、幻觉护栏、上下文引擎、技能 / 插件系统
- **Tauri GUI 壳**（`gui/`）：React 轻量前端 + Rust 薄封装
- **质量门禁**：构建零警告、测试 100% 通过、clippy 清零、依赖审计、调用链路 < 50ms

## 快速开始

```bash
# 构建
cargo build

# 交互模式（TUI，普通消息走 AgentRuntime 真实回复）
cargo run -p marguerite-core --

# 单指令模式（接入 AgentRuntime，含 -f 文件内容 / --pipe 标准输入）
cargo run -p marguerite-core -- -p "你的指令"
cargo run -p marguerite-core -- -p "解释 main.rs" -f main.rs
echo "你的指令" | cargo run -p marguerite-core -- --pipe

# 子命令
cargo run -p marguerite-core -- doctor          # 系统诊断
cargo run -p marguerite-core -- model list      # 列出模型路由规则
cargo run -p marguerite-core -- mcp list        # 列出已配置 MCP 服务器
cargo run -p marguerite-core -- skills list     # 列出已安装技能
cargo run -p marguerite-core -- skills install <yaml-url>   # 远程安装技能
cargo run -p marguerite-core -- session list    # 列出会话
cargo run -p marguerite-core -- session delete <id>
```

首次运行会在 `~/.marguerite/config.yaml` 生成配置（模板见 [config.yaml](config.yaml)）。
设置 `model.api_key` 后，TUI 普通消息与 `-p` 单指令将获得真实 LLM 回复；
未配置时应用会明确提示，其余功能不受影响。

TUI 内置 slash 命令：`/help` `/new` `/clear` `/model` `/skills` `/status` `/compress` `/exit`。

## 文档

| 文档 | 说明 |
| --- | --- |
| [架构](docs/architecture.md) | 模块分层、数据流、配置与质量门禁 |
| [技能系统](docs/skills.md) | 技能 YAML 格式、内置技能、新增步骤 |
| [MCP 集成](docs/mcp.md) | 客户端 / 服务端 / 工具注册表 |
| [记忆系统](docs/memory.md) | 三层记忆架构与检索策略 |
| [开发指南](docs/development.md) | 环境、命令、代码规范、测试策略 |

## 测试

```bash
cargo test                                # 单元 + E2E + 性能门禁
cargo clippy --workspace --all-targets    # 零 warning
cargo audit                               # 零已知漏洞
```

## 架构一览

```
用户输入 → CLI 路由 → 意图分类 → 模型路由 → Agent 运行时
  → 工具循环 / MCP → 幻觉护栏（语法/语义/事实/契约）→ 自动修复
  → 上下文引擎（分页/缓存/预算）→ 记忆沉淀（感知/短期/长期）→ 输出
```

详见 [架构文档](docs/architecture.md)。

## 已集成的第三方 AI Harness

| Harness | 集成方式 | 插件/模块数 | 说明 |
|---|---|---|---|
| **DeepSeek Harness (DSH)** | PluginManifest 插件化（8 份 manifests）+ 3 个 Rust 原生模块 | 10 plugins 二创复刻 | 视觉 Modlens / 交互 GenUI / Daily Usage / 鲸鱼娘 Pet / Mini Games / Market / Better Sidebar / Router 3 模式；7 条 slash 命令；63 条单测全绿；详细见 [DSH 集成指南](docs/dsh-integration-v1.md)。 |
| UnivAgent | 架构对齐 + 子代理编排增强 | — | 见 `univagent-consolidation-to-marguerite-v1.md`。 |

> **DSH 集成亮点（2026-08-19 v1.0）**
> - 解决「视觉→交互→工作台→路由器」工作流中断：4 阶段 E2E 闭环 1 次跑通。
> - Usage：SQLite 聚合（by_day/by_month）+ 字符热力图 + budget 阈值告警。
> - GenUI：Card/Button/Progress/Form 全类型 Widget + 点击回传 SteeringEvent。
> - Pet：6 帧 ASCII 状态机 + 摸头/投喂 + affinity 持久化。
> - 不新增任何原生 Cargo 依赖；零 Electron/Tauri/WebView（宪法 D12/D16 守护）。
