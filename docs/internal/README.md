# Internal engineering records

> **Not documentation.** Nothing in this directory is part of the user-facing contract,
> describes supported behaviour, or is maintained against the current code. For anything
> that *is* maintained, start at the [documentation index](../README.md).

These files were moved here out of the repository root during the publish-readiness
sprint. They were previously scattered at the top level, where a first-time visitor
would reasonably mistake them for entry-point documentation.

| File | What it actually is |
|---|---|
| `PHASE_C_PROGRESS.md` | Progress log for phase C (notify / hotkey / tray / menu). |
| `PHASE_D_PROGRESS.md` | Progress log for phase D (`run_turn` + React event stream). |
| `PHASE_E_PROGRESS.md` | Progress log for phase E. |
| `PHASE_F_RELEASE.md` | Release-phase planning notes — packaging, signing, update channel. The signature story in particular is **stale**: it records that code signing was never configured. |
| `Security_Audit_L3_Sandbox_2026-08-11.md` | A dated security audit of the L3 sandbox layer. Treat as a point-in-time record, not a current guarantee. Current guarantees live in [`SECURITY.md`](../../SECURITY.md). |
| `LOCAL_MODEL_WORKBENCH.md` | Design notes for a local-model (quantized) workbench. Shipped guidance is [`heavy-runtime-guide.md`](../heavy-runtime-guide.md). |

## Why these are kept at all

For traceability: they record *why* decisions were made and what was tried. They are
deliberately excluded from the CLI-drift scan in `scripts/check-doc-cli-drift.py`, which
globs only `docs/*.md` — so a stale command shown here cannot masquerade as a documented
contract. When one of these conflicts with the code, the code is authoritative.
