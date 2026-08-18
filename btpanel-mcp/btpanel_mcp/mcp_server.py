"""
标准 MCP Server 实现（协议版本 2024-11-05，自实现、零第三方依赖）。

- stdio 传输：stdin/stdout 换行分隔的 JSON-RPC
- SSE 传输：基于 http.server 的 HTTP+SSE
  * GET  /sse                      建立 SSE 流，先发 ``event: endpoint``（data: /message?sessionId=xxx）
  * POST /message?sessionId=xxx    接收 JSON-RPC 消息，响应经 SSE 流回传（202 Accepted）
- 实现方法：initialize / notifications/initialized / ping / tools/list / tools/call
"""
from __future__ import annotations

import json
import queue
import sys
import threading
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Callable
from urllib.parse import parse_qs, urlparse

from .bt_api import BaotaAPI, BaotaConfig
from .code_check import check_code, format_report, skip_code_check

PROTOCOL_VERSION = "2024-11-05"
SERVER_NAME = "btpanel-mcp"
SERVER_VERSION = "0.1.0"

# --------------------------------------------------------------------------- #
# 工具定义
# --------------------------------------------------------------------------- #

# baota_exec 高危命令白名单（仅只读命令）
_EXEC_ALLOWED: tuple[str, ...] = ("df -h", "free -m", "uptime", "pwd")


def _is_exec_allowed(cmd: str) -> bool:
    """白名单校验：仅放行列出的只读命令与 ls 前缀。"""
    c = cmd.strip()
    if c in _EXEC_ALLOWED:
        return True
    return c == "ls" or c.startswith("ls ")


def _schema(required: list[str], props: dict[str, Any]) -> dict[str, Any]:
    """构造 JSON Schema（object 类型工具参数）。"""
    return {"type": "object", "properties": props, "required": required}


def _need_api(srv: "BtMCP") -> tuple[BaotaAPI, None] | tuple[None, str]:
    """取 API 实例；缺配置/缺凭据时返回 (None, 错误消息)。"""
    api = srv.api
    if api is None:
        return None, ("错误: 未配置面板地址，请用 baota_login 提供 base_url/账号密码，"
                      "或设置环境变量 BAOTA_BASE_URL（例如 http://127.0.0.1:8888/安全路径）。")
    if not api.has_credentials():
        return None, ("错误: 缺少面板凭据，请用 baota_login 输入面板地址、账号与密码登录，"
                      "或设置环境变量 BAOTA_REQUEST_TOKEN / BAOTA_SESSION_FILE。")
    return api, None


def _json_text(data: Any) -> str:
    return json.dumps(data, ensure_ascii=False, indent=2)


def _h_health(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    """检查配置与面板连通性；无凭据时仅返回配置状态（不报错）。"""
    cfg = srv.config
    api = srv.api
    detail = [
        f"base_url: {cfg.base_url or '(未配置)'}",
        f"dry_run: {cfg.dry_run}",
        f"confirm_exec: {cfg.confirm_exec}",
    ]
    if api is None:
        detail.append("credentials: (无 - 仅返回配置状态)")
        detail.append("connectivity: 未检查（未配置面板地址）")
    else:
        cred_src = []
        if cfg.request_token:
            cred_src.append("env token")
        if cfg.session_file:
            cred_src.append(f"session file: {cfg.session_file}")
        if not api.has_credentials():
            cred_src.append("(空)")
        detail.append("credentials: " + ", ".join(cred_src))
        ok = api.refresh_token()
        detail.append("connectivity: " + ("OK（已探测/刷新 token）" if ok or api.has_credentials()
                                          else "面板可达但未取到 token，需手动导入会话"))
    return _json_text({"config_ok": True, "note": "无凭据时仅返回配置状态", "detail": detail}), False


def _h_sites_list(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    api, err = _need_api(srv)
    if err:
        return err, True
    r = api.list_sites(page=int(args.get("page") or 1), limit=int(args.get("limit") or 15))
    return _json_text(r), not r.get("ok")


def _h_site_create(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    api, err = _need_api(srv)
    if err:
        return err, True
    webname = str(args.get("webname") or "").strip()
    path = str(args.get("path") or "").strip()
    if not webname or not path:
        return "错误: webname(域名) 与 path 为必填参数。", True
    php = str(args.get("php_version") or "82")
    r = api.create_site(webname, path, php)
    return _json_text(r), not r.get("ok")


def _h_site_status(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    api, err = _need_api(srv)
    if err:
        return err, True
    site_name = str(args.get("site_name") or "").strip()
    status = args.get("status")
    if not site_name or status not in (0, 1):
        return "错误: site_name 必填，status 必须为 0(停) 或 1(启)。", True
    r = api.set_site_status(site_name, int(status))
    return _json_text(r), not r.get("ok")


def _h_databases_list(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    api, err = _need_api(srv)
    if err:
        return err, True
    r = api.list_databases(page=int(args.get("page") or 1), limit=int(args.get("limit") or 15))
    return _json_text(r), not r.get("ok")


def _h_database_create(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    api, err = _need_api(srv)
    if err:
        return err, True
    name = str(args.get("name") or "").strip()
    if not name:
        return "错误: name(数据库名) 为必填参数。", True
    password = str(args["password"]) if args.get("password") else None
    db_user = str(args["db_user"]) if args.get("db_user") else None
    r = api.create_database(name, password, db_user)
    return _json_text(r), not r.get("ok")


def _h_ftps_list(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    api, err = _need_api(srv)
    if err:
        return err, True
    r = api.list_ftps(page=int(args.get("page") or 1), limit=int(args.get("limit") or 15))
    return _json_text(r), not r.get("ok")


def _h_ftp_create(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    api, err = _need_api(srv)
    if err:
        return err, True
    name = str(args.get("name") or "").strip()
    password = str(args.get("password") or "").strip()
    path = str(args.get("path") or "").strip()
    if not name or not password or not path:
        return "错误: name、password、path 均为必填参数。", True
    r = api.create_ftp(name, password, path)
    return _json_text(r), not r.get("ok")


def _h_files_list(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    api, err = _need_api(srv)
    if err:
        return err, True
    path = str(args.get("path") or "").strip()
    if not path:
        return "错误: path 为必填参数。", True
    r = api.list_dir(path)
    return _json_text(r), not r.get("ok")


def _h_file_read(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    api, err = _need_api(srv)
    if err:
        return err, True
    path = str(args.get("path") or "").strip()
    if not path:
        return "错误: path 为必填参数。", True
    r = api.read_file(path)
    return _json_text(r), not r.get("ok")


def _h_file_write(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    api, err = _need_api(srv)
    if err:
        return err, True
    path = str(args.get("path") or "").strip()
    content = args.get("content")
    if not path or content is None:
        return "错误: path 与 content 为必填参数。", True
    if not skip_code_check():
        result = check_code(path, str(content))
        if not result.ok:
            return (format_report(result, path)
                    + "\n\n已拒绝写入（可设 BAOTA_SKIP_CODE_CHECK=1 强制跳过）。"), True
    encoding = str(args.get("encoding") or "utf-8")
    r = api.write_file(path, str(content), encoding)
    return _json_text(r), not r.get("ok")


def _h_file_delete(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    api, err = _need_api(srv)
    if err:
        return err, True
    path = str(args.get("path") or "").strip()
    if not path:
        return "错误: path 为必填参数。", True
    r = api.delete_file(path)
    return _json_text(r), not r.get("ok")


def _h_file_upload(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    api, err = _need_api(srv)
    if err:
        return err, True
    local = str(args.get("local_path") or "").strip()
    remote = str(args.get("remote_dir") or "").strip()
    if not local or not remote:
        return "错误: local_path 与 remote_dir 为必填参数。", True
    try:
        chunk = int(float(args.get("chunk_size_mb") or 2) * 1024 * 1024)
    except (TypeError, ValueError):
        return "错误: chunk_size_mb 必须是数字（单位 MB）。", True
    if not skip_code_check():
        try:
            local_bytes = Path(local).read_bytes()
        except OSError as e:
            return f"错误: 无法读取本地文件 {local}: {e}", True
        try:
            local_text = local_bytes.decode("utf-8")
        except UnicodeDecodeError:
            local_text = None  # 二进制文件不做语法校验
        if local_text is not None:
            result = check_code(local, local_text)
            if not result.ok:
                return (format_report(result, local)
                        + "\n\n已拒绝上传（可设 BAOTA_SKIP_CODE_CHECK=1 强制跳过）。"), True
    r = api.upload_file(local, remote, chunk)
    return _json_text(r), not r.get("ok")


def _h_system_info(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    api, err = _need_api(srv)
    if err:
        return err, True
    r = api.system_info()
    return _json_text(r), not r.get("ok")


def _h_network_info(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    api, err = _need_api(srv)
    if err:
        return err, True
    r = api.network_info()
    return _json_text(r), not r.get("ok")


def _h_exec(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    """高危工具：默认拒绝，需 BAOTA_CONFIRM_EXEC=true 且命令在只读白名单内。"""
    api, err = _need_api(srv)
    if err:
        return err, True
    cmd = str(args.get("cmd") or "").strip()
    if not cmd:
        return "错误: cmd 为必填参数。", True
    if not srv.config.confirm_exec:
        return ("错误: 高危操作被拒绝。执行命令需要 BAOTA_CONFIRM_EXEC=true"
                "（当前被关闭），已拒绝执行。"), True
    if not _is_exec_allowed(cmd):
        return ("错误: 高危操作被拒绝。仅允许只读白名单命令: "
                f"df -h / free -m / uptime / pwd / ls <前缀>。收到: {cmd!r}"), True
    r = api.exec_shell(cmd)
    return _json_text(r), not r.get("ok")


def _h_login(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    """面板账号密码登录：验证地址连通后获取会话并持久化，密码不回显。"""
    base_url = str(args.get("base_url") or "").strip() or srv.config.base_url
    if not base_url:
        return ("错误: 请提供面板地址 base_url（例如 http://127.0.0.1:8888/安全路径），"
                "或先在环境变量 BAOTA_BASE_URL 配置。"), True
    username = str(args.get("username") or "").strip()
    password = str(args.get("password") or "")
    if not username or not password:
        return "错误: username 与 password 为必填参数。", True
    # base_url 变化时更新配置并重建客户端
    if base_url != srv.config.base_url:
        srv.config.base_url = base_url
        srv._api = None
    api = srv.api
    if api is None:
        return f"错误: 无法初始化面板客户端（base_url={base_url}）。", True
    r = api.login(username, password)
    if r.get("ok"):
        detail = api.credentials_status()
        detail["password_echo"] = "（已保存，不回显）"
        return _json_text({"ok": True, "msg": r.get("msg"), "credentials": detail}), False
    return _json_text({"ok": False, "msg": r.get("msg"), "data": r.get("data")}), True


def _h_credentials(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    """查看当前面板凭据状态（不含密码）。"""
    if srv.api is None:
        path = srv.config.resolved_credentials_path()
        import os
        return _json_text({
            "ok": True,
            "credentials": {
                "base_url": srv.config.base_url or "(未配置)",
                "source": "none",
                "mode": "none",
                "has_username": False,
                "credentials_file": path,
                "credentials_file_exists": bool(path and os.path.exists(path)),
            },
        }), False
    return _json_text({"ok": True, "credentials": srv.api.credentials_status()}), False


def _h_logout(srv: "BtMCP", args: dict[str, Any]) -> tuple[str, bool]:
    """清除已保存的面板凭据（登出）。"""
    if srv.api is None:
        return _json_text({"ok": True, "msg": "当前无面板凭据"}), False
    ok = srv.api.clear_credentials()
    return _json_text({
        "ok": ok,
        "msg": "已清除面板凭据（会话与账号密码）" if ok else "清除凭据文件失败",
    }), not ok


class _Tool:
    """单个 MCP 工具：名称 + 描述 + JSON Schema + 处理函数。"""

    def __init__(self, name: str, description: str, schema: dict[str, Any],
                 handler: Callable[["BtMCP", dict[str, Any]], tuple[str, bool]]):
        self.name = name
        self.description = description
        self.schema = schema
        self.handler = handler

    def to_dict(self) -> dict[str, Any]:
        return {"name": self.name, "description": self.description, "inputSchema": self.schema}


_TOOLS: list[_Tool] = [
    _Tool("baota_health", "检查配置与面板连通性；无凭据时返回配置状态而非报错",
          _schema([], {}), _h_health),
    _Tool("baota_sites_list", "列出面板站点",
          _schema([], {"page": {"type": "integer", "description": "页码，默认 1"},
                       "limit": {"type": "integer", "description": "每页数量，默认 15"}}), _h_sites_list),
    _Tool("baota_site_create", "创建站点（webname 为域名，内部包一层 domainlist）",
          _schema(["webname", "path"],
                  {"webname": {"type": "string", "description": "域名，如 example.com"},
                   "path": {"type": "string", "description": "站点根目录，如 /www/wwwroot/example.com"},
                   "php_version": {"type": "string", "description": "PHP 版本，如 82，默认 82"}}), _h_site_create),
    _Tool("baota_site_status", "启停站点",
          _schema(["site_name", "status"],
                  {"site_name": {"type": "string", "description": "站点名/域名"},
                   "status": {"type": "integer", "enum": [0, 1], "description": "0=停用, 1=启用"}}), _h_site_status),
    _Tool("baota_databases_list", "列出面板数据库",
          _schema([], {"page": {"type": "integer", "description": "页码，默认 1"},
                       "limit": {"type": "integer", "description": "每页数量，默认 15"}}), _h_databases_list),
    _Tool("baota_database_create", "创建 MySQL 数据库（未提供密码时自动随机生成）",
          _schema(["name"],
                  {"name": {"type": "string", "description": "数据库名"},
                   "password": {"type": "string", "description": "数据库密码，可选"},
                   "db_user": {"type": "string", "description": "数据库用户名，默认同 name"}}), _h_database_create),
    _Tool("baota_ftps_list", "列出面板 FTP 账号",
          _schema([], {"page": {"type": "integer", "description": "页码，默认 1"},
                       "limit": {"type": "integer", "description": "每页数量，默认 15"}}), _h_ftps_list),
    _Tool("baota_ftp_create", "创建 FTP 账号",
          _schema(["name", "password", "path"],
                  {"name": {"type": "string", "description": "FTP 用户名"},
                   "password": {"type": "string", "description": "FTP 密码"},
                   "path": {"type": "string", "description": "FTP 根目录"}}), _h_ftp_create),
    _Tool("baota_files_list", "列出远端目录内容",
          _schema(["path"], {"path": {"type": "string", "description": "远端绝对路径，如 /www/wwwroot"}}), _h_files_list),
    _Tool("baota_file_read", "读取远端文件内容",
          _schema(["path"], {"path": {"type": "string", "description": "远端文件绝对路径"}}), _h_file_read),
    _Tool("baota_file_write", "写入/覆盖远端文件（写入前自动做语法与风险校验，语法错误或高危模式将拒绝）",
          _schema(["path", "content"],
                  {"path": {"type": "string", "description": "远端文件绝对路径"},
                   "content": {"type": "string", "description": "文件内容"},
                   "encoding": {"type": "string", "description": "编码，默认 utf-8"}}), _h_file_write),
    _Tool("baota_file_delete", "删除远端文件",
          _schema(["path"], {"path": {"type": "string", "description": "远端文件绝对路径"}}), _h_file_delete),
    _Tool("baota_file_upload", "分片上传本地文件到面板目录（每片默认 2MB；文本文件上传前自动做语法与风险校验）",
          _schema(["local_path", "remote_dir"],
                  {"local_path": {"type": "string", "description": "本地文件绝对路径"},
                   "remote_dir": {"type": "string", "description": "远端目标目录"},
                   "chunk_size_mb": {"type": "number", "description": "分片大小 MB，默认 2"}}), _h_file_upload),
    _Tool("baota_system_info", "获取面板系统概览（服务状态）",
          _schema([], {}), _h_system_info),
    _Tool("baota_network_info", "获取面板网络流量信息",
          _schema([], {}), _h_network_info),
    _Tool("baota_exec", "高危：在面板终端执行只读白名单命令（df -h / free -m / uptime / pwd / ls 前缀）",
          _schema(["cmd"], {"cmd": {"type": "string", "description": "只读命令，如 df -h"}}), _h_exec),
    _Tool("baota_login", "使用面板账号密码登录（地址/账号/密码），成功后会话持久化，后续工具自动使用",
          _schema(["username", "password"],
                  {"base_url": {"type": "string", "description": "面板地址（含安全路径），如 http://127.0.0.1:8888/C70AGNQm；缺省用已配置地址"},
                   "username": {"type": "string", "description": "面板登录账号"},
                   "password": {"type": "string", "description": "面板登录密码（不回显）"}}), _h_login),
    _Tool("baota_credentials", "查看当前面板凭据状态（地址/来源/模式，不含密码）",
          _schema([], {}), _h_credentials),
    _Tool("baota_logout", "清除已保存的面板凭据（会话与账号密码）",
          _schema([], {}), _h_logout),
]

_TOOL_MAP: dict[str, _Tool] = {t.name: t for t in _TOOLS}

# --------------------------------------------------------------------------- #
# MCP 核心（JSON-RPC 消息处理，stdio / SSE 共用）
# --------------------------------------------------------------------------- #


class BtMCP:
    """MCP Server 实例：持有配置、懒加载 BaotaAPI、分发 JSON-RPC 消息。"""

    def __init__(self, config: BaotaConfig):
        self.config = config
        self._api: BaotaAPI | None = None

    @property
    def api(self) -> BaotaAPI | None:
        """懒创建 API 实例；未配置面板地址时返回 None。"""
        if self._api is None and self.config.base_url:
            self._api = BaotaAPI(self.config)
        return self._api

    # ------------------------------------------------------------------ #
    # JSON-RPC 分发
    # ------------------------------------------------------------------ #
    def handle_message(self, msg: Any) -> dict[str, Any] | None:
        """处理一条 JSON-RPC 消息；通知返回 None（不回复）。"""
        if not isinstance(msg, dict) or msg.get("jsonrpc") != "2.0":
            if isinstance(msg, dict) and "id" in msg:
                return self._error(msg, -32600, "Invalid Request")
            return None
        method = msg.get("method")
        if method == "initialize":
            result = {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {"listChanged": False}},
                "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
            }
            return self._result(msg, result)
        if method == "notifications/initialized":
            return None
        if method == "ping":
            return self._result(msg, {})
        if method == "tools/list":
            return self._result(msg, {"tools": [t.to_dict() for t in _TOOLS]})
        if method == "tools/call":
            return self._call_tool(msg)
        return self._error(msg, -32601, f"Method not found: {method}")

    def _call_tool(self, msg: dict[str, Any]) -> dict[str, Any]:
        params = msg.get("params") or {}
        name = params.get("name")
        arguments = params.get("arguments")
        tool = _TOOL_MAP.get(name)
        if tool is None:
            return self._error(msg, -32602, f"Unknown tool: {name}")
        if not isinstance(arguments, dict):
            arguments = {}
        try:
            text, is_error = tool.handler(self, arguments)
        except Exception as e:  # 兜底：工具内部意外异常
            text, is_error = f"工具执行异常: {e}", True
        return self._result(msg, {"content": [{"type": "text", "text": text}], "isError": is_error})

    @staticmethod
    def _result(msg: dict[str, Any], result: Any) -> dict[str, Any]:
        return {"jsonrpc": "2.0", "id": msg.get("id"), "result": result}

    @staticmethod
    def _error(msg: dict[str, Any], code: int, message: str) -> dict[str, Any]:
        return {"jsonrpc": "2.0", "id": msg.get("id"), "error": {"code": code, "message": message}}

    # ------------------------------------------------------------------ #
    # stdio 传输
    # ------------------------------------------------------------------ #
    def run_stdio(self) -> None:
        """逐行读取 stdin，处理后将响应以换行分隔 JSON 写回 stdout（禁止其他 stdout 输出）。"""
        stdin, stdout = sys.stdin.buffer, sys.stdout.buffer
        while True:
            line = stdin.readline()
            if not line:
                break
            line = line.strip()
            if not line:
                continue
            try:
                msg = json.loads(line.decode("utf-8"))
            except (json.JSONDecodeError, UnicodeDecodeError):
                continue
            resp = self.handle_message(msg)
            if resp is not None:
                stdout.write((json.dumps(resp, ensure_ascii=False) + "\n").encode("utf-8"))
                stdout.flush()

    # ------------------------------------------------------------------ #
    # SSE 传输
    # ------------------------------------------------------------------ #
    def run_sse(self, host: str = "127.0.0.1", port: int = 8765) -> None:
        """HTTP+SSE 传输（仅监听 127.0.0.1，请勿对外开放）。"""
        httpd = _SSEHttpServer((host, port), _SSEHandler, mcp=self)
        print(f"[btpanel-mcp] SSE 服务已启动: http://{host}:{port}/sse", file=sys.stderr)
        try:
            httpd.serve_forever()
        except KeyboardInterrupt:
            pass


class _SSEHttpServer(ThreadingHTTPServer):
    """携带 MCP 实例的线程化 HTTP 服务器。"""

    daemon_threads = True

    def __init__(self, addr, handler_class, mcp: BtMCP):
        self.mcp = mcp
        self._sessions: dict[str, queue.Queue] = {}
        self._lock = threading.Lock()
        super().__init__(addr, handler_class)

    def create_session(self) -> tuple[str, queue.Queue]:
        """新建 SSE 会话（endpoint 事件由此流回传）。"""
        sid = uuid.uuid4().hex
        q: queue.Queue = queue.Queue()
        with self._lock:
            self._sessions[sid] = q
        return sid, q

    def remove_session(self, sid: str) -> None:
        with self._lock:
            self._sessions.pop(sid, None)

    def post_message(self, sid: str, raw: bytes) -> int:
        """接收 POST 的消息：处理后将响应放入对应会话队列。"""
        with self._lock:
            q = self._sessions.get(sid)
        if q is None:
            return 404
        try:
            msg = json.loads(raw.decode("utf-8"))
        except (json.JSONDecodeError, UnicodeDecodeError):
            return 400
        resp = self.mcp.handle_message(msg)
        if resp is not None:
            q.put(resp)
        return 202


class _SSEHandler(BaseHTTPRequestHandler):
    """GET /sse 建流；POST /message?sessionId=xxx 收消息。"""

    server: _SSEHttpServer

    def log_message(self, fmt, *args):  # 静默默认访问日志
        pass

    def do_GET(self):
        parsed = urlparse(self.path)
        if parsed.path != "/sse":
            self.send_error(404, "Not Found")
            return
        sid, q = self.server.create_session()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream; charset=utf-8")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "keep-alive")
        self.end_headers()
        try:
            self.wfile.write(f"event: endpoint\ndata: /message?sessionId={sid}\n\n".encode("utf-8"))
            self.wfile.flush()
        except OSError:
            self.server.remove_session(sid)
            return
        while True:
            resp = q.get()
            if resp is None:
                break
            try:
                payload = f"event: message\ndata: {json.dumps(resp, ensure_ascii=False)}\n\n"
                self.wfile.write(payload.encode("utf-8"))
                self.wfile.flush()
            except OSError:  # 客户端断开
                break
        self.server.remove_session(sid)

    def do_POST(self):
        parsed = urlparse(self.path)
        if parsed.path != "/message":
            self.send_error(404, "Not Found")
            return
        sid = (parse_qs(parsed.query).get("sessionId") or [""])[0]
        if not sid:
            self.send_error(400, "Missing sessionId")
            return
        try:
            length = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            length = 0
        raw = self.rfile.read(length) if length > 0 else b""
        status = self.server.post_message(sid, raw)
        if status == 202:
            self.send_response(202)
            self.send_header("Content-Length", "0")
            self.end_headers()
        else:
            self.send_error(status)
