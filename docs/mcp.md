# MCP 集成

MCP（Model Context Protocol）是 Agent 与外部工具 / 数据源之间的标准协议。Marguerite 同时实现 MCP 客户端与服务端，基于 HTTP + JSON-RPC。

## 架构

```
┌─────────────┐   JSON-RPC (HTTP)   ┌──────────────┐
│ Agent 工具循环 │ ─────────────────→ │ MCP Server   │
│ (tool_cycle) │                     │ (外部服务)    │
└──────┬──────┘                     └──────────────┘
       │ 调用工具
       ▼
┌─────────────┐   HTTP JSON-RPC     ┌──────────────┐
│  ToolRegistry│ ←───────────────── │ McpClient    │
│ (注册表)     │   发现 / 调用        │ (客户端)      │
└─────────────┘                     └──────────────┘
```

## 客户端（`core/src/mcp/client.rs`）

- **异步连接池**：reqwest 复用 HTTP 连接，避免重复握手。
- **工具发现与缓存**：从服务器拉取工具清单，缓存 5 分钟（TTL），超时后自动刷新。
- **标准 MCP 协议**：HTTP JSON-RPC，支持 `initialize` / `tools/list` / `tools/call` 等标准方法。

### 配置

在 `$1marguerite/config.yaml` 中声明 MCP 服务器：

```yaml
mcp:
  servers:
    - name: "my-server"
      url: "http://127.0.0.1:3000/mcp"
```

## 工具注册表（`core/src/mcp/registry.rs`）

统一封装工具执行入口，`ToolExecutor` trait：

```rust
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult>;
    fn input_schema(&self) -> serde_json::Value { serde_json::json!({}) }
}
```

工具循环通过注册表调用工具，结果归并回上下文。

## 服务端（`core/src/mcp/server.rs`）

Marguerite 自身也可作为 MCP 服务器，对外暴露工具：

- **标准 MCP 协议**：HTTP JSON-RPC 请求处理。
- **工具暴露与权限控制**：仅暴露注册且授权的工具。
- **运行控制**：`start()` / `stop()` / `is_running()`，支持端口绑定。

## CLI 管理命令

当前 CLI 仅暴露 `list` 一个动词，用于查看 `config.yaml` 中已声明的 MCP 服务器（连接由引擎在启动时依据 `mcp.servers` 自动建立，无需单独的 `connect` 子命令）：

```bash
marguerite mcp list      # 列出 config.yaml 中已配置的 MCP 服务器
```

> 未匹配到已知子命令时，`mcp` 会打印内嵌帮助并退出（退出码 0）。`connect` / `add` / `remove` 等动词尚未在 CLI 暴露——新增服务器请直接编辑 `config.yaml` 的 `mcp.servers` 字段。

## 关键设计

- **超时与容错**：工具调用默认超时 30s，失败可重试。
- **缓存策略**：工具清单 5 分钟 TTL，减少频繁握手。
- **权限**：服务端暴露工具前校验权限，避免越权调用。

## 宝塔面板（BaoTa）MCP 接入

将宝塔面板（站点 / 数据库 / 文件 / 计划任务 / SSL / 日志 / 终端）封装为**标准 HTTP MCP 服务**，由 Marguerite 引擎通过 `config.mcp.servers` 中带 `url` 的条目在启动时 `best-effort` 连接并注入工具面。

### 架构走向

```
┌──────────────────┐  HTTP JSON-RPC   ┌──────────────────────────────┐
│ Marguerite 引擎   │ ───────────────→ │ baota_bridge/mcp_server.py   │
│ (Orchestrator)    │  tools/list/call │  (BaotaMcpApi 扩展 BaotaAPI) │
│ add_mcp_client()  │ ←─────────────── │  127.0.0.1:18799             │
└──────────────────┘                  └──────────────────────────────┘
```

- Python 服务独立于 Rust 引擎运行，**不需要**将 `baota_bridge` 代码 vendoring 进 Marguerite 仓库。
- 引擎侧默认 `mcp.servers` 为空 → 零行为变化；仅当配置了带 `url` 的宝塔条目才激活。

### 1. 服务端位置与依赖

| 项 | 路径 |
|----|------|
| MCP 服务入口 | `C:\Users\Administrator\WorkBuddy\xiaoyuangou\tools\baota_bridge\mcp_server.py` |
| 底层 API 封装 | 同目录 `baota_api.py`（`BaotaAPI`，已处理 auth/CSRF/Playwright 登录） |
| 凭据模板 | 同目录 `.env.example` → 复制为 `.env` |

依赖（在 `baota_bridge` 自己的 venv 中安装）：`flask`/`http.server`（标准库即可）、`requests`、`pycryptodome`、`playwright`（终端/登录需要）。

### 2. 配置凭据（`.env`）

复制 `.env.example` 为 `.env` 并填入真实值（`.env` 已在 `.gitignore`，切勿提交）：

```dotenv
BAOTA_PANEL_URL=http://<面板IP>:<端口>/<安全入口>
BAOTA_USERNAME=<面板账号>
BAOTA_PASSWORD=<面板密码>
BAOTA_SECRET_KEY=<至少32字节随机串，python -c "import secrets;print(secrets.token_urlsafe(32))">
BAOTA_STATE_FILE=./baota_state.enc      # 会话 cookie 加密落盘位置
BAOTA_DRY_RUN=true                       # true=写操作只打印不执行
BAOTA_CONFIRM_EXEC=true                  # true=baota_exec_shell 必须显式 confirm=true 才真执行
```

> 安全声明：服务仅监听 `127.0.0.1`，绝不暴露公网；凭据不硬编码，会话密钥加密保存于 `BAOTA_STATE_FILE`。

### 3. 启动服务

```bash
cd C:\Users\Administrator\WorkBuddy\xiaoyuangou\tools\baota_bridge
.venv\Scripts\python.exe mcp_server.py --host 127.0.0.1 --port 18799
```

- `GET  http://127.0.0.1:18799/` → 健康检查 `{"ok":true}`
- `POST http://127.0.0.1:18799/` → JSON-RPC 2.0：`initialize` / `tools/list` / `tools/call`
- 懒加载 `BaotaAPI`，启动**不要求**面板配置立即有效；首次调用工具时才真正建连。

### 4. 接入 Marguerite（`config.mcp.servers`）

编辑 `$1marguerite/config.yaml`，在 `mcp.servers` 增加一条带 `url` 的条目：

```yaml
mcp:
  servers:
    - name: "baota"
      url: "http://127.0.0.1:18799"
      # command / args / env 为子进程式可选字段，URL 模式留空即可
```

引擎 `Orchestrator::new` 注册完内置工具后，会遍历带 `url` 的条目，在独立 OS 线程 + `tokio` current-thread runtime 中 `connect()` 并 `list_tools()`；失败仅打印 `[mcp]` 告警、不阻断启动。连接成功后工具合并进 `describe_tools_for_llm()`（本地优先、同名冲突本地胜出），LLM 工具面与执行路由（`tool_cycle.rs` / `acp/session.rs`）均可调用。

### 5. 暴露的工具（19 个，前缀 `baota_`）

| 工具 | 能力 |
|------|------|
| `baota_system_info` | 系统服务概览 |
| `baota_network_info` | 网络流量信息 |
| `baota_exec_shell` | 面板终端执行 shell（尊重 `DRY_RUN`；非 dry-run 须 `confirm=true`） |
| `baota_list_sites` / `baota_get_site` / `baota_create_site` / `baota_set_site_status` | 站点管理（列/查/建/启停） |
| `baota_list_files` / `baota_read_file` / `baota_write_file` / `baota_upload_file` | 文件管理（列/读/写/上传） |
| `baota_list_databases` / `baota_create_database` | 数据库管理 |
| `baota_list_ftp` / `baota_create_ftp` | FTP 账号 |
| `baota_list_cron` / `baota_add_cron` | 计划任务（列/增） |
| `baota_list_ssl` | SSL 证书（best-effort，随面板版本） |
| `baota_get_logs` | 面板操作日志 |

### 6. 安全护栏

- `baota_exec_shell` 默认 `DRY_RUN` 仅返回 `{status:true,msg:"dry-run"}`；要真正执行必须 `confirm=true` 且 `BAOTA_DRY_RUN=false`。
- 未知工具返回 JSON-RPC `-32601` 错误；调用失败在 `content[].isError` 标记。

### 7. 验证

```bash
# 引擎侧
cargo test -p marguerite-core --lib            # 基线 1152 passed（含 MCP 工具面合并测试）
marguerite mcp list                            # 应列出 name=baota 的已配置服务器

# 服务侧（独立进程）
curl http://127.0.0.1:18799/                  # {"ok":true}
curl -X POST http://127.0.0.1:18799/ -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'   # 返回 19 个 baota_* 工具
```
