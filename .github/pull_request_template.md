## What changed

<!-- One paragraph. What behaviour is different after this PR? -->

## Why

<!-- The problem or the issue this closes. Use "Closes #123" when applicable. -->

## How it was verified

<!--
  Paste the evidence, not a summary of it. This repository treats
  "verification before completion" as a hard gate (AGENTS.md §8): a claim of
  "done / passing / fixed" without fresh command output is not accepted.
-->

- [ ] `bash scripts/verify.sh --clean --all` — full gate, all `[PASS]`
- [ ] `python3 scripts/check-doc-cli-drift.py` — exit 0 (if any `docs/*.md` or CLI route changed)
- [ ] New/updated tests cover the change (state the count delta)

```
<paste the command and its tail output here>
```

## Affected surfaces

<!-- Tick everything this PR touches; a reviewer uses this to pick the right smoke path. -->

- [ ] `core/` (engine, CLI, gateway, agent runtime)
- [ ] `tui/` (ratatui workbench)
- [ ] `gui/` (Tauri desktop shell / React front end)
- [ ] `webui/` (Vue 3 console)
- [ ] `pyengine/` (Python sidecar)
- [ ] `docs/` or `scripts/`
- [ ] Build / CI / packaging only

## Checklist

- [ ] The change respects the architecture invariants (AGENTS.md §3): Web UI and ACP listen on `127.0.0.1` only; no tool-surface replacement mid-session; no full-context re-injection.
- [ ] No secrets, tokens or personal absolute paths were added (including in tests, fixtures and logs).
- [ ] `docs/cli-reference.md` and the route tables (`cli/argv.rs::SUBCOMMANDS`, `program.rs::route_table.register(...)`, route-count assertions) stay in sync, or the PR explains why they do not need to.
- [ ] CLI output contract kept: structured errors on stderr with exit codes, `--json` emits `{ok,data,meta}`, `NO_COLOR` respected, ASCII `[OK]`/`[FAIL]` markers (no emoji icons).
- [ ] Breaking changes, if any, are called out at the top of this description.
- [ ] I have read [CONTRIBUTING.md](../CONTRIBUTING.md) and the [Code of Conduct](../CODE_OF_CONDUCT.md).

## Notes for the reviewer

<!-- Known gaps, follow-up work, parts you are unsure about. Being explicit here is cheaper than a review round-trip. -->
