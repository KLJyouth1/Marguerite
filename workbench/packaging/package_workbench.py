"""Package the Marguerite Workbench into a distributable zip + verified layout.

Usage (from anywhere):
    python workbench/packaging/package_workbench.py            # build, stage, verify, zip
    python workbench/packaging/package_workbench.py --no-zip   # stage + verify only
    python workbench/packaging/package_workbench.py --no-verify

Canonical workbench layout (post-consolidation):

    workbench/
      backend/marguerite_workbench/   # python package (engine + API + CLI + tests)
      frontend/                       # React SPA (workbench-web)
      packaging/                      # this packager + layout/version guards
      dist/                           # generated zips land here
      VERSION                         # single source of truth (checked by check_version_sync)

The staged layout is self-contained and runnable in place:

    marguerite-workbench/<version>/
      marguerite_workbench/   # python package (sources + tests)
      webapp/                 # built SPA served by the API at "/"
      requirements.txt  VERSION  README.md
      start.ps1 / start.bat / start.sh   # one-command launchers (loopback)

Verification boots the *staged* artifact with uvicorn on a free loopback port,
then probes ``/api/health``, the SPA index at ``/`` and a static asset — the
same topology end users will run.  No network beyond 127.0.0.1 is touched.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import time
import urllib.request
import zipfile
from pathlib import Path

WORKBENCH_ROOT = Path(__file__).resolve().parents[1]
PACKAGE_SRC = WORKBENCH_ROOT / "backend" / "marguerite_workbench"
WEB_DIR = WORKBENCH_ROOT / "frontend"
DELIVERABLES = WORKBENCH_ROOT / "dist"
VERSION_FILE = WORKBENCH_ROOT / "VERSION"

EXCLUDE_DIRS = {"__pycache__", ".pytest_cache", "data", "sessions", "node_modules", ".git"}
_VERSION_RE = re.compile(r'version="([0-9]+\.[0-9]+\.[0-9]+)"')


def detect_version() -> str:
    """Single source of truth: workbench/VERSION, cross-checked against api.py."""
    declared = VERSION_FILE.read_text(encoding="utf-8").strip()
    text = (PACKAGE_SRC / "api.py").read_text(encoding="utf-8")
    match = _VERSION_RE.search(text)
    if not match:
        raise RuntimeError("cannot detect version from api.py")
    if match.group(1) != declared:
        raise RuntimeError(
            f"version drift: VERSION={declared} but api.py={match.group(1)} "
            f"(run check_version_sync.py)"
        )
    return declared


def build_web() -> None:
    npm = shutil.which("npm")
    if not npm:
        raise RuntimeError("npm not found on PATH; install Node.js to build the SPA")
    print("[1/5] building workbench-web …")
    subprocess.run([npm, "run", "build"], cwd=str(WEB_DIR), check=True,
                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                   encoding="utf-8", errors="replace")


def copy_tree_filtered(src: Path, dst: Path) -> int:
    """Copy sources while skipping caches/runtime data. Returns file count."""
    count = 0
    for path in src.rglob("*"):
        if any(part in EXCLUDE_DIRS for part in path.relative_to(src).parts):
            continue
        if path.is_dir():
            continue
        target = dst / path.relative_to(src)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)
        count += 1
    return count


def stage(version: str) -> Path:
    print("[2/5] staging package tree …")
    root = DELIVERABLES / "package_stage" / f"marguerite-workbench-{version}"
    if root.exists():
        shutil.rmtree(root)  # managed-python removal (bare rm is hook-guarded here)
    root.mkdir(parents=True)

    n_py = copy_tree_filtered(PACKAGE_SRC, root / "marguerite_workbench")

    dist = WEB_DIR / "dist"
    if not (dist / "index.html").is_file():
        raise RuntimeError("workbench-web/dist/index.html missing — run the web build first")
    n_web = copy_tree_filtered(dist, root / "webapp")

    shutil.copy2(PACKAGE_SRC / "requirements.txt", root / "requirements.txt")
    shutil.copy2(PACKAGE_SRC / "README.md", root / "README.md")
    (root / "VERSION").write_text(version + "\n", encoding="utf-8")

    (root / "start.ps1").write_text(_START_PS1, encoding="utf-8-sig")  # BOM: PowerShell 5.1 readability
    (root / "start.bat").write_text(_START_BAT, encoding="gbk", errors="replace")
    (root / "start.sh").write_text(_START_SH, encoding="utf-8")
    for sh in (root / "start.sh",):
        os.chmod(sh, 0o755)

    print(f"      staged {n_py} python files + {n_web} web assets → {root}")
    return root


def _free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _get(opener: urllib.request.OpenerDirector, url: str) -> tuple[int, bytes]:
    with opener.open(url, timeout=10) as resp:
        return resp.status, resp.read()


def verify(root: Path) -> None:
    print("[3/5] verifying staged artifact (real boot on loopback) …")
    import tempfile

    port = _free_port()
    env = dict(os.environ)
    tmp = tempfile.mkdtemp(prefix="mq_pkg_verify_")
    env.update({
        "MARGERITE_DATA_DIR": str(Path(tmp) / "data"),
        "MARGERITE_WORKSPACE": str(Path(tmp) / "workspace"),
        "MARGERITE_WEBAPP_DIR": str(root / "webapp"),
        "MARGUERITE_HOME": str(Path(tmp) / "home"),     # keep verification off the real core store
        "NO_PROXY": "127.0.0.1,localhost",
        "no_proxy": "127.0.0.1,localhost",
    })
    # Explicit empty ProxyHandler defeats Windows system-proxy hijacking of loopback.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    process = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "marguerite_workbench.api:app",
         "--host", "127.0.0.1", "--port", str(port), "--log-level", "warning"],
        cwd=str(root), env=env,
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        encoding="utf-8", errors="replace",
    )
    base = f"http://127.0.0.1:{port}"
    try:
        deadline = time.time() + 30
        last_error: str | None = "server did not come up"
        while time.time() < deadline:
            if process.poll() is not None:
                out = process.stdout.read() if process.stdout else ""
                raise RuntimeError(f"uvicorn exited early:\n{out[-2000:]}")
            try:
                status, body = _get(opener, f"{base}/api/health")
                if status == 200 and b"ok" in body:
                    last_error = None
                    break
            except Exception as exc:  # noqa: BLE001 — retry until the deadline
                last_error = str(exc)
            time.sleep(0.5)
        if last_error:
            raise RuntimeError(f"health probe failed: {last_error}")

        checks: dict[str, object] = {}
        status, html = _get(opener, f"{base}/")
        checks["spa_index"] = status == 200 and b'id="root"' in html
        asset = re.search(rb'src="(/assets/[^"]+\.js)"', html)
        if asset:
            a_status, _ = _get(opener, base + asset.group(1).decode())
            checks["spa_asset"] = a_status == 200
        _, tools = _get(opener, f"{base}/api/agent/tools")
        checks["agent_tools"] = len(json.loads(tools)["tools"]) >= 4
        _, caps = _get(opener, f"{base}/harness/capabilities")
        checks["capabilities"] = json.loads(caps)["service"] == "marguerite-file-intel"

        failed = [k for k, ok in checks.items() if not ok]
        if failed:
            raise RuntimeError(f"verification checks failed: {failed}")
        print(f"      all checks passed: {sorted(checks)} @ {base}")
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
        shutil.rmtree(tmp, ignore_errors=True)


def make_zip(root: Path, version: str) -> Path:
    print("[4/5] creating zip …")
    DELIVERABLES.mkdir(parents=True, exist_ok=True)
    zip_base = DELIVERABLES / f"Marguerite_Workbench_{version}"
    archive = zipfile.ZipFile(str(zip_base) + ".zip", "w", zipfile.ZIP_DEFLATED)
    with archive:
        for path in sorted(root.rglob("*")):
            if path.is_file():
                archive.write(path, path.relative_to(root.parent))
    size_mb = (Path(str(zip_base) + ".zip").stat().st_size) / (1024 * 1024)
    print(f"      {zip_base}.zip ({size_mb:.2f} MB)")
    return Path(str(zip_base) + ".zip")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--no-zip", action="store_true", help="stage + verify without archiving")
    parser.add_argument("--no-verify", action="store_true", help="skip the boot smoke test")
    args = parser.parse_args(argv)

    version = detect_version()
    print(f"packaging Marguerite Workbench v{version}")

    build_web()
    root = stage(version)
    if not args.no_verify:
        verify(root)
    else:
        print("[3/5] verification skipped (--no-verify)")
    if args.no_zip:
        print("[4/5] zip skipped (--no-zip)")
    else:
        make_zip(root, version)
    print("[5/5] done.")
    return 0


_START_PS1 = r'''
# Marguerite Workbench — one-command launcher (loopback only).
$ErrorActionPreference = "Stop"
$root = $PSScriptRoot
$env:MARGERITE_WEBAPP_DIR = Join-Path $root "webapp"
Set-Location $root
Write-Host "Marguerite Workbench -> http://127.0.0.1:8011  (Ctrl+C to stop)" -ForegroundColor Cyan
python -m marguerite_workbench serve --host 127.0.0.1 --port 8011
'''

_START_BAT = (
    "@echo off\r\n"
    "setlocal\r\n"
    "cd /d \"%~dp0\"\r\n"
    "set MARGERITE_WEBAPP_DIR=%~dp0webapp\r\n"
    "echo Marguerite Workbench -^> http://127.0.0.1:8011  (Ctrl+C to stop)\r\n"
    "python -m marguerite_workbench serve --host 127.0.0.1 --port 8011\r\n"
    "endlocal\r\n"
)

_START_SH = '''#!/usr/bin/env bash
# Marguerite Workbench — one-command launcher (loopback only).
set -e
root="$(cd "$(dirname "$0")" && pwd)"
export MARGERITE_WEBAPP_DIR="$root/webapp"
cd "$root"
echo "Marguerite Workbench -> http://127.0.0.1:8011  (Ctrl+C to stop)"
exec python3 -m marguerite_workbench serve --host 127.0.0.1 --port 8011
'''

if __name__ == "__main__":
    raise SystemExit(main())
