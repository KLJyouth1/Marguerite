#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""专家定义自洽性检查（RSI 断点 B5：双真相源）。

背景：8 个专家团 / 43 名成员同时存在于两处 ——
  1. 仓库内 `workbench/backend/marguerite_workbench/expert_teams.py`（TEAM_DEFS + AGENT_SPECS）
  2. WorkBuddy 客户端插件 `my-experts/plugins/<pkg>/agents/*.md`
两处独立维护必然漂移（2026-10-26 已实证：改了客户端 8 包，仓库内 43 角色零变化）。

本脚本两种模式：
  默认            只查仓库内自洽（CI 可跑，不依赖客户端、不 import、不执行任何代码）
  --client-dir D  额外比对客户端插件（本地开发机跑）

退出码：0 = 通过；1 = 不一致；2 = 无法验证（文件缺失 / 解析失败，绝不冒充通过）。
"""

from __future__ import annotations

import argparse
import ast
import io
import os
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC_FILE = os.path.join(
    REPO, "workbench", "backend", "marguerite_workbench", "expert_teams.py"
)
EXPECTED_TEAMS = 8
EXPECTED_ROLES = 43


def fail(msg: str) -> None:
    print("  [FAIL] " + msg)


def ok(msg: str) -> None:
    print("  [PASS] " + msg)


def _const(node):
    """取 ast 常量的值；不是常量返回 None。"""
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return node.value
    return None


def parse_specs(path: str):
    """静态解析 expert_teams.py，返回 (team_defs, agent_specs, a_param_order)。

    不 import、不执行 —— 避免 CI 环境缺 pydantic 时把「跑不了」误报成「不一致」。
    """
    src = io.open(path, encoding="utf-8").read()
    tree = ast.parse(src, filename=path)

    team_defs = {}          # team_key -> {lead_role, name}
    agent_specs = []        # [{role, team, kind}]
    a_order = None          # A() 的位置参数顺序

    for node in tree.body:
        # def A(role, team, ...) —— 确认位置参数顺序，防止解析假设失效
        if isinstance(node, ast.FunctionDef) and node.name == "A":
            a_order = [a.arg for a in node.args.args]
            continue

        # 兼容两种写法：`X = ...` 与 `X: dict[...] = ...`（本仓用的是后者）
        if isinstance(node, ast.Assign):
            targets, value = node.targets, node.value
        elif isinstance(node, ast.AnnAssign) and node.value is not None:
            targets, value = [node.target], node.value
        else:
            continue

        if len(targets) != 1 or not isinstance(targets[0], ast.Name):
            continue
        var = targets[0].id

        # TEAM_DEFS = { ... }
        if var == "TEAM_DEFS" and isinstance(value, ast.Dict):
            for k, v in zip(value.keys, value.values):
                key = _const(k)
                if key is None or not isinstance(v, ast.Dict):
                    continue
                entry = {}
                for kk, vv in zip(v.keys, v.values):
                    name = _const(kk)
                    if name:
                        entry[name] = _const(vv)
                team_defs[key] = entry

        # AGENT_SPECS = [ A(...), ... ]
        elif var == "AGENT_SPECS" and isinstance(value, ast.List):
            for el in value.elts:
                if not isinstance(el, ast.Call):
                    continue
                fn = el.func
                fname = fn.id if isinstance(fn, ast.Name) else None
                if fname != "A":
                    continue
                role = _const(el.args[0]) if len(el.args) > 0 else None
                team = _const(el.args[1]) if len(el.args) > 1 else None
                kind = _const(el.args[3]) if len(el.args) > 3 else None
                agent_specs.append(
                    {"role": role, "team": team, "kind": kind, "line": el.lineno}
                )

    return team_defs, agent_specs, a_order


def check_repo(team_defs, agent_specs, a_order) -> int:
    bad = 0

    if a_order is None:
        print("  [SKIP] 未找到 def A(...)，无法确认参数顺序 —— 拒绝判定")
        return 2
    if len(a_order) < 4 or a_order[0] != "role" or a_order[1] != "team" or a_order[3] != "kind":
        fail("A() 参数顺序变了（期望 role, team, …, kind），解析假设失效：%s" % a_order)
        return 2
    ok("A() 参数顺序符合假设：%s" % ", ".join(a_order[:4]))

    # 1) 数量
    if len(team_defs) != EXPECTED_TEAMS:
        fail("TEAM_DEFS 数量 %d，期望 %d" % (len(team_defs), EXPECTED_TEAMS))
        bad += 1
    else:
        ok("TEAM_DEFS = %d 个团队" % len(team_defs))

    if len(agent_specs) != EXPECTED_ROLES:
        fail("AGENT_SPECS 数量 %d，期望 %d" % (len(agent_specs), EXPECTED_ROLES))
        bad += 1
    else:
        ok("AGENT_SPECS = %d 个角色" % len(agent_specs))

    # 2) 字段完整
    for s in agent_specs:
        if not s["role"] or not s["team"] or not s["kind"]:
            fail("第 %d 行的 A(...) 缺 role/team/kind" % s["line"])
            bad += 1
    if bad == 0:
        ok("全部角色都带 role / team / kind")

    # 3) role 唯一
    seen = {}
    for s in agent_specs:
        r = s["role"]
        if r in seen:
            fail("role 重复：%s（第 %d 行与第 %d 行）" % (r, seen[r], s["line"]))
            bad += 1
        seen[r] = s["line"]
    if bad == 0:
        ok("role 全局唯一")

    # 4) 每个角色的 team 必须存在
    for s in agent_specs:
        if s["team"] not in team_defs:
            fail("角色 %s 的 team=%s 不在 TEAM_DEFS" % (s["role"], s["team"]))
            bad += 1

    # 5) 每个团队的 lead_role 必须存在且 kind=lead，且恰好一个 lead
    by_team = {}
    for s in agent_specs:
        by_team.setdefault(s["team"], []).append(s)
    for key, meta in team_defs.items():
        members = by_team.get(key, [])
        if not members:
            fail("团队 %s 没有任何角色" % key)
            bad += 1
            continue
        leads = [m for m in members if m["kind"] == "lead"]
        if len(leads) != 1:
            fail("团队 %s 的 lead 数量为 %d，期望恰好 1" % (key, len(leads)))
            bad += 1
        lead_role = meta.get("lead_role")
        if not lead_role:
            fail("团队 %s 的 TEAM_DEFS 缺 lead_role" % key)
            bad += 1
        elif lead_role not in seen:
            fail("团队 %s 的 lead_role=%s 不在 AGENT_SPECS" % (key, lead_role))
            bad += 1
        elif leads and leads[0]["role"] != lead_role:
            fail(
                "团队 %s 的 lead_role=%s 与实际 lead=%s 不一致"
                % (key, lead_role, leads[0]["role"])
            )
            bad += 1
        if len(members) < 2:
            fail("团队 %s 只有 %d 个角色（至少 1 lead + 1 member）" % (key, len(members)))
            bad += 1

    if bad == 0:
        ok("每个团队的 lead 唯一、lead_role 对齐、成员数 ≥ 2")

    return 0 if bad == 0 else 1


def _normalize_role(r: str) -> str:
    """把两侧的命名约定归一，再比对。

    已知约定差异（**不是漂移**）：
      - 客户端主理人按 expert-manager 规范必须命名 `<pkg>-team-lead.md`
      - 仓库内命名为 `<team>_lead`
    若不归一，8 个主理人会永远显示为「两边各缺 8 个」，把真问题淹掉。
    """
    r = r.replace("-", "_")
    if r.endswith("_team_lead"):
        r = r[: -len("_team_lead")] + "_lead"
    return r


def check_client(client_dir: str, agent_specs, team_keys) -> int:
    """比对客户端插件与仓库内定义。role 命名归一化：kebab-case → snake_case。

    只统计 `team_keys` 对应的 8 个团队包 —— 客户端还装了 `senior-dev-engineer`
    这类单专家包，它们不属于「8 团 43 成员」，计进来会造成假差异。
    """
    if not os.path.isdir(client_dir):
        print("  [SKIP] 客户端目录不存在：%s" % client_dir)
        return 2

    team_pkgs = {k.replace("_", "-") for k in team_keys}
    repo_roles = {_normalize_role(s["role"]) for s in agent_specs}
    client_roles = set()
    pkgs = 0
    skipped = []
    for name in sorted(os.listdir(client_dir)):
        pkg = os.path.join(client_dir, name)
        agents = os.path.join(pkg, "agents")
        if not os.path.isdir(agents):
            continue
        if name not in team_pkgs:
            skipped.append(name)
            continue
        pkgs += 1
        for f in os.listdir(agents):
            if f.endswith(".md"):
                client_roles.add(_normalize_role(f[:-3]))

    print("  客户端团队包数：%d（跳过非团队包：%s）" % (pkgs, ", ".join(skipped) or "无"))
    print("  客户端角色数：%d，仓库内角色数：%d" % (len(client_roles), len(repo_roles)))
    only_repo = sorted(repo_roles - client_roles)
    only_client = sorted(client_roles - repo_roles)

    if only_repo:
        print("  [WARN] 仅存在于仓库内（客户端缺）：%s" % ", ".join(only_repo[:10]))
    if only_client:
        print("  [WARN] 仅存在于客户端（仓库缺）：%s" % ", ".join(only_client[:10]))

    if not only_repo and not only_client:
        ok("客户端与仓库内角色完全一致")
        return 0
    return 1


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--client-dir", default=None, help="my-experts/plugins 路径（本地比对用）")
    args = ap.parse_args()

    print("== 专家定义自洽性检查 ==")
    if not os.path.isfile(SPEC_FILE):
        print("  [SKIP] 找不到 %s —— 无法验证" % SPEC_FILE)
        return 2
    print("  源文件：%s" % SPEC_FILE)

    try:
        team_defs, agent_specs, a_order = parse_specs(SPEC_FILE)
    except SyntaxError as e:
        print("  [SKIP] 解析失败（语法错误）：%s —— 无法验证" % e)
        return 2

    print("\n-- 仓库内自洽 --")
    rc = check_repo(team_defs, agent_specs, a_order)

    if args.client_dir:
        print("\n-- 客户端比对 --")
        rc2 = check_client(args.client_dir, agent_specs, list(team_defs.keys()))
        if rc2 == 1:
            rc = 1
        elif rc2 == 2 and rc == 0:
            rc = 2

    print("\nRESULT: %s" % {0: "PASS", 1: "FAIL", 2: "UNVERIFIABLE"}[rc])
    return rc


if __name__ == "__main__":
    sys.exit(main())
