#!/usr/bin/env python3
"""RPC 面与事件面的双向对账：声明 ↔ 实现 ↔ 前端调用。

前身版本用 `seg = methods_rs[idx:idx+30000]` 截断扫描 `dispatch_rust`，只要分派表
超过 30 KB，后面所有 arm 都会被当成「未实现」—— 实测它对 200 个方法谎报「声明但
无分派」（`desktop.attach`、`goal.create`、`harness.ledger.*` 全在名单里）。一份把
两千行正常代码判成缺陷的报告，等于没有报告。

本版按花括号配对取整个函数体，没有长度上限；三类结论分开报，且分开给退出码：

  [A] 有实现但没声明  → 真实反向漂移：实现可达，`supportsMethod()` 却是 false，
      前端永远不调。**非空即 exit 1**。
  [B] 声明但无实现    → 孤儿声明：UI 认为能用，一调就失败。**非空即 exit 1**。
  [C] 前端动态方法名  → 模板串拼出来的方法名，静态扫描不可判定，逐条列出供人工确认。
      只报不拦（它不是缺陷本身，但它让 A/B 的静态结论有盲区）。
  [D] 事件面          → 服务端声明的事件 vs 前端 `on(`/`subscribe` 的事件名。

用法：`python scripts/audit_rpc_surface2.py [--format=json] [--strict-dynamic]`
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CORE = ROOT / "core" / "src"
METHODS_RS = CORE / "gateway" / "methods.rs"
PROTOCOL_RS = CORE / "gateway" / "protocol.rs"
WEBUI = ROOT / "webui" / "src"


# --------------------------------------------------------------------------- #
# Rust source scanning (brace-matched, string/comment aware)
# --------------------------------------------------------------------------- #
def strip_noncode(src: str, keep_literals: bool = False) -> str:
    """Blank out comments and (by default) literals, preserving offsets.

    `keep_literals=True` blanks comments only — needed when the literals are the
    payload being extracted, as in `list_literal`.

    Brace matching must not be fooled by `'{'` inside a string or a commented-out
    block — both occur in this file.
    """
    out: list[str] = []
    i, n = 0, len(src)
    while i < n:
        ch = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if ch == "/" and nxt == "/":
            j = src.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif ch == "/" and nxt == "*":
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append("".join(c if c == "\n" else " " for c in src[i:j]))
            i = j
        elif ch == "r" and nxt in ('"', "#"):
            # r"..." / r#"..."# / r##"..."##
            hashes = 0
            j = i + 1
            while j < n and src[j] == "#":
                hashes += 1
                j += 1
            if j < n and src[j] == '"':
                close = '"' + "#" * hashes
                k = src.find(close, j + 1)
                k = n if k < 0 else k + len(close)
                out.append("".join(c if c == "\n" else " " for c in src[i:k]))
                i = k
            else:
                out.append(ch)
                i += 1
        elif ch == '"':
            j = i + 1
            while j < n:
                if src[j] == "\\":
                    j += 2
                    continue
                if src[j] == '"':
                    j += 1
                    break
                j += 1
            out.append(
                src[i:j]
                if keep_literals
                else "".join(c if c == "\n" else " " for c in src[i:j])
            )
            i = j
        elif ch == "'":
            # A char literal is `'x'` or `'\n'`; a lifetime is `'a` with no closer.
            m = re.match(r"'(?:\\.[^']{0,8}|[^'\\])'", src[i:])
            if m:
                out.append(" " * len(m.group(0)))
                i += len(m.group(0))
            else:
                out.append(ch)
                i += 1
        else:
            out.append(ch)
            i += 1
    return "".join(out)


def function_body(src: str, signature_regex: str) -> str:
    """Return the brace-matched body of the first function matching the regex."""
    code = strip_noncode(src)
    match = re.search(signature_regex, code)
    if not match:
        return ""
    open_brace = code.find("{", match.end())
    if open_brace < 0:
        return ""
    depth = 0
    for i in range(open_brace, len(code)):
        if code[i] == "{":
            depth += 1
        elif code[i] == "}":
            depth -= 1
            if depth == 0:
                return src[open_brace + 1 : i]
    return src[open_brace + 1 :]


def list_literal(src: str, name: str) -> set[str]:
    # Comments inside the array carry prose (and backticked method names); strip
    # them first or `list_literal` reports sentences as declared methods.
    text = strip_noncode(src, keep_literals=True)
    match = re.search(re.escape(name) + r"[^=;]*=\s*&\[(.*?)\]\s*;", text, re.S)
    if not match:
        return set()
    return set(re.findall(r'"([^"]+)"', match.group(1)))


def table_methods(path: Path) -> set[str]:
    """Methods dispatched from a static table (`("ops.status", "status")`).

    Text after `#[cfg(test)]` is excluded: `dispatch_with_executor("ops.delete_file", …)`
    in a unit test is a fixture asserting a rejection, not a dispatch arm. Treating it
    as implemented put a working guard on a method that gateway rejects on purpose.
    """
    if not path.is_file():
        return set()
    text = path.read_text(encoding="utf-8")
    cut = text.find("#[cfg(test)]")
    if cut >= 0:
        text = text[:cut]
    return set(
        re.findall(
            r'"([a-z][a-z0-9_]*\.[a-z][a-z0-9_.]*)"\s*,',
            strip_noncode(text, keep_literals=True),
        )
    )


PYENGINE = ROOT / "pyengine" / "src"
# Method names are camelCase in the sidecar (`sessions.promptCacheKeepalive.status`);
# a lowercase-only class skipped exactly the five names this scan exists to find.
PY_METHOD = re.compile(r'@_d\.method\(\s*"([a-z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)+)"')


def pyengine_methods() -> set[str]:
    """Methods the Python sidecar registers (`@_d.method("name")`).

    The gateway forwards unknown methods to the sidecar, so a name implemented
    only in `pyengine/src` is reachable — reporting it as "declared but no
    implementation" was the analyzer lying about five working methods
    (`sandbox.resume`, `skills.deps.install`, `sessions.promptCacheKeepalive.*`,
    `sessions.messages.unsubscribe`).
    """
    if not PYENGINE.is_dir():
        return set()
    found: set[str] = set()
    for path in sorted(PYENGINE.rglob("*.py")):
        if "/tests/" in path.as_posix():
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        found.update(PY_METHOD.findall(text))
    return found


# --------------------------------------------------------------------------- #
# Surfaces
# --------------------------------------------------------------------------- #
def rpc_surface() -> dict:
    methods_src = METHODS_RS.read_text(encoding="utf-8")
    protocol_src = PROTOCOL_RS.read_text(encoding="utf-8")

    declared = list_literal(protocol_src, "pub const SUPPORTED_METHODS")
    prefixes = set(list_literal(methods_src, "static PY_ENGINE_PREFIXES"))
    prefixes = {p.rstrip(".") for p in prefixes}

    bodies = [
        function_body(methods_src, r"fn dispatch_rust\s*\("),
        function_body(methods_src, r"async fn dispatch\s*\("),
        function_body(methods_src, r"fn dispatch\s*\("),
    ]
    arms: set[str] = set()
    guards: set[str] = set()
    for body in bodies:
        # `-` belongs in the class: `sandbox.path.create-directory` was silently
        # dropped and reported as an orphan while its arm sat right there.
        arms.update(re.findall(r'"([A-Za-z][A-Za-z0-9_.\-]*)"\s*=>', body))
        guards.update(re.findall(r'method\s*==\s*"([^"]+)"', body))
        guards.update(re.findall(r'method\.starts_with\(\s*"([^"]+)"', body))

    implemented = arms | guards
    # Prefix guards (`method.starts_with("goal.")`) cover a family, not a method.
    guard_prefixes = {g for g in guards if g.endswith(".")}
    implemented |= table_methods(CORE / "gateway" / "ops_dispatch.rs")

    sidecar = pyengine_methods()
    reachable = implemented | sidecar

    def covered(name: str) -> bool:
        return name in reachable or any(name.startswith(p) for p in prefixes) or any(
            name.startswith(g) for g in guard_prefixes
        )

    undeclared = sorted(
        m
        for m in implemented
        if m not in declared and "." in m and not m.endswith(".") and not is_fixture(m)
    )
    dead = sorted(d for d in declared if not covered(d))
    # Console-reachable only through `supportsMethod()`; a sidecar method without a
    # declaration is callable by the sidecar itself but invisible to the webui.
    sidecar_only = sorted(m for m in sidecar if m not in declared and not is_fixture(m))
    return {
        "declared_count": len(declared),
        "implemented_count": len(implemented),
        "sidecar_count": len(sidecar),
        "undeclared": undeclared,
        "dead": dead,
        "sidecar_only": sidecar_only,
    }


EVENT_CALL = re.compile(r"""(?:\bon|\$on|subscribe|listen)\(\s*['"`]([a-z][a-zA-Z0-9_.]{2,})['"`]""")


# Braces are part of the class: `f"{namespace}.approval.{event}"` is an emitter
# template, and a class without them silently dropped every f-string.
QUOTED = re.compile(r'"([A-Za-z{][A-Za-z0-9_.\-:{}]{2,120})"')


def _quoted_prefixes() -> tuple[set[str], list[re.Pattern[str]]]:
    """Every dotted prefix of every quoted literal in Rust + sidecar sources.

    Event names are assembled at runtime in two places: Rust
    (`format!("session.event.{other}")` in `gateway/events.rs`) and Python
    (`self._ws_emitter(topic, "cron.run.finished", …)`). So the emitter side has
    no table to diff against — the honest check is "does any literal anywhere
    establish this namespace", compared segment-wise so `"channels.status"`
    does not vouch for `channel.status`.
    """
    sources = [path for path in sorted(CORE.rglob("*.rs"))]
    if PYENGINE.is_dir():
        sources += [
            path
            for path in sorted(PYENGINE.rglob("*.py"))
            if "/tests/" not in path.as_posix()
        ]
    prefix_set: set[str] = set()
    templates: list[re.Pattern[str]] = []
    for path in sources:
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for literal in QUOTED.findall(text):
            if "{" in literal:
                # `f"{namespace}.approval.{event}"` / `f"task.{status}"` are real
                # emitters whose names never exist as literals. The generic
                # `session.event.{other}` wrapper is skipped — if it counted,
                # every `session.event.*` subscription would be trivially covered.
                skeleton = re.sub(r"\{[^}]*\}", "\x00", literal)
                # `"{}"` / `"{a}.{b}.{c}"`-shaped literals reduce to bare wildcards
                # that match almost every event name — they carry no evidence.
                # Require at least two literal alphanumerics to count as an emitter.
                if not literal.startswith("session.event.") and sum(
                    ch.isalnum() for ch in skeleton
                ) >= 2:
                    body = "".join(
                        r"[A-Za-z0-9_.\-]+" if ch == "\x00" else re.escape(ch)
                        for ch in skeleton
                    )
                    templates.append(re.compile("^" + body + "$"))
                continue
            parts = literal.split(".")
            for i in range(1, len(parts) + 1):
                prefix_set.add(".".join(parts[:i]))
    return prefix_set, templates


def event_surface() -> dict:
    """Events have no single declaration list — that is the finding.

    `protocol.rs` declares five; the console listens on sixty-nine. Rather than
    invent an authority, this section answers the only question the code can
    answer: does any source mention the name the console listens for?
    A listener with no emitter anywhere is a subscription that can never fire.
    """
    prefixes, templates = _quoted_prefixes()

    def produced(name: str) -> bool:
        return name in prefixes or any(p.match(name) for p in templates)

    subscribed: dict[str, list[str]] = {}
    for path in sorted(WEBUI.rglob("*")):
        if path.suffix not in {".ts", ".vue", ".js"} or not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except OSError:
            continue
        for lineno, line in enumerate(text.splitlines(), 1):
            for name in EVENT_CALL.findall(line):
                subscribed.setdefault(name, []).append(
                    f"{path.relative_to(ROOT).as_posix()}:{lineno}"
                )

    def covered(name: str) -> bool:
        if produced(name):
            return True
        # The gateway wraps every sidecar event kind in `session.event.`
        # (`gateway/events.rs`), so what must exist is the kind, not the full name.
        if name.startswith("session.event."):
            return produced(name[len("session.event.") :])
        return False

    orphans = sorted(name for name in subscribed if not covered(name))
    return {
        "subscribed": {k: v[:2] for k, v in sorted(subscribed.items())},
        "no_rust_emitter": orphans,
    }


RPC_CALL = re.compile(r"""(?:call|invoke|request|rpc|send)\w*\(\s*`([^`]+)`""")
METHOD_SHAPE = re.compile(r"^[a-z][a-z0-9_]*(\.[a-z0-9_]+)+$")

FIXTURE_TAILS = {"nope", "foo", "bar", "baz", "test", "x", "y"}


def is_fixture(name: str) -> bool:
    """Test fixtures disguise themselves as dispatched methods (`ops.nope`)."""
    return name.rsplit(".", 1)[-1] in FIXTURE_TAILS


def dynamic_calls() -> list[str]:
    """Method names built from template literals at a real call site.

    Only backtick strings handed to a call/invoke helper count. An earlier
    version matched every template literal containing a dot and reported 1,077
    hits — CSS classes and pixel widths are not RPC methods, and a 1,077-line
    "finding" buries the handful that matter.
    """
    hits: list[str] = []
    for path in sorted(WEBUI.rglob("*")):
        if path.suffix not in {".ts", ".vue"} or not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except OSError:
            continue
        for lineno, line in enumerate(text.splitlines(), 1):
            for literal in RPC_CALL.findall(line):
                if "${" not in literal:
                    continue
                skeleton = re.sub(r"\$\{[^}]*\}", "x", literal)
                if METHOD_SHAPE.match(skeleton):
                    hits.append(f"{path.relative_to(ROOT).as_posix()}:{lineno}: {literal}")
    return hits


def main(argv: list[str]) -> int:
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(errors="backslashreplace")

    rpc = rpc_surface()
    events = event_surface()
    dynamic = dynamic_calls()

    if "--format=json" in argv:
        print(json.dumps({"rpc": rpc, "events": events, "dynamic": dynamic}, ensure_ascii=False, indent=2))
    else:
        print(
            f"声明 {rpc['declared_count']} 个方法，分派面命中 {rpc['implemented_count']} 个"
        )
        print(f"\n[A] 有实现未声明（反向漂移，前端永远不调）: {len(rpc['undeclared'])}")
        for name in rpc["undeclared"]:
            print(f"   - {name}")
        print(f"\n[B] 声明但无实现（孤儿声明，一调就失败）: {len(rpc['dead'])}")
        for name in rpc["dead"]:
            print(f"   - {name}")
        sidecar_only = rpc["sidecar_only"]
        print(f"\n[E] 侧车实现但未声明（控制台不可达，侧车/CLI 可达）: {len(sidecar_only)}")
        for name in sidecar_only[:12]:
            print(f"   - {name}")
        if len(sidecar_only) > 12:
            print(f"   ... 另有 {len(sidecar_only) - 12} 个（按前缀分组，非缺陷清单）")
        print(f"\n[C] 前端动态方法名（静态不可判定，需人工确认）: {len(dynamic)}")
        for hit in dynamic[:40]:
            print(f"   - {hit}")
        if len(dynamic) > 40:
            print(f"   ... 另有 {len(dynamic) - 40} 处")
        print(
            f"\n[D] 事件面：前端订阅 {len(events['subscribed'])} 个名字，其中 "
            f"{len(events['no_rust_emitter'])} 个在 core/src 里找不到任何发射点"
        )
        print("    （事件没有单一权威声明表，这一节只是「有人听、没人喊」的反向对账）")
        for name in events["no_rust_emitter"][:25]:
            print(f"   - {name}  ← {events['subscribed'][name][0]}")
        if len(events["no_rust_emitter"]) > 25:
            print(f"   ... 另有 {len(events['no_rust_emitter']) - 25} 个")

    strict_dynamic = "--strict-dynamic" in argv
    failing = bool(rpc["undeclared"]) or bool(rpc["dead"])
    if strict_dynamic and dynamic:
        failing = True
    return 1 if failing else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
