#!/usr/bin/env python3
"""Cache Bench（计划 W5.5）：把「缓存命中率」变成可复现实验，而不是口号。

一条命令：

    python scripts/bench/cache/bench_prefix_cache.py --spawn

或 attach 到已经在跑的网关（必须先按 AGENTS §4 用隔离配置目录起）：

    python scripts/bench/cache/bench_prefix_cache.py --url ws://127.0.0.1:18877/ws

它做四件事：
1. 固定一个会话、固定工具面、12 轮短问（`chat.send` → 等 `session.event.done`）；
2. 每轮前后各读一次 `cache.status`，用**本次窗口的增量**评估，而不是读累计值；
3. 逐轮判定 `cache_read > 0`（非首轮）与 `stable_prefix_drift` 增量；
4. 输出 JSON + Markdown（默认 `artifacts/cache-bench-<UTC 日期>.{json,md}`）。

诚实性红线（不可绕过，见 AGENTS.md §8「跳过 ≠ 通过」）：
- 命中率的**分母**只有两种合法说法：`server_cache_coverage_pct`（可缓存前缀的
  token 覆盖率）与 `server_cache_request_hit_rate_pct`（合格请求命中率）。
  任何「100% 缓存命中」的表述必须带分母，否则本报告拒绝生成结论。
- provider 是本地 mock 或没有 LLM（离线回显）时，`cache_read` 是**伪造不出来也
  测不出来**的：脚本直接判 `NOT PROVIDER EVIDENCE` 并以退出码 2 结束。
  只有显式 `--allow-mock`（用来自检本脚本自身）才会继续，且报告里逐处标注
  「不是上游缓存证据」。
- 本脚本**不进 `verify.sh`**：它需要真实 key 与费用。
"""

from __future__ import annotations

import argparse
import asyncio
import datetime as dt
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

# 控制台代码页在中文 Windows 上是 cp936；报告里有中文标签，直接 print 会炸。
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8", errors="replace")
    except Exception:  # pragma: no cover - 极老的 stdlib / 非 tty
        pass

TERMINAL_EVENTS = ("session.event.done",)
TIMEOUT_S = 180.0

DEFAULT_DENOMINATOR_NOTE = (
    "coverage = Δcache_read / (Δcache_read + Δcache_creation + Δtotal_input)；"
    "request_hit_rate = Δhit_requests / Δeligible_requests（热窗口=第 2..N 轮）。"
    "两者分母不同，不可混称「命中率」。"
)

PROMPTS = [
    "用一句话说明这个项目是什么。",
    "列出三个核心模块。",
    "core 的入口文件是哪个？",
    "webui 用的是什么框架？",
    "门禁脚本叫什么名字？",
    "它有多少项检查？",
    "pyengine 是怎么接入的？",
    "workbench 和引擎是什么关系？",
    "README 的中文版文件名？",
    "AGENTS.md 的铁律是哪三条？",
    "桌面端加载的地址是什么？",
    "最后一问：用一句话总结。",
]


class BenchError(RuntimeError):
    pass


# --------------------------------------------------------------------------- rpc


class WsRpc:
    """最小 WS JSON-RPC 客户端（与 scripts/ws_*.py 同一套线格式）。"""

    def __init__(self, ws: Any) -> None:
        self._ws = ws
        self._rid = 0
        self.events: list[dict] = []

    async def connect(self, guest_key: str) -> dict:
        return await self.call("connect", {"auth": {"guestSessionKey": guest_key}})

    async def call(self, method: str, params: dict) -> dict:
        self._rid += 1
        rid = f"b{self._rid}"
        await self._ws.send(json.dumps({"type": "req", "id": rid, "method": method, "params": params}))
        while True:
            msg = json.loads(await asyncio.wait_for(self._ws.recv(), TIMEOUT_S))
            if msg.get("id") == rid:
                if not msg.get("ok", False):
                    err = msg.get("error") or {}
                    raise BenchError(f"{method} 失败: {err.get('code')} {err.get('message')}")
                return msg.get("payload") or {}
            # 非本请求的帧：服务端推送（session.event.*）
            self.events.append(msg)

    @staticmethod
    def _is_terminal(msg: dict, session_key: str) -> bool:
        if msg.get("type") != "event" or not msg.get("event"):
            return False
        if not any(str(msg["event"]).endswith(e) for e in TERMINAL_EVENTS):
            return False
        meta = msg.get("meta") or {}
        payload = msg.get("payload") or {}
        owner = str(meta.get("session") or payload.get("sessionKey") or payload.get("session") or "")
        # 归属不明的事件保守放行：网关的会话路由键缺失时不该让基准挂死等待，
        # 但报告里保留 seq 以便核对（基准是单会话，串扰即自我污染）。
        return not owner or owner == session_key

    async def wait_for(self, session_key: str) -> None:
        """等待本会话的回合结束事件；连接错误如实抛出，不静默返回。"""
        pending = [m for m in self.events if self._is_terminal(m, session_key)]
        if pending:
            return
        while True:
            msg = json.loads(await asyncio.wait_for(self._ws.recv(), TIMEOUT_S))
            self.events.append(msg)
            if self._is_terminal(msg, session_key):
                return

    async def close(self) -> None:
        await self._ws.close()


async def open_rpc(url: str, guest_key: str) -> WsRpc:
    try:
        import websockets  # noqa: PLC0415
    except ImportError as exc:  # pragma: no cover
        raise BenchError("需要 websockets（pyengine dev 依赖里有）：pip install websockets") from exc
    ws = await websockets.connect(url, open_timeout=8, max_size=None)
    rpc = WsRpc(ws)
    try:
        # 首帧可能是 hello/challenge，先吞掉再正式 connect（与既有探针一致）。
        try:
            await asyncio.wait_for(ws.recv(), 5)
        except (asyncio.TimeoutError, Exception):
            pass
        await rpc.connect(guest_key)
    except BenchError:
        pass  # guest 连接回执缺失也继续：call() 会如实报错
    return rpc


# ------------------------------------------------------------------- providers


def provider_of(config_payload: dict) -> dict:
    llm = config_payload.get("llm") or {}
    model = config_payload.get("model") or {}
    base = str(llm.get("base_url") or llm.get("baseUrl") or model.get("base_url") or "")
    provider = str(llm.get("provider") or model.get("provider") or "")
    name = str(llm.get("model") or model.get("name") or "")
    host = ""
    if "://" in base:
        host = base.split("://", 1)[1].split("/", 1)[0]
    return {"provider": provider, "model": name, "base_url": base, "host": host}


def classify_provider(p: dict) -> tuple[str, str]:
    """返回 (kind, reason)。kind ∈ live|mock|offline。"""
    base = p.get("base_url") or ""
    host = p.get("host") or ""
    if not p.get("model") or not base:
        return "offline", "config.llm 没有 model/base_url —— 引擎会走离线回显，不产生真实上游调用"
    if p.get("model") == "mock-model" or host.startswith("127.0.0.1") or host.startswith("localhost"):
        return "mock", f"provider 指向本地（model={p.get('model')} host={host or base}）—— 上游 KV 缓存不存在"
    return "live", f"{p.get('provider') or '?'} / {p.get('model')} @ {host}"


# ---------------------------------------------------------------------- metrics


def delta(before: dict, after: dict, key: str) -> float:
    return float(after.get(key, 0) or 0) - float(before.get(key, 0) or 0)


def round_metrics(before: dict, after: dict) -> dict:
    read = delta(before, after, "server_cache_read_tokens")
    creation = delta(before, after, "server_cache_creation_tokens")
    total_input = delta(before, after, "server_total_input_tokens")
    eligible = delta(before, after, "server_cache_eligible_requests")
    hits = delta(before, after, "server_cache_hit_requests")
    drift = delta(before, after, "stable_prefix_drift")
    denom = read + creation + total_input
    coverage = (read / denom * 100.0) if denom > 0 else None
    return {
        "cache_read_tokens_delta": read,
        "cache_creation_tokens_delta": creation,
        "total_input_tokens_delta": total_input,
        "eligible_requests_delta": eligible,
        "hit_requests_delta": hits,
        "stable_prefix_drift_delta": drift,
        "window_coverage_pct": round(coverage, 2) if coverage is not None else None,
        "server_cumulative": {
            "server_cache_coverage_pct": after.get("server_cache_coverage_pct"),
            "server_cache_request_hit_rate_pct": after.get("server_cache_request_hit_rate_pct"),
            "server_total_input_tokens": after.get("server_total_input_tokens"),
            "server_cache_read_tokens": after.get("server_cache_read_tokens"),
            "server_cache_creation_tokens": after.get("server_cache_creation_tokens"),
            "server_cache_eligible_requests": after.get("server_cache_eligible_requests"),
            "server_cache_hit_requests": after.get("server_cache_hit_requests"),
            "stable_prefix_drift": after.get("stable_prefix_drift"),
            "stable_prefix_drift_by_scope": after.get("stable_prefix_drift_by_scope"),
        },
    }


def evaluate(turns: list[dict], min_rounds: int) -> dict:
    """按 §7.4 的口径判定；任何一条不满足都要打印定位信息，而不是只给总数。

    分母纪律：第 1 轮是冷启动（允许 read=0），所以**请求命中率只算热窗口
    （第 2..N 轮）**；同时把含冷启动的全窗口数字一并列出，避免两个口径被混用。
    """
    problems: list[str] = []
    if len(turns) < min_rounds:
        problems.append(f"只完成 {len(turns)} 轮（要求 {min_rounds}）")
    for i, t in enumerate(turns):
        if not t["ok"]:
            problems.append(f"第 {i + 1} 轮请求失败：{t.get('error')}")
            continue
        m = t["metrics"]
        if i > 0 and m["cache_read_tokens_delta"] <= 0:
            problems.append(
                f"第 {i + 1} 轮 cache_read=0（非首轮不应为零）"
                f"｜工具数={t.get('tool_count')}｜技能数={t.get('skill_count')}"
                f"｜稳定半区哈希={t.get('stable_hash', '未暴露')}"
            )
        if m["stable_prefix_drift_delta"] > 0:
            problems.append(
                f"第 {i + 1} 轮 stable_prefix_drift +{m['stable_prefix_drift_delta']:.0f}"
                f"｜scope 明细={m['server_cumulative'].get('stable_prefix_drift_by_scope')}"
            )

    def window(rows: list[dict]) -> dict:
        ok = [t for t in rows if t["ok"]]
        eligible = sum(t["metrics"]["eligible_requests_delta"] for t in ok)
        hits = sum(t["metrics"]["hit_requests_delta"] for t in ok)
        read = sum(t["metrics"]["cache_read_tokens_delta"] for t in ok)
        creation = sum(t["metrics"]["cache_creation_tokens_delta"] for t in ok)
        total_in = sum(t["metrics"]["total_input_tokens_delta"] for t in ok)
        denom = read + creation + total_in
        return {
            "rounds": len(ok),
            "eligible_requests": eligible,
            "hit_requests": hits,
            "request_hit_rate_pct": round(hits / eligible * 100.0, 2) if eligible else None,
            "cache_read_tokens": read,
            "cache_creation_tokens": creation,
            "total_input_tokens": total_in,
            "cache_coverage_pct": round(read / denom * 100.0, 2) if denom else None,
        }

    warm = window(turns[1:])
    overall = window(turns)
    drift = sum(t["metrics"]["stable_prefix_drift_delta"] for t in turns if t["ok"])
    if warm["eligible_requests"] and warm["request_hit_rate_pct"] != 100.0:
        problems.append(
            f"热窗口请求命中率 {warm['request_hit_rate_pct']}%"
            f"（{warm['hit_requests']:.0f}/{warm['eligible_requests']:.0f}）≠ 100%"
        )
    return {
        "rounds": len(turns),
        "warm_window": warm,
        "overall_window": overall,
        "request_hit_rate_pct": warm["request_hit_rate_pct"],
        "eligible_requests": warm["eligible_requests"],
        "hit_requests": warm["hit_requests"],
        "cache_coverage_pct": overall["cache_coverage_pct"],
        "cache_read_tokens": overall["cache_read_tokens"],
        "cache_creation_tokens": overall["cache_creation_tokens"],
        "total_input_tokens": overall["total_input_tokens"],
        "stable_prefix_drift": drift,
        "problems": problems,
        "verdict": "PASS" if not problems else "FAIL",
    }


# --------------------------------------------------------------------- gateway


def spawn_gateway(port: int, keep_dir: bool = False) -> tuple[subprocess.Popen, Path]:
    root = Path(__file__).resolve().parents[3]
    exe = next(
        (p for p in (root / "target" / "debug" / "marguerite-core.exe", root / "target" / "release" / "marguerite-core.exe") if p.exists()),
        None,
    )
    if exe is None:
        raise BenchError("找不到 target/{debug,release}/marguerite-core.exe —— 先 cargo build -p marguerite-core --bin marguerite-core")
    home = Path(tempfile.mkdtemp(prefix="marguerite-cache-bench-"))
    local = home / "AppData" / "Local"
    local.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ)
    # Windows 上 dirs::home_dir() 不认 USERPROFILE/HOME（AGENTS §4）—— 必须用显式覆盖 + LOCALAPPDATA。
    env["MARGUERITE_CONFIG_DIR"] = str(home / "marguerite-config")
    env["LOCALAPPDATA"] = str(local)
    proc = subprocess.Popen([str(exe), "gateway", "--port", str(port)], env=env, cwd=str(root))
    ready = False
    for _ in range(120):
        time.sleep(0.5)
        if proc.poll() is not None:
            raise BenchError(f"网关启动即退出 rc={proc.returncode}")
        try:
            import urllib.request

            with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/health", timeout=2) as r:  # noqa: S310 - 回环
                if r.status == 200:
                    ready = True
                    break
        except Exception:
            continue
    if not ready:
        proc.terminate()
        raise BenchError("网关 60s 内没有 /api/health 200")
    if not keep_dir:
        print(f"[OK  ] 隔离网关就绪（配置目录 {home}，结束后可自行删除）")
    return proc, home


# ------------------------------------------------------------------------ main


async def run_bench(args: argparse.Namespace) -> int:
    url = args.url or f"ws://127.0.0.1:{args.port}/ws"
    rpc = await open_rpc(url, args.guest_key)
    try:
        config = await rpc.call("config.get", {})
        prov = provider_of(config)
        kind, reason = classify_provider(prov)
        if kind != "live" and not args.allow_mock:
            print(f"[FAIL] provider={kind}：{reason}")
            print("       上游前缀缓存只存在于真实 provider。要么配置真实 key，")
            print("       要么加 --allow-mock 仅用于自检本脚本（报告会显式标注「不是上游缓存证据」）。")
            return 2
        marker = "OK  " if kind == "live" else "WARN"
        print(f"[{marker}] provider({kind}): {reason}")

        catalog = await rpc.call("tools.catalog", {}) if args.with_tools else {}
        skills = await rpc.call("skills.list", {}) if args.with_tools else {}
        tool_count = len(catalog.get("items") or [])
        skill_count = len(skills.get("skills") or [])
        print(f"[OK  ] 固定面：tools={tool_count} skills={skill_count} session={args.session} turns={args.turns}")

        turns: list[dict] = []
        before = await rpc.call("cache.status", {})
        for i in range(args.turns):
            prompt = PROMPTS[i % len(PROMPTS)]
            t0 = time.time()
            record = {"round": i + 1, "prompt": prompt, "ok": False, "tool_count": tool_count, "skill_count": skill_count}
            try:
                await rpc.call("chat.send", {"message": prompt, "sessionKey": args.session})
                await rpc.wait_for(args.session)
                after = await rpc.call("cache.status", {})
                record.update(ok=True, metrics=round_metrics(before, after), elapsed_s=round(time.time() - t0, 2))
                before = after
                m = record["metrics"]
                print(
                    f"[OK  ] 第 {i + 1} 轮 read=+{m['cache_read_tokens_delta']:.0f} "
                    f"creation=+{m['cache_creation_tokens_delta']:.0f} "
                    f"window_coverage={m['window_coverage_pct']}% "
                    f"drift=+{m['stable_prefix_drift_delta']:.0f} ({record['elapsed_s']}s)"
                )
            except BenchError as err:
                record["error"] = str(err)
                record["elapsed_s"] = round(time.time() - t0, 2)
                print(f"[FAIL] 第 {i + 1} 轮：{err}")
                break
            turns.append(record)

        summary = evaluate(turns, min_rounds=args.turns)
        summary["provider_kind"] = kind
        summary["provider_reason"] = reason
        summary["is_provider_evidence"] = kind == "live"
        summary["denominator_note"] = DEFAULT_DENOMINATOR_NOTE
        report = {
            "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
            "gateway": url,
            "session": args.session,
            "config": {"tools": tool_count, "skills": skill_count, "turns": args.turns},
            "rounds": turns,
            "summary": summary,
        }
        out = Path(args.out_dir)
        out.mkdir(parents=True, exist_ok=True)
        stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%d")
        json_path = out / f"cache-bench-{stamp}.json"
        md_path = out / f"cache-bench-{stamp}.md"
        json_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        md_path.write_text(render_markdown(report), encoding="utf-8")
        print(f"[OK  ] 报告：{json_path}")
        print(f"[OK  ] 报告：{md_path}")
        verdict = summary["verdict"] if kind == "live" else f"{verdict} (mock —— 不是上游缓存证据)"
        print(f"结论：{verdict}")
        for p in summary["problems"]:
            print(f"  - {p}")
        return 0 if summary["verdict"] == "PASS" else 1
    finally:
        await rpc.close()


def render_markdown(report: dict) -> str:
    s = report["summary"]
    w = s["warm_window"]
    o = s["overall_window"]
    denom = o["cache_read_tokens"] + o["cache_creation_tokens"] + o["total_input_tokens"]
    lines = [
        f"# Cache Bench {report['generated_at']}",
        "",
        f"- 网关：`{report['gateway']}` 会话：`{report['session']}`",
        f"- provider：**{s.get('provider_kind', '未标注')}** —— {s.get('provider_reason', '未标注')}",
        f"- 是否构成上游缓存证据：**{'是' if s.get('is_provider_evidence') else '否'}**",
        f"- 口径：{s.get('denominator_note', DEFAULT_DENOMINATOR_NOTE)}",
        "",
        "| 轮 | ok | Δread | Δcreation | Δinput | 窗口 coverage | Δdrift | 耗时 |",
        "|---:|:--|--:|--:|--:|--:|--:|--:|",
    ]
    for t in report["rounds"]:
        m = t.get("metrics") or {}
        lines.append(
            f"| {t['round']} | {'y' if t['ok'] else 'n'} | {m.get('cache_read_tokens_delta', '-')} | "
            f"{m.get('cache_creation_tokens_delta', '-')} | {m.get('total_input_tokens_delta', '-')} | "
            f"{m.get('window_coverage_pct', '-')} | {m.get('stable_prefix_drift_delta', '-')} | {t.get('elapsed_s', '-')} |"
        )
    lines += [
        "",
        "## 汇总",
        "",
        "热窗口 = 第 2..N 轮（第 1 轮是冷启动，允许 read=0，不进命中分母）。",
        "",
        f"- request_hit_rate（热窗口）：**{w['request_hit_rate_pct']}%**"
        f"（{w['hit_requests']:.0f}/{w['eligible_requests']:.0f} 合格请求）",
        f"- request_hit_rate（含冷启动）：{o['request_hit_rate_pct']}%"
        f"（{o['hit_requests']:.0f}/{o['eligible_requests']:.0f}）",
        f"- coverage（全窗口）：**{o['cache_coverage_pct']}%**"
        f"（read {o['cache_read_tokens']:.0f} / read+creation+input {denom:.0f}）",
        f"- stable_prefix_drift：**{s['stable_prefix_drift']:.0f}**",
        f"- 判定：**{s['verdict']}**",
        "",
    ]
    if s["problems"]:
        lines.append("## 问题定位")
        lines.append("")
        lines += [f"- {p}" for p in s["problems"]]
    return "\n".join(lines) + "\n"


def self_test() -> int:
    """离线自检：不需要网关、不产生任何「缓存命中」宣称，只验证判定逻辑本身。"""

    def st(read, creation, total, eligible, hits, drift):
        return {
            "server_cache_read_tokens": read,
            "server_cache_creation_tokens": creation,
            "server_total_input_tokens": total,
            "server_cache_eligible_requests": eligible,
            "server_cache_hit_requests": hits,
            "stable_prefix_drift": drift,
            "server_cache_coverage_pct": 0,
            "server_cache_request_hit_rate_pct": 0,
            "stable_prefix_drift_by_scope": {},
        }

    def turns_of(rows):
        out, before = [], st(0, 0, 0, 0, 0, 0)
        for i, after in enumerate(rows):
            m = round_metrics(before, after)
            out.append({"round": i + 1, "ok": True, "metrics": m, "tool_count": 19, "skill_count": 3, "stable_hash": "-"})
            before = after
        return out

    assert classify_provider({"provider": "deepseek", "model": "deepseek-chat", "base_url": "https://api.deepseek.com/v1", "host": "api.deepseek.com"})[0] == "live"
    assert classify_provider({"provider": "x", "model": "mock-model", "base_url": "http://127.0.0.1:18999/v1", "host": "127.0.0.1:18999"})[0] == "mock"
    assert classify_provider({"provider": "", "model": "", "base_url": "", "host": ""})[0] == "offline"
    print("[OK] provider 三档分类（live/mock/offline）")

    good = turns_of([st(0, 1200, 800, 1, 0, 0), st(1500, 100, 900, 2, 1, 0), st(3000, 0, 900, 3, 2, 0)])
    s = evaluate(good, 3)
    assert s["verdict"] == "PASS", s
    assert s["request_hit_rate_pct"] == 100.0 and s["eligible_requests"] == 2, s
    assert s["overall_window"]["eligible_requests"] == 3, s
    assert good[0]["metrics"]["window_coverage_pct"] == 0.0, good[0]
    print("[OK] 热窗口分母（第 1 轮冷启动不进命中分母，全窗口数字并列）")

    stalled = turns_of([st(0, 1200, 800, 1, 0, 0), st(0, 1200, 800, 1, 0, 0)])
    s = evaluate(stalled, 2)
    assert s["verdict"] == "FAIL" and any("第 2 轮 cache_read=0" in p for p in s["problems"]), s
    print("[OK] 非首轮 read=0 判红并带工具/技能定位")

    drifted = turns_of([st(0, 1200, 800, 1, 0, 0), st(1500, 100, 900, 2, 1, 1)])
    s = evaluate(drifted, 2)
    assert s["verdict"] == "FAIL" and any("stable_prefix_drift" in p for p in s["problems"]), s
    print("[OK] 稳定半区漂移判红")

    short = turns_of([st(0, 1200, 800, 1, 0, 0), st(1500, 100, 900, 2, 1, 0)])
    assert evaluate(short, 3)["verdict"] == "FAIL"
    rep = {"generated_at": "x", "gateway": "ws://x", "session": "s", "rounds": short, "summary": evaluate(short, 2)}
    assert "热窗口" in render_markdown(rep)
    print("[OK] 轮次不足判红 + markdown 可渲染")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description="Cache Bench（W5.5）：可复现的前缀缓存实验")
    ap.add_argument("--url", help="已有网关的 ws 地址；省略则用 --port")
    ap.add_argument("--port", type=int, default=18911)
    ap.add_argument("--spawn", action="store_true", help="自己起一个隔离网关（临时 MARGUERITE_CONFIG_DIR + LOCALAPPDATA）")
    ap.add_argument("--guest-key", default="cache-bench")
    ap.add_argument("--session", default=f"bench.cache.{int(time.time())}")
    ap.add_argument("--turns", type=int, default=12)
    ap.add_argument("--out-dir", default="artifacts")
    ap.add_argument("--allow-mock", action="store_true", help="允许 mock/offline 仅用于自检脚本（报告标注非证据）")
    ap.add_argument("--with-tools", action="store_true", help="额外读 tools.catalog / skills.list 以记录固定面")
    ap.add_argument("--self-test", action="store_true", help="离线自检判定逻辑（不连网关、不产生任何缓存宣称）")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    proc = None
    home = None
    try:
        if args.spawn:
            proc, home = spawn_gateway(args.port)
        return asyncio.run(run_bench(args))
    except BenchError as err:
        print(f"[FAIL] {err}")
        return 2
    finally:
        if proc is not None:
            proc.terminate()
            try:
                proc.wait(10)
            except subprocess.TimeoutExpired:
                proc.kill()
        if home is not None and os.environ.get("CACHE_BENCH_KEEP_TMP") != "1":
            shutil.rmtree(home, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
