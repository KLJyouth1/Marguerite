# scripts/dev/ — historical one-off diagnostics

> **Not maintained. Not run by any gate. Safe to ignore.**

These are ad-hoc probe and diagnostic scripts written during specific debugging
sessions — `check_com_fns2.cjs`, `test_office3.cjs`, `verify_inject_v4.cjs`,
`diag_*.cjs`, and so on. Most were one shot: written to answer a single question,
run once, and kept only because deleting evidence is worse than filing it.

They were moved here out of `scripts/` during the publish-readiness pass, where a
first-time reader would otherwise have to sort ~100 scratch files to find the four
scripts that `verify.sh` actually calls. Nothing in the repository references
anything in this directory — verified with `git grep` across all tracked files —
so moving them broke no path.

## Why keep them at all

For traceability. When a past bug report asks "how was this measured?", the answer
is often one of these. They are the working-out, not the product.

## Ground rules if you use one

- They assume the author's local environment: absolute Windows paths, a running
  gateway on a specific port, pre-seeded local state. Expect to edit before running.
- Several reference services, accounts or fixtures that no longer exist.
- None of them are part of the test suite. A green `verify.sh` says nothing about
  this directory, and **nothing here should be treated as a quality signal**.

If you need behaviour that one of these probes checks, promote it into a real test
in the owning crate or package rather than reviving the script here.
