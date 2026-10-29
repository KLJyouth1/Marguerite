"""Dry-run parse validator for Chinese Windows batch scripts.

cmd.exe slices a batch file into byte windows; with chcp 65001 over UTF-8-without-BOM
those windows land mid-character on multi-byte Chinese and a line gets split into a
phantom command. The split can cut a real command in half, so this is not cosmetic.

Detection cannot be a read of the bytes: it needs cmd's own parser. So every command
token is neutered to `echo` and the copy is handed to `cmd /c`. A parse-healthy file
produces empty stderr regardless of what it echoes.

The validator must decode subprocess output with the *same* encoding it wrote, or the
GBK bytes come back as UTF-8 mojibake and invent failures that do not exist.
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# Tokens that would actually do something if the dry run executed them.
NEUTER = (
    r"cargo|python|powershell|robocopy|xcopy|copy|move|del|erase|rd|rmdir|mkdir|"
    r"md|pushd|popd|cd|call|start|taskkill|reg|sc|msiexec|curl|wget|tar|zip|unzip|"
    r"git|npm|npx|node|goto|exit|pause|cls|chcp|setlocal|endlocal|title|echo"
)
LINE_COMMAND = re.compile(r"^(\s*)(" + NEUTER + r")\b", re.IGNORECASE)
# `if exist "x" cmd` / `if errorlevel 1 (` — the `if` must stay so the block structure
# (and therefore the byte offsets cmd reads) is preserved exactly.
INLINE_COMMAND = re.compile(r"^(\s*if\b[^\(]*?)\b(" + NEUTER + r")\b", re.IGNORECASE)


def neuter(text: str) -> str:
    out = []
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.upper().startswith("REM") or stripped.startswith(":"):
            out.append(line)
            continue
        line, hits = LINE_COMMAND.subn(r"\1echo", line, count=1)
        if hits == 0:
            line = INLINE_COMMAND.sub(r"\1 echo", line, count=1)
        out.append(line)
    return "\n".join(out)


def read_bat(path: Path) -> tuple[str, str, bytes]:
    raw = path.read_bytes()
    if raw.startswith(b"\xef\xbb\xbf"):
        return raw[3:].decode("utf-8"), "utf-8", b"\xef\xbb\xbf"
    try:
        return raw.decode("utf-8"), "utf-8", b""
    except UnicodeDecodeError:
        return raw.decode("gbk"), "gbk", b""


def check(path: Path, log: Path) -> int:
    text, enc, bom = read_bat(path)
    with tempfile.TemporaryDirectory(prefix="bat-parse-") as tmp:
        script = Path(tmp) / "dry_run.cmd"
        script.write_bytes(bom + neuter(text).replace("\n", "\r\n").encode(enc))
        try:
            proc = subprocess.run(
                ["cmd", "/c", str(script)],
                capture_output=True,
                text=True,
                errors="replace",
                encoding=enc,
                stdin=subprocess.DEVNULL,
                timeout=120,
            )
        except subprocess.TimeoutExpired:
            # A neutered script can only hang if cmd is still waiting for the rest of a
            # block it failed to close, i.e. the mis-parse swallowed a `)`.
            with log.open("a", encoding="utf-8") as handle:
                handle.write(f"=== {path} ({enc}) ===\nblocked: unclosed block, timed out\n")
            print(f"[FAIL] {path}: cmd.exe blocked on a broken block ({enc}, timed out)")
            return 1
        stderr = proc.stderr.strip()
        total = len(text.splitlines())
        if stderr:
            bad = [ln for ln in stderr.splitlines() if ln.strip()]
            # The lines a mis-parse produces are mojibake by construction, so the
            # console copy is escaped to ASCII and the readable full text goes to a log.
            with log.open("a", encoding="utf-8") as handle:
                handle.write(f"=== {path} ({enc}, {total} lines) ===\n")
                for ln in bad:
                    handle.write(ln + "\n")
            print(f"[FAIL] {path}: cmd.exe mis-parses {enc} + CRLF batch")
            for ln in bad[:6]:
                print("        " + ln.encode("ascii", "backslashreplace").decode("ascii"))
            print(f"        ({len(bad)} stderr line(s); full text in {log})")
            return 1
        print(f"[OK]   {path}: parse clean ({enc}, {total} lines)")
        return 0


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("usage: python scripts/check-batch-parse.py <file.cmd> [...]", file=sys.stderr)
        return 2
    # A cp936 console cannot encode the U+FFFD / rare CJK marks that a mis-parse emits;
    # crashing the reporter would hide the very evidence this script exists to produce.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(errors="backslashreplace")
    log = Path(tempfile.gettempdir()) / "batch-parse-details.txt"
    log.write_text("", encoding="utf-8")
    codes = [check(Path(p), log) for p in argv[1:]]
    return 1 if any(codes) else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
