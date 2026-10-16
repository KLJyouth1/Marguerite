# Contributing to Marguerite

Thanks for considering a contribution. Marguerite is a terminal-first agent
workbench: **one Rust engine, three front ends** (ratatui TUI, Tauri desktop,
Vue 3 console) with a Python sidecar. This document is the short path to a
mergeable pull request.

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).
Security issues must **not** go through the public tracker — see
[SECURITY.md](SECURITY.md).

> 中文附注：本文以英文为准；每个关键小节末尾附中文要点。核心原则是「证据优先」——
> 任何「完成 / 通过 / 已修复」的声明都必须附带当次真实运行的命令输出。

---

## 1. Development environment

| Requirement | Version | Why |
|---|---|---|
| Rust | **1.95.0** | `rust-toolchain.toml` pins it. `yrs 0.27.3` uses the `if let` match-arm guard (RFC 2294) which only stabilised in 1.95.0; older toolchains fail with `E0658`. |
| Node.js | **22.12.0 or newer** | Declared in `webui/package.json` → `engines.node` (Vite 8 / Vitest 4). The minimum is Node v22.12 — Node 20 is **not** sufficient. Pinned once in `webui/.node-version`, which CI reads via `node-version-file`. |
| Python | 3.x (standard library only) for the gates; **3.11+ with `pytest` for `pyengine`** | The repo gates (`scripts/check-doc-cli-drift.py`, `third_party/validate_integration.py`) import nothing outside the stdlib, so no Python is needed for the Rust or front-end stacks. The `--pyengine` layer additionally needs `pytest` + `pytest-asyncio`; without them that layer reports `[SKIP]` and is counted as **not verified** rather than silently passing. |
| Tauri desktop build | Tauri 2 prerequisites for your OS | Only needed if you touch `gui/`. On Debian/Ubuntu: `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev` + `build-essential pkg-config`. |

Install the pinned Node major (Debian/Ubuntu, via NodeSource):

```bash
curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash -
sudo apt-get install -y nodejs
```

`rust-toolchain.toml` pins the *full msvc host triple*
(`1.95.0-x86_64-pc-windows-msvc`) because the reference development machines are
Windows. On Linux/macOS either install the host-native twin and export
`RUSTUP_TOOLCHAIN`, or edit the channel locally — **do not commit a host-specific
channel**; the CI workflow shows the supported override.

> 中文：Rust 必须是 1.95.0（`rust-toolchain.toml` 锁死，低于此版本编译 `yrs` 会报
> `E0658`）；Node 必须 ≥ 22.12.0（不是 20）。工具链文件里锁的是 Windows msvc 三元组，
> Linux/macOS 上用 `RUSTUP_TOOLCHAIN` 覆盖，不要把本机三元组提交进仓库。

---

## 2. Build and test

```bash
# Rust — the CLI binary and the library unit tests
cargo build -p marguerite-core --bin marguerite-core
cargo test  -p marguerite-core --lib

# Rust — whole workspace (core + tui + the Tauri shell)
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings

# Documentation ↔ CLI drift gate (pure Python, no dependencies)
python3 scripts/check-doc-cli-drift.py

# Web console (run inside webui/)
npm ci
npm run check:architecture   # 10 guard scripts: colors/motion/radius/theme-contract/i18n/…
npm run typecheck            # check:architecture + vue-tsc --noEmit
npm run test:unit            # Vitest
npm run build                # typecheck + vite build + artifact validation
```

The current test baseline is recorded in [AGENTS.md](AGENTS.md) §2. Do not
regress it; if the baseline legitimately changes, update that section in the same
pull request.

> 中文：以上是权威命令。测试基线写在 `AGENTS.md` §2，改动后不得回退。

---

## 3. The verification gate (required before you claim "done")

Everything above is collapsed into one mechanical gate:

```bash
bash scripts/verify.sh --all              # everyday full gate (16 checks)
bash scripts/verify.sh --clean --all      # REQUIRED for a "done" claim (clears cargo cache first)
bash scripts/verify.sh --release          # adds `cargo tauri build` — release gate
bash scripts/verify.sh --fast             # pre-commit: core tests + clippy + webui typecheck
bash scripts/verify.sh --pyengine         # Python sidecar only (tests/unit, no network, no LLM)
```

**Iron law: no completion claims without fresh verification evidence.** A `[FAIL]`
line, or quoting an earlier green run instead of this one, means the work is not
done. `AGENTS.md` §8 lists the banned phrasings ("should be fine", "looks like it
passed", …).

A `[SKIP]` is **not** a pass. The gate keeps skipped checks in a separate list and
prints them under "跳过 — 未验证，不计入通过", so a missing toolchain can never be
mistaken for a green result.

Environment traps that make a green run red for no reason:

* `NO_COLOR=1` makes two pre-existing theme tests take the monochrome fallback and
  fail. `verify.sh` unsets it; if you run `cargo test` by hand, unset it too.
* Stale build artifacts hide library-level compile errors. That is exactly why the
  authoritative run uses `--clean`.

One more, specific to a non-UTF-8 locale (Chinese Windows defaults to GBK/cp936):
Python's `Path.read_text()` with no `encoding=` uses the *locale* encoding, so any
`pyengine` test that reads a UTF-8 source file raises `UnicodeDecodeError`. This is
red on a Windows dev box and green on Linux CI. Prefer `read_text(encoding="utf-8")`
in new tests. Note that a few existing tests set `gbk`/`latin-1`/`cp1252`
*deliberately*, so do not bulk-rewrite them.

> 中文：`bash scripts/verify.sh --clean --all` 全绿（每条 `[PASS]`、退出码 0）之前，
> 不要声称完成。手工跑测试前先 `unset NO_COLOR`。

---

## 4. Repository layout

```
core/         Rust engine + CLI (`marguerite-core`), gateway, agent runtime, ACP shell
tui/          ratatui terminal workbench (`marguerite-tui`)
gui/          Tauri 2 desktop app (React + TypeScript); Rust shell in gui/src-tauri
webui/        Vue 3 console (Vite + Pinia + vue-i18n); served on loopback only
pyengine/     Python sidecar (OpenSquilla lineage), reached through the gateway
workbench/    Local-model experimentation bench (FastAPI + React)
third_party/  Reference + adapter layer (NOT part of the cargo workspace)
docs/         Documentation; docs/cli-reference.md is the CLI authority
scripts/      Gates, packaging and diagnostics (verify.sh is the entry point)
```

> 中文：`third_party/` 不参与 cargo workspace 编译，只作参考与适配层。

---

## 5. Architecture invariants (breaking these is a defect, not a style choice)

1. **Loopback only.** The Web console and the unified gateway listen on
   `127.0.0.1` (default port `18790`). ACP `serve` listens on `127.0.0.1` only.
   Never `0.0.0.0`, never a public interface, never from the Tauri front end.
2. **Offline capable.** Orchestration and `agent.invoke` run through
   `Orchestrator::analyze` without an LLM call; with no API key the engine
   degrades to an offline echo.
3. **CLI output contract.** Structured errors go to stderr with an exit code;
   `--json` emits `{ok,data,meta}`; `NO_COLOR` is respected; status markers are
   ASCII `[OK]` / `[FAIL]` — **emoji icons are forbidden**.
4. **Route triplication.** `cli/argv.rs::SUBCOMMANDS`, the
   `route_table.register(...)` calls in `program.rs`, and the route-count
   assertions in `test_program_*` must agree. Adding a route without touching all
   three fails the suite.
5. **Monotonic tool surface.** Within a session the tool/service set only grows
   (Bloom anchoring `Anchored → Ptc → Full`). Never withdraw tools mid-session,
   never replace a service for an already-provided `TypeId` outside
   `reload_plugin()`.
6. **No full-context re-injection.** Sessions only append messages. Replacing the
   system prompt or resetting history is forbidden; plugins declaring
   `system_prompt` / `full_context` are flagged as context-leak suspects.
7. **Cache honesty.** `MultiTierCache` Tier 0 shares the gateway's
   `Arc<PrefixCache>`; semantic (approximate) hits are disabled on the tool path
   because dropping a tool call silently is a correctness bug. `cache.status`
   reports measured hit rates, never a fabricated 100 %.
8. **Docs lose to code.** When a document and the implementation disagree, fix the
   document. Never invent a CLI verb to make an example compile.

> 中文：只监听回环、离线可运行、CLI 输出契约（stderr + 退出码 + ASCII 标记）、
> 路由三处同步、工具面单调扩展、禁止全上下文重注入、缓存不得虚报、文档以代码为准。

---

## 6. Commits and branches

Use Conventional Commits. The history of this repository is the reference:

```
feat(core): 五项深度强化 — 会话级预填充去重/路由优化/联合调用
fix(ui): 桌宠拖拽后位置未持久化
chore(release): 版本 0.1.4 → 0.1.5
docs(cli): 补 `marguerite cache` 小节
test(core): 覆盖 CompletionGate 的落盘核验分支
```

* Subject line ≤ 72 characters, imperative mood.
* Common scopes: `core`, `tui`, `gui`, `webui`, `dsh`, `docs`, `ci`, `release`.
  Descriptive Chinese or English subjects are both fine — match the surrounding
  history.
* One logical change per commit; keep pure formatting or renames in their own
  commit.
* Branch names: `feat/<short-slug>`, `fix/<short-slug>`, `docs/<short-slug>`.

> 中文：提交信息用 Conventional Commits（`feat(scope): 描述`），主题行 ≤ 72 字符。

---

## 7. Pull requests

Before opening one:

1. Rebase on the current default branch (`master`).
2. Run `bash scripts/verify.sh --clean --all` and paste the tail of the output
   into the PR template — the template asks for evidence, not a summary.
3. If you touched `docs/*.md` or any CLI route, run
   `python3 scripts/check-doc-cli-drift.py` and confirm exit code 0.
4. Keep the PR focused. Large mixed PRs (behaviour + formatting + dependency
   bumps) are asked to split.

A reviewer will look for: the invariant list in §5, the CLI output contract, no
new secrets or machine-specific absolute paths (in code, tests **and** logs), and
the route triplication staying consistent.

> 中文：PR 前必须跑全量闸门并把输出贴进模板；改动 CLI 路由或文档示例必须过漂移门禁。

---

## 8. Documentation

* `docs/cli-reference.md` is the authoritative CLI manual — update it in the same
  commit as the code change.
* `docs/mcp.md` and `docs/development.md` are the technical guides.
* `AGENTS.md` is the repository guide for AI agents and contributors; it also
  carries the test baseline and the gotcha list.
* Documentation is scanned by the drift gate, so **write only commands that exist**.
  The gate reads fenced code blocks and inline `code` spans; prose is ignored.

> 中文：改 CLI 就同步改 `docs/cli-reference.md`；文档里只写真实存在的命令。

---

## 9. Reporting bugs and requesting features

Use the issue forms: **Bug report** and **Feature request**. The bug form asks for
`marguerite doctor` output and the affected surface (CLI / TUI / desktop / console
/ sidecar) because reproduction depends heavily on which of the four you are in.

Redact API keys before pasting logs. Keys look like `sk-…`; the engine already
redacts known secret shapes in its own logs, but pasted transcripts are your
responsibility.

> 中文：用 issue 表单提报；贴日志前先抹掉 `sk-` 开头的密钥。
