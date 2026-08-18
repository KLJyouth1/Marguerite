"""
宝塔面板(BT Panel) HTTP API 封装。

- 零第三方依赖：基于 urllib.request 自实现 HTTP 会话（不用 requests）
- token 探测/刷新：访问面板首页，正则双匹配 Windows / Linux 面板并自动切换
  * Windows 面板：请求头 ``x-http-token`` 校验，正则 ``window.vite_public_request_token``
  * Linux 面板：表单字段 ``request_token`` + ``request_time``，正则 ``request_token``
- 响应统一归一化：``{"ok": bool, "msg": str, "data": Any}``
- 写操作支持 dry-run 分支（默认开启，只模拟不执行）
"""
from __future__ import annotations

import json
import os
import re
import secrets
import string
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import urljoin, urlparse

# Windows 面板 token（首页 JS 变量）
RE_WIN_TOKEN = re.compile(r"window\.vite_public_request_token\s*=\s*['\"]([^'\"]+)['\"]")
# Linux 面板 token（首页 JS 变量）
RE_LINUX_TOKEN = re.compile(r"request_token\s*=\s*['\"]([^'\"]+)['\"]")

UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")

# Linux 面板请求中不放进 Cookie 头、仅作为表单字段的键
_FORM_ONLY_COOKIE_KEYS = ("__request_token__", "request_token", "request_time")


@dataclass
class BaotaConfig:
    """MCP Server 运行配置（环境变量优先，凭据文件兜底）。"""

    base_url: str = ""
    session_file: str | None = None
    request_token: str | None = None
    dry_run: bool = True
    confirm_exec: bool = True
    credentials_file: str | None = None

    def resolved_credentials_path(self) -> str | None:
        """凭据文件路径：显式配置优先，否则默认 ~/.marguerite/btpanel-credentials.json。"""
        if self.credentials_file:
            return self.credentials_file
        try:
            home = Path.home()
        except (OSError, RuntimeError):
            return None
        if not home:
            return None
        return str(home / ".marguerite" / "btpanel-credentials.json")


def parse_panel_url(url: str) -> tuple[str, str]:
    """解析面板地址，返回 (panel_url 含安全路径, api_base_url 剥离安全路径)。"""
    url = (url or "").strip().rstrip("/")
    if not url:
        return "", ""
    parsed = urlparse(url)
    return url, f"{parsed.scheme}://{parsed.netloc}/"


def _random_password(length: int = 16) -> str:
    """生成随机密码（建库/建 FTP 未提供密码时使用）。"""
    chars = string.ascii_letters + string.digits + "!@#$%"
    return "".join(secrets.choice(chars) for _ in range(length))


class BaotaAPI:
    """宝塔面板操作入口（Windows / Linux 面板 token 方式自动切换）。"""

    def __init__(self, config: BaotaConfig):
        self.config = config
        self.panel_url, self.api_base = parse_panel_url(config.base_url)
        self.dry_run = config.dry_run
        self.confirm_exec = config.confirm_exec

        self._cookies: dict[str, str] = {}
        self._win_token: str = ""      # Windows 面板：x-http-token 请求头
        self._linux_token: str = ""    # Linux 面板：request_token 表单字段
        self._linux_time: str = ""
        self._username: str = ""       # 账号密码登录（凭据文件），用于自动重登
        self._password: str = ""

        # 1) 环境变量直接指定的 token（Windows 面板风格，优先级最高）
        if config.request_token:
            self._win_token = config.request_token.strip()
        # 2) 会话 JSON 文件导入 cookie（浏览器开发者工具复制形态）
        if config.session_file:
            self._load_session_file()
        # 3) 凭据文件兜底：恢复已登录会话；仅有账号密码则留待自动登录
        if not (self._win_token or self._linux_token or self._cookies):
            self._load_credentials_file()

    # ------------------------------------------------------------------ #
    # 凭据文件（账号密码登录态持久化，WebUI「配置凭据」落盘目标）
    # ------------------------------------------------------------------ #
    def _credentials_path(self) -> Path | None:
        raw = self.config.resolved_credentials_path()
        return Path(raw) if raw else None

    def _load_credentials_file(self) -> None:
        path = self._credentials_path()
        if path is None or not path.exists():
            return
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            return
        if not isinstance(data, dict):
            return
        if data.get("base_url") and not self.panel_url:
            self.panel_url, self.api_base = parse_panel_url(str(data["base_url"]))
        cookies = data.get("cookies")
        if isinstance(cookies, dict):
            self._cookies = {str(k): str(v) for k, v in cookies.items()}
        if data.get("win_token"):
            self._win_token = str(data["win_token"])
        if data.get("linux_token"):
            self._linux_token = str(data["linux_token"])
            self._linux_time = str(data.get("linux_time") or int(time.time()))
        self._username = str(data.get("username") or "")
        self._password = str(data.get("password") or "")

    def save_credentials(self, username: str | None = None, password: str | None = None) -> bool:
        """把当前登录态与账号密码写入凭据文件（0600 尽力）。"""
        path = self._credentials_path()
        if path is None:
            return False
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            data = {
                "base_url": self.panel_url,
                "cookies": self._cookies,
                "win_token": self._win_token,
                "linux_token": self._linux_token,
                "linux_time": self._linux_time,
            }
            if username is not None:
                data["username"] = username
            if password is not None:
                data["password"] = password
            path.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")
            if os.name != "nt":
                try:
                    path.chmod(0o600)
                except OSError:
                    pass
            return True
        except OSError:
            return False

    def clear_credentials(self) -> bool:
        """删除凭据文件（登出）。"""
        path = self._credentials_path()
        if path is None:
            return False
        try:
            if path.exists():
                path.unlink()
            self._cookies = {}
            self._win_token = ""
            self._linux_token = ""
            self._linux_time = ""
            return True
        except OSError:
            return False

    # ------------------------------------------------------------------ #
    # 账号密码登录
    # ------------------------------------------------------------------ #
    def login(self, username: str, password: str) -> dict:
        """使用面板账号密码登录：POST /login，提取 session cookie 与 token。

        - Linux 面板：登录请求需携带首页探测到的 request_token 表单字段；
          成功后 Set-Cookie: session=xxx，后续请求以 Cookie 回传。
        - Windows 面板：登录成功后响应 data 可能携带 request_token（x-http-token）。
        """
        if not self.panel_url:
            return {"ok": False, "msg": "未配置面板地址（base_url / BAOTA_BASE_URL）", "data": None}
        username = (username or "").strip()
        if not username or not password:
            return {"ok": False, "msg": "账号与密码不能为空", "data": None}
        self.refresh_token()  # 先探测首页 token（两种面板均需）
        payload = {"username": username, "password": password}
        payload.update(self._auth_fields())
        url = urljoin(self.api_base, "login")
        headers = {
            "User-Agent": UA,
            "Referer": self.panel_url + "/",
            "X-Requested-With": "XMLHttpRequest",
            "Accept": "application/json, text/javascript, */*; q=0.01",
            "Content-Type": "application/x-www-form-urlencoded",
        }
        body = urllib.parse.urlencode(payload).encode("utf-8")
        req = urllib.request.Request(url, data=body, headers=headers, method="POST")
        raw = b""
        status = 0
        set_cookies: list[str] = []
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                raw = resp.read()
                status = resp.status
                set_cookies = resp.get_all("Set-Cookie") or []
        except urllib.error.HTTPError as e:
            raw = e.read() if e.fp else b""
            status = e.code
            set_cookies = e.headers.get_all("Set-Cookie") or [] if e.headers else []
        except Exception as e:
            return {"ok": False, "msg": f"登录请求异常: {e}", "data": None}

        for sc in set_cookies:
            name, _, rest = sc.partition("=")
            value = rest.split(";")[0].strip()
            if name.strip().lower() == "session":
                self._cookies["session"] = value

        normalized = self._normalize(raw, url, status)
        # Windows 面板：响应 data 内可能携带 token；仅登录响应显式返回的 token 才算认证
        new_token_from_login = False
        dt = normalized.get("data")
        if isinstance(dt, dict):
            tok = dt.get("request_token") or dt.get("token") or dt.get("x_http_token")
            if tok:
                self._win_token = str(tok)
                new_token_from_login = True

        # 认证判据：Linux/通用面板以 session cookie 为准；Windows 以登录响应 token 为准。
        # 首页探测到的匿名 token（refresh_token 所得）不足以判定登录成功。
        authenticated = bool(self._cookies.get("session")) or new_token_from_login
        if authenticated:
            self._username = username
            self._password = password
            self.save_credentials(username, password)
            mode = "win" if self._win_token else ("linux" if self._linux_token else "cookie")
            return {"ok": True, "msg": "登录成功", "data": {
                "base_url": self.panel_url, "mode": mode,
                "cookie_session": bool(self._cookies.get("session")),
            }}
        msg = normalized.get("msg") or "登录失败（未获得面板会话，请检查地址/账号/密码）"
        return {"ok": False, "msg": msg, "data": normalized.get("data")}

    # ------------------------------------------------------------------ #
    # 会话
    # ------------------------------------------------------------------ #
    def _load_session_file(self) -> None:
        """从 JSON 会话文件导入 cookie（含 Windows 的 __request_token__ 与 Linux 的 request_token）。"""
        path = Path(self.config.session_file or "")
        if not path.exists():
            print(f"[warn] 会话文件不存在: {path}", file=sys.stderr)
            return
        # 权限检查（Windows 下 mode 位无意义，尽力而为）
        try:
            if os.name != "nt":
                mode = path.stat().st_mode & 0o777
                if mode & 0o077:
                    print(f"[warn] 会话文件权限过宽(建议 0600): {path} (mode={oct(mode)})", file=sys.stderr)
        except OSError:
            pass
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as e:
            print(f"[warn] 会话文件解析失败: {path} ({e})", file=sys.stderr)
            return
        if not isinstance(data, dict):
            print(f"[warn] 会话文件格式错误（应为 JSON 对象）: {path}", file=sys.stderr)
            return
        self._cookies = {str(k): str(v) for k, v in data.items()}
        if self._cookies.get("__request_token__"):
            self._win_token = self._cookies["__request_token__"]
        if self._cookies.get("request_token"):
            self._linux_token = self._cookies["request_token"]
            self._linux_time = self._cookies.get("request_time", str(int(time.time())))

    def has_credentials(self) -> bool:
        """是否已具备可用凭据（token、cookie、或账号密码任一存在）。"""
        return bool(self._win_token or self._linux_token or self._cookies
                    or (self._username and self._password))

    def credentials_status(self) -> dict:
        """凭据状态摘要（供 baota_credentials 工具与 UI 展示，不含密码）。"""
        path = self._credentials_path()
        return {
            "base_url": self.panel_url or "(未配置)",
            "source": ("env-token" if self.config.request_token else
                       "session-file" if self.config.session_file else
                       "credentials-file" if path and path.exists() else "none"),
            "mode": ("win" if self._win_token else
                     "linux" if self._linux_token else
                     "cookie" if self._cookies else "account-only"),
            "cookie_session": bool(self._cookies.get("session")),
            "has_username": bool(self._username),
            "credentials_file": str(path) if path else None,
            "credentials_file_exists": bool(path and path.exists()),
        }

    # ------------------------------------------------------------------ #
    # 请求头 / token
    # ------------------------------------------------------------------ #
    def _headers(self) -> dict[str, str]:
        headers = {
            "User-Agent": UA,
            "Referer": self.api_base + "home",
            "X-Requested-With": "XMLHttpRequest",
            "Accept": "application/json, text/javascript, */*; q=0.01",
        }
        # Windows 面板：token 走请求头
        if self._win_token:
            headers["x-http-token"] = self._win_token
        # 其余 cookie 原样回传
        if self._cookies:
            pairs = [f"{k}={v}" for k, v in self._cookies.items() if k not in _FORM_ONLY_COOKIE_KEYS]
            if pairs:
                headers["Cookie"] = "; ".join(pairs)
        return headers

    def refresh_token(self) -> bool:
        """访问面板首页，正则双匹配刷新 Windows / Linux token。"""
        if not self.panel_url:
            return False
        try:
            req = urllib.request.Request(self.panel_url, headers={"User-Agent": UA})
            with urllib.request.urlopen(req, timeout=20) as resp:
                html = resp.read().decode("utf-8", "ignore")
        except Exception as e:
            print(f"[warn] 刷新 token 失败: {e}", file=sys.stderr)
            return False
        found = False
        m = RE_WIN_TOKEN.search(html)
        if m:
            self._win_token = m.group(1)
            found = True
        m2 = RE_LINUX_TOKEN.search(html)
        if m2:
            self._linux_token = m2.group(1)
            self._linux_time = str(int(time.time()))
            found = True
        return found

    def _ensure_token(self) -> None:
        """首次请求前：有 token/cookie 直接可用；仅有账号密码则自动登录。"""
        if self._win_token or self._linux_token or self._cookies:
            return
        if self._username and self._password:
            self.login(self._username, self._password)
            return
        self.refresh_token()

    def _auth_fields(self) -> dict[str, str]:
        """Linux 面板需追加到表单的 token 字段；Windows 面板走请求头，返回空。"""
        if self._win_token:
            return {}
        if self._linux_token:
            return {"request_token": self._linux_token, "request_time": self._linux_time}
        return {}

    # ------------------------------------------------------------------ #
    # 底层请求
    # ------------------------------------------------------------------ #
    def _request(self, endpoint: str, data: dict | None = None,
                 files: dict[str, tuple[str, bytes]] | None = None,
                 timeout: int = 60) -> dict:
        """POST 面板内部接口，返回归一化 {ok, msg, data}。"""
        url = urljoin(self.api_base, endpoint.lstrip("/"))
        self._ensure_token()
        headers = self._headers()
        payload: dict[str, str] = {str(k): str(v) for k, v in (data or {}).items()}
        payload.update(self._auth_fields())

        if files:
            body, content_type = self._multipart(payload, files)
            headers["Content-Type"] = content_type
        else:
            body = urllib.parse.urlencode(payload).encode("utf-8")
            headers["Content-Type"] = "application/x-www-form-urlencoded"

        req = urllib.request.Request(url, data=body, headers=headers, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return self._normalize(resp.read(), url, resp.status)
        except urllib.error.HTTPError as e:
            raw = e.read() if e.fp else b""
            return self._normalize(raw, url, e.code)
        except Exception as e:
            return {"ok": False, "msg": f"请求异常: {e}", "data": None}

    @staticmethod
    def _normalize(raw: bytes, url: str, http_code: int) -> dict:
        """把面板原始响应归一化为 {ok, msg, data}。"""
        text = raw.decode("utf-8", "ignore").strip()
        if not text:
            return {"ok": False, "msg": "空响应", "data": None}
        try:
            j = json.loads(text)
        except json.JSONDecodeError:
            return {"ok": False, "msg": "非JSON响应", "data": None,
                    "http_code": http_code, "url": url, "body": text[:800]}
        if not isinstance(j, dict):
            return {"ok": False, "msg": "响应不是 JSON 对象", "data": j}
        status = j.get("status")
        if status is None:
            ok = True  # 部分端点（如 get_dir_list）不返回 status
        else:
            ok = status in (True, 1, 0, "0", "true")
        return {"ok": ok, "msg": j.get("msg", ""), "data": j.get("data", j)}

    @staticmethod
    def _multipart(fields: dict[str, str], files: dict[str, tuple[str, bytes]]) -> tuple[bytes, str]:
        """构造 multipart/form-data 请求体；files: {字段名: (文件名, 字节)}。"""
        boundary = "----btmcp" + uuid.uuid4().hex
        parts: list[bytes] = []
        for k, v in fields.items():
            parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'.encode("utf-8"))
        for k, (fname, blob) in files.items():
            parts.append(
                f'--{boundary}\r\nContent-Disposition: form-data; name="{k}"; filename="{fname}"\r\n'
                "Content-Type: application/octet-stream\r\n\r\n".encode("utf-8")
            )
            parts.append(blob)
            parts.append(b"\r\n")
        parts.append(f"--{boundary}--\r\n".encode("utf-8"))
        return b"".join(parts), f"multipart/form-data; boundary={boundary}"

    # ------------------------------------------------------------------ #
    # 系统 / 终端
    # ------------------------------------------------------------------ #
    def exec_shell(self, cmd: str) -> dict:
        """在面板终端执行 shell 命令（调用方须先完成白名单校验）。"""
        if self.dry_run:
            return {"ok": True, "msg": "dry-run: 已模拟执行，未真正运行", "data": {"cmd": cmd}}
        return self._request("/system?action=ExecShell", {"shell": cmd})

    def system_info(self) -> dict:
        """系统概览（服务状态）。"""
        return self._request("/system/GetServiceStatus")

    def network_info(self) -> dict:
        """网络流量信息。"""
        return self._request("/system/GetNetWork")

    # ------------------------------------------------------------------ #
    # 站点管理
    # ------------------------------------------------------------------ #
    def list_sites(self, page: int = 1, limit: int = 15) -> dict:
        return self._request("/site/get_site_list", {"page": page, "limit": limit})

    def site_info(self, site_name: str) -> dict:
        return self._request("/site?action=GetSiteInfo", {"siteName": site_name})

    def create_site(self, webname: str, path: str, php_version: str = "82") -> dict:
        """创建站点（webname 为域名，内部包一层 JSON domainlist）。"""
        if self.dry_run:
            return {"ok": True, "msg": "dry-run: 已模拟创建站点，未真正执行",
                    "data": {"webname": webname, "path": path, "php_version": php_version}}
        data = {
            "webname": json.dumps({"domain": webname, "domainlist": [], "count": 0}),
            "path": path,
            "type": "PHP",
            "version": php_version,
            "port": "80",
            "ps": "created by btpanel-mcp",
            "ftp": "false",
            "sql": "false",
        }
        return self._request("/site?action=AddSite", data)

    def set_site_status(self, site_name: str, status: int) -> dict:
        """status: 0=停用, 1=启用。"""
        if self.dry_run:
            return {"ok": True, "msg": "dry-run: 已模拟切换站点状态，未真正执行",
                    "data": {"site_name": site_name, "status": status}}
        return self._request("/site?action=SetSiteStatus", {"id": site_name, "status": status})

    # ------------------------------------------------------------------ #
    # 数据库管理
    # ------------------------------------------------------------------ #
    def list_databases(self, page: int = 1, limit: int = 15) -> dict:
        return self._request("/data?action=getData", {"table": "databases", "limit": limit, "p": page})

    def create_database(self, name: str, password: str | None = None, db_user: str | None = None) -> dict:
        data = {
            "name": name,
            "codeing": "utf8mb4",
            "db_user": db_user or name,
            "password": password or _random_password(),
            "ps": "created by btpanel-mcp",
            "dtype": "MySQL",
        }
        if self.dry_run:
            return {"ok": True, "msg": "dry-run: 已模拟创建数据库，未真正执行", "data": data}
        return self._request("/database?action=AddDatabase", data)

    # ------------------------------------------------------------------ #
    # FTP 管理
    # ------------------------------------------------------------------ #
    def list_ftps(self, page: int = 1, limit: int = 15) -> dict:
        return self._request("/data?action=getData", {"table": "ftps", "limit": limit, "p": page})

    def create_ftp(self, name: str, password: str, path: str) -> dict:
        data = {
            "ftp_username": name,
            "ftp_password": password,
            "path": path,
            "ps": "created by btpanel-mcp",
        }
        if self.dry_run:
            return {"ok": True, "msg": "dry-run: 已模拟创建 FTP，未真正执行", "data": data}
        return self._request("/ftp?action=AddFtp", data)

    # ------------------------------------------------------------------ #
    # 文件管理
    # ------------------------------------------------------------------ #
    def list_dir(self, path: str) -> dict:
        """列目录：优先 /files?action=GetDir，失败回退 /files/get_dir_list。"""
        resp = self._request("/files?action=GetDir", {"path": path})
        if resp.get("ok"):
            return {"ok": True, "msg": resp.get("msg", ""), "data": self._normalize_dir(resp.get("data"))}
        resp2 = self._request("/files/get_dir_list", {
            "p": 1, "limit": 500, "path": path, "sort": "name", "reverse": "false",
        })
        if resp2.get("ok"):
            return {"ok": True, "msg": resp2.get("msg", ""), "data": self._normalize_dir(resp2.get("data"))}
        return resp

    @staticmethod
    def _normalize_dir(data) -> dict:
        """归一化 GetDir / get_dir_list 两种目录结构差异。"""
        if isinstance(data, str):
            try:
                data = json.loads(data)
            except json.JSONDecodeError:
                return {"dirs": [], "files": [], "raw": data[:500]}
        if not isinstance(data, dict):
            return {"dirs": [], "files": []}
        dirs: list = []
        files: list = []
        for key, target in (("DIR", dirs), ("dirs", dirs), ("dir", dirs),
                            ("FILES", files), ("files", files)):
            for entry in data.get(key, []) or []:
                if isinstance(entry, str):
                    target.append(entry.split(";", 1)[0])
                elif isinstance(entry, dict):
                    target.append(entry)
        return {"dirs": dirs, "files": files}

    def read_file(self, path: str) -> dict:
        return self._request("/files?action=GetFileBody", {"path": path})

    def write_file(self, path: str, content: str, encoding: str = "utf-8") -> dict:
        """写文件：优先 /files?action=SaveFileBody，失败回退 /files/save_file_body（filename 字段）。"""
        if self.dry_run:
            return {"ok": True, "msg": "dry-run: 已模拟写入文件，未真正执行",
                    "data": {"path": path, "bytes": len(content.encode("utf-8"))}}
        resp = self._request("/files?action=SaveFileBody", {"path": path, "data": content, "encoding": encoding})
        if not resp.get("ok"):
            resp2 = self._request("/files/save_file_body", {"filename": path, "data": content, "encoding": encoding})
            if resp2.get("ok"):
                return resp2
        return resp

    def delete_file(self, path: str) -> dict:
        """删除文件：优先 /files/delete_file，失败回退 /files?action=DeleteFile。"""
        if self.dry_run:
            return {"ok": True, "msg": "dry-run: 已模拟删除文件，未真正执行", "data": {"path": path}}
        resp = self._request("/files/delete_file", {"path": path})
        if not resp.get("ok"):
            resp2 = self._request("/files?action=DeleteFile", {"path": path})
            if resp2.get("ok"):
                return resp2
        return resp

    def upload_file(self, local_path: str, remote_dir: str, chunk_size: int = 2 * 1024 * 1024) -> dict:
        """分片上传本地文件到面板目录（POST /files/upload，每片默认 2MB）。"""
        lp = Path(local_path)
        if not lp.is_file():
            return {"ok": False, "msg": f"本地文件不存在: {local_path}", "data": None}
        total = lp.stat().st_size
        total_chunks = max(1, (total + chunk_size - 1) // chunk_size)
        filename = lp.name
        if self.dry_run:
            return {"ok": True, "msg": "dry-run: 已模拟分片上传，未真正执行",
                    "data": {"filename": filename, "remote_dir": remote_dir, "size": total, "chunks": total_chunks}}
        with lp.open("rb") as f:
            for idx in range(total_chunks):
                f.seek(idx * chunk_size)
                blob = f.read(chunk_size)
                fields = {
                    "path": remote_dir,
                    "filename": filename,
                    "chunk": idx,
                    "chunks": total_chunks,
                    "size": total,
                    "force": 1,
                }
                resp = self._request("/files/upload", fields, files={"blob": (filename, blob)})
                if not resp.get("ok"):
                    return {"ok": False, "msg": f"分片 {idx + 1}/{total_chunks} 上传失败: {resp.get('msg', '')}",
                            "data": resp.get("data")}
        return {"ok": True, "msg": f"上传完成（{total_chunks} 分片，共 {total} 字节）",
                "data": {"filename": filename, "remote_dir": remote_dir, "size": total}}
