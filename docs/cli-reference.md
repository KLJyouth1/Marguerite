# Marguerite CLI 参考手册（v0.1.5）

> 本手册对应 `marguerite-core` / `marguerite-tui` 二进制（构建产物名 `marguerite-core.exe` / `marguerite-tui.exe`）。
> 适用于 **v0.1.5**。所有子命令均支持 `--help` 查看内嵌帮助（退出码 0）。
> 工作台形态（Solo / MAX / 团队 / 集群）由编排内核**自动**决定，无需、也不提供手动选择开关。

---

## 1. 概览

Marguerite 是终端优先的智能体工作台：单 Rust 引擎（`marguerite-core`）+ 双原生前端（ratatui TUI / egui 桌面），**无 Web 端**。未配置 API Key 时自动降级为离线回显。

| 形态 | 启动方式 | 说明 |
|------|----------|------|
| 交互工作台 | `marguerite` | 默认；三栏 ratatui 工作台 |
| 单指令 | `marguerite -p "指令"` | 执行后退出，适合脚本/管道 |
| 管道 | `marguerite --pipe` | 从 stdin 读取指令 |
| 子命令 | `marguerite <资源> <动作>` | 见 §4 |

> 排障回退：设置环境变量 `MARGUERITE_LEGACY_UI=1` 可启动旧版单栏 TUI（仅用于排查渲染问题）。

---

## 2. 全局选项

| 选项 | 说明 |
|------|------|
| `-h`, `--help` | 显示顶层帮助，退出 0 |
| `-V`, `--version` | 显示版本号，退出 0 |
| `-p <指令>`, `--prompt <指令>` | 单指令模式 |
| `-f <文件>`, `--file <文件>` | 将文件内容附加到指令（文件不存在则警告并继续） |
| `--pipe` | 从 stdin 读取指令 |
| `--json`, `--output-format json` | 机器可读输出（见 §5） |

未知全局选项（如 `--bogus`）→ 退出码 **2** 并打印用法。
未知裸词子命令（如 `frobnicate`）→ 退出码 **2** 并打印可用子命令列表。

---

## 3. 退出码

| 码 | 含义 | 典型触发 |
|----|------|----------|
| 0 | 成功 | 正常完成；`--help`/`--version` |
| 1 | 一般错误 | 单指令执行失败；沙箱拒绝写命令；子命令运行期错误 |
| 2 | 用法错误 | 未知选项 / 未知子命令 / 缺必选参数 |
| 3 | 认证失败 | API Key 缺失或无效 |
| 4 | 资源未找到 | 会话/文件等不存在 |
| 5 | 临时失败（可重试） | 网络错误 / 限流 |
| 9 | 冲突 | 并发 / 状态不一致 |

`--json` 模式下，错误信息为 `{"ok":false,"error":{"code","message","hint","retryable"},...}`，`code` 字段与退出码对应。

---

## 4. 子命令

### 4.1 `doctor` — 系统诊断

```
marguerite doctor           运行诊断，输出环境摘要（文本）
marguerite doctor --json    以 JSON 输出（便于脚本 / agent 接入）
```

`--json` 字段：`rust_arch`, `os`, `config_dir`, `model{name,provider}`, `api_key_configured`, `mcp_servers`, `skills_dir`, `status`。

示例：
```
marguerite doctor
marguerite doctor --json | jq .api_key_configured
```

### 4.2 `model` — 模型档案管理

```
marguerite model list                                   列出全部档案
marguerite model current                                显示当前主 / 副模型
marguerite model use <id>                               切换主模型
marguerite model copilot <id|off>                       设置 / 关闭 Bloom 副模型
marguerite model add --id <id> --base-url <url> \
    --model <name> [--provider <p>] [--label <l>] \
    [--api-key <k>] [--kind cloud|local|custom]         新增自定义接入点
marguerite model remove <id>                            删除档案（至少保留一个）
marguerite model path                                   显示 models.json 路径
```

示例：
```
marguerite model list
marguerite model use deepseek-v4-pro
```

### 4.3 `mcp` — MCP 工具服务器管理

```
marguerite mcp list     列出 config.yaml 中已配置的 MCP 服务器
marguerite mcp tools    实时连接带 url 的 HTTP MCP 服务器并拉取工具目录
```

示例：
```
marguerite mcp list
marguerite mcp tools
marguerite mcp list | cat   # 脚本中读取服务器清单
```

> MCP 客户端桥：配置 `mcp.servers[].url` 后，引擎启动时自动连接并拉取工具目录，注册进 ToolRegistry（describe_mcp_tools / McpToolExecutor 缓存执行）；命令式 stdio 桥为下一阶段。

### 4.4 `skills` — 技能管理

```
marguerite skills list           列出已安装技能
marguerite skills install <url>  从 YAML URL 安装技能
```

示例：
```
marguerite skills list
marguerite skills install https://example.com/skill.yaml
```

**插件运行时（claude-code-plugins 融合，Phase C-4）**：声明式 `PluginManifest`（hooks / slash_commands / subagents / skills）经 `ManifestPlugin` 桥接进既有 `PluginRegistry`，与原生编译期插件共存；`HookRegistry` 对齐 Claude Code 的 PreToolUse / PostToolUse / Notification / Stop 钩子派发，钩子执行复用 L1/L2/L3 护栏概念。接口位于 `core/src/plugins/manifest.rs`。

### 4.5 `session` — 会话管理

```
marguerite session list           列出最近会话
marguerite session delete <id>    删除指定会话
```

示例：
```
marguerite session list
marguerite session delete <前 8 位 ID>
```

### 4.6 `exec` — 终端沙箱命令执行

```
marguerite exec -- <命令> [参数...]    经终端沙箱门控后执行
```

> 沙箱 fail-closed：非交互 CLI 下，写类命令（如 `rm`）默认拒绝。`--` 之后的参数原样传递，例如 `marguerite exec -- ls -h` 中的 `-h` 透传给 `ls`。

示例：
```
marguerite exec -- ls -la
marguerite exec -- git status
```

### 4.7 `deploy` — 一键部署

```
SSH : marguerite deploy --target ssh://[user@]host[:port][/dir] \
        [--bin <release二进制>] [--service <name>] [--health-url <url>] \
        [--no-systemd] [--dry-run] [--user <u>] [--port <p>] [--identity <key>]
宝塔: marguerite deploy --target baota://<别名> [--bin <二进制>] [--site <域名>] \
        [--service <name>] [--health-url <url>] [--no-ssl] [--dry-run]
注册: marguerite deploy --target baota://<别名> --add-panel \
        --endpoint https://host:8888 --api-key <面板API密钥>
```

流程：探针 → 建目录 → 本地 sha256 → 上传 → 快照回滚点 → 就位 → 安装(systemd/nohup) → 健康探活 →（失败）回滚。`--dry-run` 仅演练远端步骤，不实际连接。

示例：
```
marguerite deploy --target ssh://user@host:22/opt/marguerite --dry-run
marguerite deploy --target baota://mysite --add-panel --endpoint https://host:8888 --api-key $KEY
```

---

### 4.8 `serve` — ACP 回环服务

启动或探测 ACP（Agent Client Protocol）回环服务。**仅绑定 `127.0.0.1`，绝不暴露公网**。

```
健康检查（非阻塞）: marguerite serve --health
启动监听（阻塞）  : marguerite serve [--port <N>]     # 默认端口 18181
```

- `serve --health`：返回 JSON 健康态（校验回环端口可绑定 + API Key 是否已配置），进程立即退出，适合探针 / CI。
- `serve [--port N]`：在 `127.0.0.1:N` 启动 ACP listener，提供 12 个方法（见下「ACP 方法面」）；`agent.invoke` 路由到 `Orchestrator` 离线分析（不调 LLM），`tools.*` 复用 `ToolRegistry` 执行本地工具，`memory.*` 复用 KB 2.0 + claude-mem 融合记忆后端。阻塞运行，按 `Ctrl+C` 退出。

**ACP 方法面（hermes-agent + claude-mem 融合，Phase C-2 / C-3）**：
| 方法 | 说明 |
|------|------|
| `ping` | 健康检查探针，回 `pong` + 会话 id |
| `session.info` | 会话元信息（方法清单 / 回环标志 / sidecar 地址） |
| `agent.invoke` | 经注入的 `AgentInvoker` 调用引擎（离线分析） |
| `health` | 引擎健康快照（转发 invoker.health） |
| `tools.list` | 枚举本地工具（名称/描述/输入 schema），来自 `ToolRegistry::describe_local_tools` |
| `tools.invoke` | 经 `ToolRegistry::execute_local_tool` 执行本地工具，参数 `name` + `args{...}`；复用既有 `ToolExecutor` 回路与分层安全护栏 |
| `permissions.list` | 分层护栏能力清单（L1 命令/执行/SSRF、L2 恶意代码扫描、L3 操作护栏） |
| `auth.capabilities` | 回环信任模型（无外部鉴权、会话作用域） |
| `events.list` | 引擎可发射事件目录（订阅能力 `not_implemented`，诚实标注） |
| `session.list` | 当前活跃会话（ACP 薄壳按 listener 单会话） |
| `memory.search` | 按 `category`（user/project/feedback/auto）+ `query` 召回 top_k 条记忆；`project` 走 KB 2.0 BM25，其余走内存映射（claude-mem 融合，Phase C-3） |
| `memory.remember` | 写入一条记忆（`category` + `key` + `value`），`project` 入 KB 2.0 索引，其余入内存映射 |

> 工具回路仅在 `serve` 注入了工具注册表时启用 `tools.*`；`chat --tools` 与 `serve` 共用同一 `register_builtin_tools`，横幅/计数均动态取自注册表（当前约 24 个内置原生工具）。

`--health` 的 JSON 字段（`data` 内）：`status`（`ok`/`degraded`）、`loopback_bindable`、`port`、`acp_methods[]`、`engine{tools,research_docs,automations}`、`config{model,provider,api_key_configured}`。

示例：
```
marguerite serve --health           # 探针：{"ok":true,"data":{"status":"ok",...}}
marguerite serve --port 19090       # 在回环 19090 启动 ACP 服务
```

> 安全约束：无论端口如何指定，`serve` 永远只监听 `127.0.0.1`（loopback）。不要尝试将其绑定到 `0.0.0.0`。

---

### 4.9 `chat` — 流式对话（纯终端 REPL）

Codex 风格流式对话：可进入交互式 REPL，也可单发一句话。支持接续已有会话（多轮持久化）。

```
marguerite chat                               # 进入交互式流式 REPL（Ctrl-D / /exit 退出）
marguerite chat "<问题>"                      # 单发：流式打印一个回答后退出（等价于 -p，但走流）
marguerite chat --json "<问题>"               # 单发并以 JSON 返回 {"ok":true,"data":{"response":...},"meta":...}
marguerite chat --tools                       # 进入 REPL 并启用 LLM function calling 工具回路
marguerite chat --tools "<问题>"              # 单发并以工具回路作答（文件/Shell/检索等内置工具）
marguerite chat -c                            # 恢复最近一次会话继续对话
marguerite chat --resume <session-id>         # 恢复指定会话（支持 session list 里的短 id 前缀）
marguerite chat -c "接着上面..."              # 续聊时带初始 prompt
```

**会话选项**：
- `-c`, `--continue`：恢复最近一次会话继续对话。
- `--resume <session-id>`：恢复指定会话（支持 `session list` 输出的短 id 前缀）。
- `-t`, `--tools`：启用 LLM function calling 工具回路（21 个内置原生工具）。

**REPL 内斜杠命令**：

| 命令 | 作用 |
|------|------|
| `/help` | 显示命令清单 |
| `/history` | 显示当前会话历史（角色 + 内容预览） |
| `/clear` | 清空对话历史（不删除已落盘会话，当前会话 id 不变） |
| `/save` | 立即保存当前会话；空历史时提示「无需保存」，不产生空记录 |
| `/sessions` | 列出最近 10 个会话，当前会话标 `*` |
| `/load <id>` | 切换到指定会话（支持短 id 前缀匹配） |
| `/model` | 显示模型路由说明（模型由 router 按意图/复杂度自动选择） |
| `/exit` 或 `/quit` | 退出 REPL（或按 Ctrl-D） |

- REPL **每轮对话自动落盘**，与 `marguerite session list` / `marguerite session delete` **共用同一存储**（`$1marguerite/sessions.db`，SQLite）。REPL 留下的会话可在终端用 `session` 子命令管理。
- 裸单发（无 `-c`/`--resume`）**不落盘**；带 `-c`/`--resume` 的单发会接续既有上下文并落盘。
- 落盘失败时 REPL 仍可继续对话（存储为可降级封装，失败仅提示一次，不中断会话）。
- 无 API Key 时自动离线降级。

示例：
```
marguerite chat  # 进入 REPL
marguerite chat -c  # 恢复最近会话
marguerite chat --resume 3f2a1b9c  # 恢复指定会话
marguerite chat --tools  # 启用工具回路
marguerite chat --tools "读取 src/main.rs 并加超时"  # 单发+工具
marguerite chat "用 Rust 写一个带超时的 TCP 探测"  # 单发
marguerite chat -c "接着上面，补充错误处理"  # 续聊
marguerite chat --json "一句话解释 tokio runtime" | jq .data.response  # JSON 输出
```

> 工具回路（`--tools`）：REPL 与单发均可启用 LLM function calling。`model/llm.rs` 已补齐 function calling（请求体 `tools` 字段 + 流式 `tool_calls` 累积解析），`agent/runtime.rs::run_tool_loop` 驱动 plan→act→observe 闭环，复用 `agent/tool_cycle.rs` 的 `ToolCycle`（顺序执行 + 超时保护）与 `mcp/registry.rs` 的 `ToolRegistry`（经 `describe_local_tools` 生成 `ToolDef`）。启用后模型可在对话中调用 21 个内置原生工具（文件读写/Shell/检索/Git 等），工具调用过程实时打印。详见整合方案 Phase C-1。

---

### 4.10 `subagent` — 子代理团队 DAG 编排

`marguerite subagent team "<目标描述>" [--reflect]`：分层 DAG 编排（调研 → 规划 → 实现 → 审校），子代理经 `SubAgentManager::spawn_with_spec` **真实执行**（真实模型路由 + 真实工具注册表），层间依赖保证——下游节点注入上游全部产出后执行，最终由 lead 综合。

- `--reflect`：启用 **agent-s 分阶段反思（s2_5）** 融合（Phase C-5）。在「实现」与「审校」之间插入显式「反思自检」节点，编排结束后对各行阶段产出（工作记忆）做确定性 go/no-go 自检并打印决策。无 `--reflect` 时走经典 4 阶段 `Build` 形态。
- 治理门前置：含破坏性/越权操作的 DAG 会被宪法门拦截，需人工放行。
- 接口：`core/src/orchestration/dag.rs`（`DagKind` / `decompose` / `topo_layers` / `reflect_pass`）；编排入口 `orchestration::Orchestrator::dispatch_team_dag`。

### 4.11 `browser` — 真实浏览器 CDP 操控

`marguerite browser open <url> [--out <file.png>]`：启动真实 Chromium（headless），导航 → 截图保存 → 提取 DOM 元素 `getBoundingClientRect` 归一化坐标（0..1）打印。截图 + 归一化坐标即 computer-use 的「感知」基础（agent-s 对齐：GUI 操控以截图与元素坐标为输入），后续可扩展为点击/输入等真实动作。

- 依赖本机 Chromium/Chrome 可执行文件；未检测到时提示 `browser_bridge 不可用` 并退出。
- 接口：`core/src/browser/mod.rs`（`BrowserBridge`）。

### 4.12 `sandbox` — 沙箱命令执行

`marguerite sandbox -- <命令>`：在受限沙箱中执行命令（L1 命令护栏 + 执行超时保护），输出经治理门核验后返回。与 `exec` 的区别：`sandbox` 走更严格的隔离与权限边界（`sec.guardrail` 分层安全 L3），适合运行不可信或高权限操作。

---

### 4.13 `games` — 内置小游戏（Rust 引擎，离线可玩）

```
marguerite games list                  列出已实装小游戏（2048 / 井字棋 / 贪吃蛇）
marguerite games play <id> <动作...>   按动作驱动引擎并渲染棋盘/结果
marguerite games list --json           以 JSON 输出（便于脚本 / agent 接入）
```

示例：
```
marguerite games list
marguerite games play tictactoe 0 3 1 4 2
marguerite games play 2048 left up left
marguerite games play snake up left down
```

- 引擎零外部依赖、确定性输出；同一动作序列可复现。
- 统一网关同步暴露 `games.list` / `games.play` RPC，Web 控制台与脚本亦可调用。
- 接口：`core/src/games/{mod,game2048,snake,tictactoe}.rs`。

---

## 5. 输出格式契约

- **stdout = 业务数据**；**stderr = 错误/日志/进度**。日志默认 warn 级，可用 `RUST_LOG` 覆盖。
- **文本模式**：人类可读，无 ANSI 色、无 emoji（便于被 agent 抓取与脚本解析）。
- **JSON 模式**（`--json`）：结构稳定——
  - 成功：`{"ok":true,"data":...,"meta":{"schema_version":"1.0","cli":"marguerite"}}`
  - 失败（stderr）：`{"ok":false,"error":{"code","message","hint","retryable"},"meta":{...}}`

---

## 6. 交互模式速览

启动 `marguerite` 进入三栏工作台（ratatui）。编排内核根据任务自动选择形态，用户输入指令即可，无需选择模式。常用交互：输入指令回车执行；`Ctrl+C` 中断；未配置 API Key 时自动离线回显。

> 完整交互键位与面板说明见工作台内 `?` 帮助（如已实现）或 TUI 源码 `core/src/tui/`。

---

## 7. 常见问题

| 现象 | 处理 |
|------|------|
| `未配置 API Key` | 在 `config.yaml` 的 `model.api_key` 设置；或用 `marguerite model add` 新增接入点 |
| 子命令报 `未知子命令` | 运行 `marguerite --help` 查看可用资源 |
| `exec` 写命令被拒 | 非交互 CLI 下沙箱 fail-closed；交互模式或显式授权后再执行 |
| 部署找不到二进制 | 用 `--bin` 指定 release 单文件，或先构建产出 release |

---

*本手册由 `docs/cli-tui-optimization-plan.md` Phase 2 落地生成；`serve`（§4.8）与 `chat`（§4.9）由 CLI 核心重构 Phase B / B-P1 增补。对应代码 `core/src/cli/{argv,program,output,repl,mod}.rs`。不引入任何 Web GUI，符合项目锁定决策。*
