# Marguerite documentation

> Index of the user-facing documentation. For the project pitch and quickstart, start
> at the [root README](../README.md). · [中文](../README.zh-Hans.md)

## Start here

| Document | What it covers |
|---|---|
| [Product positioning](PRODUCT.md) | Who this is for, the core journeys, the three differentiators, and the explicit **NOSO list** of things this project refuses to do. |
| [Architecture](architecture.md) | Module layering, data flow, configuration, and the quality gates that guard the design. |
| [CLI reference](cli-reference.md) | The authoritative CLI contract — every subcommand, flag, exit code, and the `--json` envelope. |
| [Commands and modes](commands-and-modes-reference.md) | Quick reference for the runtime modes (`plan` / `agent` / `ensemble` / `bloom` / `omni`). |

## Subsystems

| Document | What it covers |
|---|---|
| [MCP integration](mcp.md) | Model Context Protocol on both sides — Marguerite as client *and* as server, plus the tool registry. |
| [Memory](memory.md) | The three-layer memory model: sensory → short-term → long-term, and how retrieval degrades. |
| [Skills](skills.md) | YAML skills, the loader, and just-in-time triggering by intent. |
| [Goals](goal.md) | The goal loop: persistent objectives, autonomous continuation rounds, and blocked gating. |
| [Heavy runtime guide](heavy-runtime-guide.md) | Optional ASR / local quantized models. Nothing heavy is pre-downloaded or vendored — this explains how to opt in. |

## Working on the project

| Document | What it covers |
|---|---|
| [Development guide](development.md) | Environment setup, commands, code conventions, and the testing strategy. |
| [Contributing](../CONTRIBUTING.md) | How to propose a change, and the gates a change must pass. |
| [Security policy](../SECURITY.md) | How to report a vulnerability privately, and the security model (loopback-only, four guardrail layers, secrets handling). |
| [Agent guide](../AGENTS.md) | The architectural invariants that reviews check against — the fastest way to understand *why* the code is shaped this way. |
| [Changelog](../CHANGELOG.md) | Released changes, newest first. |

## Verifying a change

This repository's rule is blunt: **no completion claim without fresh verification
evidence.**

```bash
unset NO_COLOR                      # NO_COLOR=1 flips two theme tests to a false failure
bash scripts/verify.sh --clean --all
```

See [Contributing](../CONTRIBUTING.md) for the individual gates and what each one proves.

---

## About this directory

`docs/` contains the documents above, plus:

- `decisions/` — decision records and open questions.
- `internal/` — phase progress logs, the dated L3 sandbox security audit, and the
  local-model workbench design. Internal engineering records, not documentation.
- `planning/`, `reports/`, `research/`, `uiux/`, `archive/` — historical working
  notes from the project's development. They are kept for traceability and are
  **not** part of the user-facing contract; where they disagree with the code, the
  code wins.

Everything under `internal/`, `planning/`, `reports/`, `research/`, `uiux/` and
`archive/` is excluded from the `docs/*.md` CLI-drift scan (`scripts/check-doc-cli-drift.py`
globs only the top level), so only the documents above carry the CLI contract.
