#!/usr/bin/env python3
"""Validate ``scripts/bench/capability/tasks.jsonl`` (W4.7 Capability Bench 60).

Standard library only -- no third-party dependencies, no network, no clock.

Checks
------
* UTF-8 (no BOM), 60 non-empty JSONL lines, no ``#`` comment lines.
* Required fields ``{id, prompt, expected_family, allowed_tools, forbidden_tools}``
  and the optional ``workspace_fixture``; unknown fields are rejected.
* Exactly 60 unique ids with the stable ``<prefix>-NN`` shape and 1..N numbering.
* Exact family coverage: file 10 / shell 5 / browser 10 / desktop 10
  (desktop.uia 7 + desktop.pixel 3) / memory 5 / kb 5 / skill 5 / mcp 5 /
  cron 3 / team 2.
* Prompts are ASCII English (no CJK) and at least 20 characters long.
* Tool names are real: they must come from ``capability_cards::cards()`` /
  ``tool_schema::FIXED_FACE`` (mirrored below), and each task may only allow
  tools belonging to its own capability family.
* ``allowed_tools`` and ``forbidden_tools`` are disjoint; non-shell families
  must forbid the script substitution (``shell_exec`` / ``code_exec``);
  the shell family must not forbid its own tools.
* ``mcp`` / ``team`` have no model-side tool (``allowed_tools == []``);
  MCP tool names are dynamic (``sanitize_mcp_tool_name``) and therefore cannot
  be enumerated statically.

Usage
-----
    python scripts/bench/capability/validate_tasks.py [path/to/tasks.jsonl]

Exit codes: 0 = clean, 1 = violations (printed), 2 = cannot read/parse the file.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

EXPECTED_TOTAL = 60
EXPECTED_BY_FAMILY = {
    "file": 10,
    "shell": 5,
    "browser": 10,
    "desktop.uia": 7,
    "desktop.pixel": 3,
    "memory": 5,
    "kb": 5,
    "skill": 5,
    "mcp": 5,
    "cron": 3,
    "team": 2,
}
# id prefix -> family. "desktop" covers both desktop.uia and desktop.pixel.
PREFIX_TO_FAMILY = {
    "file": "file",
    "shell": "shell",
    "browser": "browser",
    "desktop": "desktop",
    "memory": "memory",
    "kb": "kb",
    "skill": "skill",
    "mcp": "mcp",
    "cron": "cron",
    "team": "team",
}

# Mirrors of the real tool names (core/src/agent/capability_cards.rs::cards()
# + core/src/agent/tool_schema.rs::FIXED_FACE). Keep in sync with the Rust
# integration test, which reads the constants themselves.
FIXED_FACE = {
    "read_file",
    "list_dir",
    "str_replace_editor",
    "write_file",
    "apply_patch",
    "grep",
    "web_search",
    "web_fetch",
    "browser",
    "memory_search",
    "kb_search",
    "list_skills",
    "use_skill",
    "desktop_launch",
    "desktop_action",
    "computer_use",
    "shell_exec",
    "code_exec",
}
CARD_TOOLS = {
    "read_file", "write_file", "str_replace_editor", "apply_patch",
    "list_dir", "grep", "shell_exec", "code_exec", "browser",
    "desktop_launch", "desktop_action", "computer_use",
    "memory_search", "memory_save", "kb_search", "kb_ingest",
    "search_skills", "list_skills", "use_skill", "automation",
}
KNOWN_TOOLS = FIXED_FACE | CARD_TOOLS
SCRIPT_TOOLS = {"shell_exec", "code_exec"}

FAMILY_TOOLS = {
    "file": {"read_file", "write_file", "str_replace_editor", "apply_patch", "list_dir", "grep"},
    "shell": {"shell_exec", "code_exec"},
    "browser": {"browser"},
    "desktop.uia": {"desktop_launch", "desktop_action"},
    "desktop.pixel": {"computer_use"},
    "memory": {"memory_search", "memory_save"},
    "kb": {"kb_search", "kb_ingest"},
    "skill": {"search_skills", "list_skills", "use_skill"},
    "mcp": set(),
    "cron": {"automation"},
    "team": set(),
}

REQUIRED_FIELDS = {"id", "prompt", "expected_family", "allowed_tools", "forbidden_tools"}
OPTIONAL_FIELDS = {"workspace_fixture"}
ID_RE = re.compile(r"^(file|shell|browser|desktop|memory|kb|skill|mcp|cron|team)-(\d{2})$")

errors: list[str] = []


def fail(message: str) -> None:
    errors.append(message)


def _string_list(value: object, field: str, task_id: str) -> list[str]:
    if not isinstance(value, list):
        fail(f"{task_id}: field '{field}' must be a list")
        return []
    out: list[str] = []
    for item in value:
        if not isinstance(item, str) or not item.strip():
            fail(f"{task_id}: field '{field}' contains a non-string or empty entry")
            continue
        out.append(item)
    if len(out) != len(set(out)):
        fail(f"{task_id}: field '{field}' contains duplicates")
    return out


def validate(path: Path) -> dict[str, int]:
    try:
        raw = path.read_bytes()
    except OSError as exc:
        print(f"[FAIL] cannot read {path}: {exc}")
        raise SystemExit(2)

    if raw.startswith(b"\xef\xbb\xbf"):
        fail("file starts with a UTF-8 BOM (must be plain UTF-8)")
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as exc:
        print(f"[FAIL] {path} is not valid UTF-8: {exc}")
        raise SystemExit(2)

    lines = text.splitlines()
    if len(lines) == 0:
        fail("file is empty")
    for number, line in enumerate(lines, start=1):
        if not line.strip():
            fail(f"line {number}: blank line (JSONL must not contain blank lines)")
        if line.lstrip().startswith("#"):
            fail(f"line {number}: comment line (JSONL does not support comments)")
    if len(lines) != EXPECTED_TOTAL:
        fail(f"expected {EXPECTED_TOTAL} lines, found {len(lines)}")

    ids: list[str] = []
    suffixes: dict[str, list[int]] = {}
    family_counts: dict[str, int] = {family: 0 for family in EXPECTED_BY_FAMILY}

    for number, line in enumerate(lines, start=1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        try:
            task = json.loads(line)
        except json.JSONDecodeError as exc:
            fail(f"line {number}: invalid JSON: {exc}")
            continue
        if not isinstance(task, dict):
            fail(f"line {number}: each line must be a JSON object")
            continue

        keys = set(task)
        missing = REQUIRED_FIELDS - keys
        if missing:
            fail(f"line {number}: missing required field(s): {', '.join(sorted(missing))}")
            continue
        unknown = keys - REQUIRED_FIELDS - OPTIONAL_FIELDS
        if unknown:
            fail(f"line {number}: unknown field(s): {', '.join(sorted(unknown))}")

        task_id = task.get("id")
        if not isinstance(task_id, str) or not task_id:
            fail(f"line {number}: 'id' must be a non-empty string")
            continue
        if task_id in ids:
            fail(f"line {number}: duplicate id '{task_id}'")
        ids.append(task_id)

        match = ID_RE.match(task_id)
        if not match:
            fail(f"{task_id}: id must match '<family-prefix>-NN' (two digits)")
        else:
            prefix, suffix = match.group(1), int(match.group(2))
            suffixes.setdefault(prefix, []).append(suffix)

        family = task.get("expected_family")
        if family not in EXPECTED_BY_FAMILY:
            fail(f"{task_id}: unknown expected_family {family!r}")
            continue
        family_counts[family] += 1
        if match:
            prefix_family = PREFIX_TO_FAMILY[match.group(1)]
            if prefix_family == "desktop":
                if family not in {"desktop.uia", "desktop.pixel"}:
                    fail(f"{task_id}: 'desktop' prefix must map to desktop.uia/desktop.pixel, got {family}")
            elif prefix_family != family:
                fail(f"{task_id}: id prefix implies {prefix_family}, expected_family is {family}")

        prompt = task.get("prompt")
        if not isinstance(prompt, str) or not prompt.strip():
            fail(f"{task_id}: 'prompt' must be a non-empty string")
        else:
            if not prompt.isascii():
                fail(f"{task_id}: prompt must be English (found non-ASCII characters)")
            if len(prompt) < 20:
                fail(f"{task_id}: prompt is too short to be a realistic request")
            if prompt != prompt.strip():
                fail(f"{task_id}: prompt has leading/trailing whitespace")

        allowed = _string_list(task.get("allowed_tools"), "allowed_tools", task_id)
        forbidden = _string_list(task.get("forbidden_tools"), "forbidden_tools", task_id)

        for name in allowed + forbidden:
            if name not in KNOWN_TOOLS:
                fail(f"{task_id}: '{name}' is not a real registered tool name")
        for name in allowed:
            if name not in FAMILY_TOOLS[family]:
                fail(f"{task_id}: allowed tool '{name}' does not belong to family {family}")
        for name in forbidden:
            if name not in SCRIPT_TOOLS:
                fail(f"{task_id}: forbidden tool '{name}' is not a script-substitution tool")

        overlap = set(allowed) & set(forbidden)
        if overlap:
            fail(f"{task_id}: tools both allowed and forbidden: {', '.join(sorted(overlap))}")

        if family == "shell":
            if forbidden:
                fail(f"{task_id}: shell tasks must not forbid their own script tools")
        else:
            if "shell_exec" not in forbidden:
                fail(f"{task_id}: non-shell task must forbid shell_exec as a script substitution")
        if family in {"mcp", "team"}:
            if allowed:
                fail(f"{task_id}: {family} has no model-side tool; allowed_tools must be []")
        elif not allowed:
            fail(f"{task_id}: allowed_tools must not be empty for family {family}")
        if family == "desktop.uia" and not {"desktop_launch", "desktop_action"} <= set(allowed):
            fail(f"{task_id}: desktop.uia requires desktop_launch + desktop_action together")
        if family == "desktop.pixel" and allowed != ["computer_use"]:
            fail(f"{task_id}: desktop.pixel allows exactly ['computer_use']")

        if "workspace_fixture" in task:
            fixture = task["workspace_fixture"]
            if not isinstance(fixture, str) or not fixture.strip():
                fail(f"{task_id}: workspace_fixture must be a non-empty string")
            elif Path(fixture).is_absolute() or ".." in Path(fixture).parts:
                fail(f"{task_id}: workspace_fixture must be a relative path inside the bench dir")
            elif not fixture.isascii():
                fail(f"{task_id}: workspace_fixture must be ASCII")

    for family, expected in EXPECTED_BY_FAMILY.items():
        actual = family_counts[family]
        if actual != expected:
            fail(f"family {family}: expected {expected} tasks, found {actual}")
    total = sum(family_counts.values())
    if total != EXPECTED_TOTAL:
        fail(f"family counts add up to {total}, expected {EXPECTED_TOTAL}")

    for prefix, values in suffixes.items():
        ordered = sorted(values)
        if ordered != list(range(1, len(ordered) + 1)):
            fail(f"{prefix}: ids must be numbered 01..{len(ordered):02d} without gaps, got {ordered}")

    return family_counts


def main(argv: list[str]) -> int:
    if len(argv) > 2:
        print("usage: validate_tasks.py [tasks.jsonl]")
        return 2
    path = Path(argv[1]) if len(argv) == 2 else Path(__file__).with_name("tasks.jsonl")
    if not path.is_file():
        print(f"[FAIL] tasks file not found: {path}")
        return 2

    counts = validate(path)
    if errors:
        for message in errors:
            print(f"[FAIL] {message}")
        print(f"[FAIL] {len(errors)} violation(s) in {path}")
        return 1

    summary = ", ".join(f"{family}={counts[family]}" for family in EXPECTED_BY_FAMILY)
    print(f"[PASS] {path}: {EXPECTED_TOTAL} tasks, unique ids, exact coverage ({summary})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
