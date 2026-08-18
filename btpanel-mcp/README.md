# btpanel-mcp

宝塔面板（BT Panel）标准 MCP Server。将 WorkBuddy `baota_bridge` 桥接层已验证的面板内部 API 能力抽取封装为 MCP 工具集，**零第三方依赖**（仅 Python 3.12+ 标准库），支持 `stdio` 与 `sse` 双传输，MCP 协议自实现（协议版本 `2024-11-05`）。

## 特性

- **无需 Playwright**：凭据从环境变量 / 会话 JSON 文件直接导入（cookie 手动导入形态），登录工具可选。
- **Windows / Linux 面板自动适配**：token 探测自动切换 —— Windows 面板走请求头 `x-http-token`（正则 `window.vite_public_request_token`），Linux 面板走表单字段 `request_token` + `request_time`（正则 `request_token`）。
- **默认 dry-run**：所有写操作（建站/建库/建 FTP/写文件/上传/删除/exec）默认只模拟，需显式关闭。
- **exec 高危白名单**：仅放行 `df -h` / `free -m` / `uptime` / `pwd` / `ls <前缀>` 只读命令，且需 `BAOTA_CONFIRM_EXEC=true`。
- **SSE 仅监听 127.0.0.1**。

## 安装

```powershell
# 方式一：本地开发安装（可选，entry point 会注册 btpanel-mcp 命令）
cd d:\Downloads\Marguerite\marguerite\btpanel-mcp
py -3.14 -m pip install -e .

# 方式二：不安装，直接用模块路径启动
py -3.14 -m btpanel_mcp.cli stdio
```

## 配置（环境变量）

| 变量 | 必填 | 说明 |
| --- | --- | --- |
| `BAOTA_BASE_URL` | 调工具时必填 | 面板地址（含安全路径），如 `http://127.0.0.1:8888/C70AGNQm`，内部自动剥离安全路径 |
| `BAOTA_SESSION_FILE` | 二选一 | 会话 cookie JSON 文件路径（浏览器开发者工具导出形态） |
| `BAOTA_REQUEST_TOKEN` | 二选一 | 面板 token（Windows 面板风格，优先级高于 SESSION_FILE） |
| `BAOTA_DRY_RUN` | 否 | 默认 `true`：写操作只模拟不执行 |
| `BAOTA_CONFIRM_EXEC` | 否 | 默认 `true`：exec 工具需该开关且命令在白名单内才执行 |

会话 JSON 文件格式（从浏览器开发者工具 → Application → Cookies 复制关键字段）：

```json
{
  "session": "xxx",
  "request_token": "xxx",
  "request_time": "1700000000",
  "__request_token__": "xxx"
}
```

- Windows 面板：`__request_token__` 会写入请求头 `x-http-token`；
- Linux 面板：`request_token` / `request_time` 会作为表单字段提交；
- 其余键原样作为 Cookie 回传。

## 启动

```powershell
# stdio 传输（MCP 客户端默认）
$env:BAOTA_BASE_URL = "http://127.0.0.1:8888/C70AGNQm"
$env:BAOTA_SESSION_FILE = "C:\path\session.json"
py -3.14 -m btpanel_mcp.cli stdio

# HTTP+SSE 传输
py -3.14 -m btpanel_mcp.cli sse --host 127.0.0.1 --port 8765
# 端点：GET http://127.0.0.1:8765/sse 建流；POST http://127.0.0.1:8765/message?sessionId=xxx 收消息
```

## 工具清单

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `baota_health` | - | 检查配置与面板连通性；无凭据时返回配置状态而非报错 |
| `baota_sites_list` | `page?` `limit?` | 列出站点 |
| `baota_site_create` | `webname` `path` `php_version?` | 创建站点（webname 为域名，内部包 JSON domainlist） |
| `baota_site_status` | `site_name` `status(0\|1)` | 启停站点 |
| `baota_databases_list` | `page?` `limit?` | 列出数据库 |
| `baota_database_create` | `name` `password?` `db_user?` | 创建 MySQL 数据库（密码缺省自动随机） |
| `baota_ftps_list` | `page?` `limit?` | 列出 FTP 账号 |
| `baota_ftp_create` | `name` `password` `path` | 创建 FTP 账号 |
| `baota_files_list` | `path` | 列出远端目录 |
| `baota_file_read` | `path` | 读取远端文件 |
| `baota_file_write` | `path` `content` `encoding?` | 写入/覆盖远端文件 |
| `baota_file_delete` | `path` | 删除远端文件 |
| `baota_file_upload` | `local_path` `remote_dir` `chunk_size_mb?` | 分片上传（每片默认 2MB） |
| `baota_system_info` | - | 系统概览（服务状态） |
| `baota_network_info` | - | 网络流量信息 |
| `baota_exec` | `cmd` | 高危：仅只读白名单命令（见上文） |

所有工具缺凭据时返回 `isError: true` + 明确提示（要求配置 `BAOTA_BASE_URL` 与 session/token）。

## 未覆盖能力（声明为不支持）

SSL 证书、计划任务、防火墙、反向代理、站点日志、PHP 版本热切、数据库备份/删除、站点删除。

## 安全说明

1. 凭据只从环境变量 / 会话文件读取，代码中不硬编码任何凭据；`.env.example` 仅含占位符。
2. 会话文件在非 Windows 平台做权限检查（提示过宽）；Windows 下尽力而为（mode 位无意义）。
3. SSE 传输仅监听 `127.0.0.1`，请勿对外暴露。
4. `baota_exec` 默认拒绝：需 `BAOTA_CONFIRM_EXEC=true` 且命令命中只读白名单。
5. `BAOTA_DRY_RUN` 默认 `true`，所有写操作先模拟，确认无误后再关闭。

## 接入 opensquilla（opensquilla.toml）

```toml
[mcp]
enabled = true

[[mcp.servers]]
name = "btpanel"
transport = "stdio"
command = "py"
args = ["-3.14", "-m", "btpanel_mcp.cli", "stdio"]
env = { BAOTA_BASE_URL = "http://127.0.0.1:8888/C70AGNQm",
        BAOTA_SESSION_FILE = "C:/path/session.json",
        BAOTA_DRY_RUN = "true",
        BAOTA_CONFIRM_EXEC = "true" }
```

或 SSE 方式（先手动启动 server）：

```toml
[[mcp.servers]]
name = "btpanel"
transport = "sse"
url = "http://127.0.0.1:8765/sse"
env = { BAOTA_BASE_URL = "http://127.0.0.1:8888/C70AGNQm",
        BAOTA_SESSION_FILE = "C:/path/session.json" }
```

## 自测

```powershell
py -3.14 d:\Downloads\Marguerite\marguerite\btpanel-mcp\smoke_stdio.py
# 输出 PASS 即通过（无需真实面板凭据）
```
