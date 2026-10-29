"""Boot the *shipped* Python sidecar and make real requests to it.

Import verification proves a module can be loaded; it says nothing about whether
calling into it works. Cython-compiled code can pass every import check and still
fail at dispatch time (2026-10-29: every starlette route answered 500 with
`health() takes exactly 1 positional argument (3 given)` because `Route` only
recognises `inspect.isfunction()` endpoints and a compiled closure is a
`cython_function_or_method`). Only an actual boot + request catches that class.

Run against a staged/materalised resource tree:

    python scripts/check-sidecar-boot.py                    # gui/src-tauri/resources
    python scripts/check-sidecar-boot.py --root target/release/resources

Exit codes: 0 ok, 1 the sidecar failed, 2 the tree is not bootable (nothing to test).
The real ~/.marguerite is never touched — OPENSQUILLA_HOME points at a temp dir.
"""

from __future__ import annotations

import argparse
import base64
import os
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ROOT = REPO_ROOT / "gui" / "src-tauri" / "resources"
TOKEN = "verify-sidecar-boot-token"

# Endpoints the desktop console hits on boot. Any 5xx is a dispatch failure; a
# 404/403 is a legitimate answer (route disabled, or needs a body we are not
# sending) and is reported but not fatal.
PROBES = (
    "/health",
    "/api/config",
    "/api/sessions",
    "/api/system/status",
    "/api/agents",
    "/api/approvals",
    "/control/",
)


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def ws_handshake(port: int, path: str = "/ws", timeout: float = 15.0) -> tuple[str, bool]:
    """Return (status line, whether the `connect.challenge` event arrived).

    A 101 only proves `websocket_session()` wrapped the endpoint; the challenge
    event proves the compiled endpoint body itself ran far enough to send.
    """
    key = base64.b64encode(os.urandom(16)).decode("ascii")
    request = (
        f"GET {path} HTTP/1.1\r\n"
        f"Host: 127.0.0.1:{port}\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        "Sec-WebSocket-Version: 13\r\n"
        f"Authorization: Bearer {TOKEN}\r\n\r\n"
    )
    chunks: list[bytes] = []
    with socket.create_connection(("127.0.0.1", port), timeout=15) as sock:
        sock.sendall(request.encode("ascii"))
        sock.settimeout(0.5)
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                data = sock.recv(4096)
            except socket.timeout:
                continue
            if not data:
                break
            chunks.append(data)
            if b"connect.challenge" in b"".join(chunks):
                break
    raw = b"".join(chunks)
    status = raw.split(b"\r\n", 1)[0].decode("latin-1", "replace") if raw else "empty response"
    return status, b"connect.challenge" in raw


def http_status(port: int, path: str) -> tuple[int | None, str]:
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}{path}",
        headers={"Authorization": f"Bearer {TOKEN}"},
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.status, response.read(160).decode("utf-8", "replace")
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read(160).decode("utf-8", "replace")
    except Exception as exc:  # noqa: BLE001 - probe, reported verbatim
        return None, f"{type(exc).__name__}: {exc}"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=str(DEFAULT_ROOT))
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args(argv)

    root = Path(args.root).resolve()
    python = root / "python" / "python.exe"
    source = root / "pyengine" / "src" / "opensquilla"
    if not python.is_file():
        print(f"[FAIL] {python} 不存在：这棵树没有内置解释器，装出来的侧车起不来")
        return 2
    if not source.is_dir():
        print(f"[FAIL] {source} 不存在：资源树里没有 pyengine 源码树")
        return 2

    home = Path(tempfile.mkdtemp(prefix="sidecar-boot-"))
    port = free_port()
    env = dict(os.environ)
    env.update(
        {
            "PYTHONPATH": str(root / "pyengine" / "src"),
            "OPENSQUILLA_HOME": str(home),
            "OPENSQUILLA_GATEWAY_TOKEN": TOKEN,
            "PYTHONUNBUFFERED": "1",
            # The tree we are booting is the installer payload; a cache directory
            # written into it is something check-shipped-resources rejects.
            "PYTHONDONTWRITEBYTECODE": "1",
        }
    )
    command = [
        str(python),
        "-m",
        "opensquilla.cli.main",
        "gateway",
        "run",
        "--bind",
        "127.0.0.1",
        "--port",
        str(port),
    ]
    print(f"[info] 启动侧车: {' '.join(command)}")
    captured = open(home / "sidecar-boot.log", "wb")  # noqa: SIM115 - closed in finally
    process = subprocess.Popen(command, env=env, stdout=captured, stderr=subprocess.STDOUT)
    try:
        deadline = time.time() + args.timeout
        listening = False
        while time.time() < deadline:
            if process.poll() is not None:
                break
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=1):
                    listening = True
                    break
            except OSError:
                time.sleep(0.5)

        if not listening:
            captured.close()
            tail = (home / "sidecar-boot.log").read_text("utf-8", "replace")[-2000:]
            print(f"[FAIL] 侧车没有监听 {port}（rc={process.poll()}）")
            print(tail)
            return 1

        failures: list[str] = []
        for path in PROBES:
            status, body = http_status(port, path)
            print(f"  {path}: {status if status is not None else body}")
            if status is None or status >= 500:
                failures.append(f"{path} -> {status if status is not None else body}")

        try:
            status, challenged = ws_handshake(port)
        except Exception as exc:  # noqa: BLE001 - probe
            status, challenged = f"{type(exc).__name__}: {exc}", False
        print(f"  /ws: {status} (connect.challenge: {'yes' if challenged else 'no'})")
        if not status.startswith("HTTP/1.1 101"):
            failures.append(f"/ws 握手失败 -> {status}")
        elif not challenged:
            failures.append("/ws 已升级但收不到 connect.challenge 事件")

        if failures:
            print(f"[FAIL] 侧车能启动但请求失败（{len(failures)} 项）:")
            for item in failures:
                print(f"  - {item}")
            return 1
        print("[PASS] 侧车启动成功，HTTP 路由与 /ws 握手均正常")
        return 0
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                process.kill()
        if not captured.closed:
            captured.close()


if __name__ == "__main__":
    # Sidecar log tails and CJK paths reach stdout; pin UTF-8 so a cp936/cp1252
    # console neither crashes the reporter nor turns the finding into mojibake.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="backslashreplace")
    sys.exit(main(sys.argv[1:]))
