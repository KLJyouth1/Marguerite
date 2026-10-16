# scripts/

> The gate is [`verify.sh`](verify.sh). Everything below exists either to be called by
> it, by CI, or by the packaging pipeline.

```bash
bash scripts/verify.sh --clean --all      # the completion gate (see AGENTS.md §8)
bash scripts/verify.sh --core             # Rust stack only
bash scripts/verify.sh --webui            # front-end stack only
```

## Called by `verify.sh`

| Script | What it proves |
|---|---|
| [`check-doc-cli-drift.py`](check-doc-cli-drift.py) | Every `marguerite <sub>` shown in `docs/*.md` has a real route in `cli/argv.rs` / `program.rs`. Glob is **top-level `docs/*.md` only** — that is why internal notes live in subdirectories. |
| [`check-version-sync.py`](check-version-sync.py) | The **authoritative** version guard: 9 locations (both Cargo crates, `tauri.conf.json`, both `package.json`, and both npm lockfiles' two version fields each) all equal `core/Cargo.toml`. Supports `--print` / `--fix`. |
| [`check-tokens-sync.mjs`](check-tokens-sync.mjs) | The blue/orange semantic theme tokens are identical across the three front ends. |
| [`check-secrets.py`](check-secrets.py) | No real-looking API key among the files `git add -A` would commit. A fixture must label itself (`sk-fake-…`) or it fails; `sk-` is also this repo's CSS class prefix, so the patterns are deliberately tight. A real key is already in the history and must be rotated by hand — see AGENTS.md §4. |
| [`check-doc-links.py`](check-doc-links.py) | Every relative link in **this project's own** docs resolves. Scope is a positive allowlist (root `*.md`, `docs/`, `.github/`, `scripts/`, `design/`) on purpose: 195 of the repo's broken links sit inside vendored third-party skill packs this project does not own. Links inside code spans and fenced blocks are ignored, because GitHub does not render them and docs legitimately quote a broken link when explaining it. |
| [`check-readme-claims.py`](check-readme-claims.py) | The English and Chinese READMEs agree on every **language-independent** fact (test baselines, ignored/skipped/files counts, locale key count, engine version, gated script list, heading shape) — and the gate total they print equals the number of checks `verify.sh --all` actually dispatches. That second half is not decoration: it caught this repo advertising "18/18" while `--all` dispatched 17. Exits `2`, never `0`, if a pattern stops matching and the comparison would go vacuous. |

## Packaging and release

| Script | What it does |
|---|---|
| [`package_app.cmd`](package_app.cmd) | Desktop installer (MSI + NSIS) from an already-built `webui/dist`. Uses an isolated `CARGO_TARGET_DIR` to dodge local file locks. Fails closed if `webui/dist` or the `pets/` sprite skins are missing. |
| [`package_full.cmd`](package_full.cmd) | Full release bundle: console, Python sidecar, skills, docs, and a smoke subset. |
| [`ws_full_health.py`](ws_full_health.py) | WebSocket surface health probe; shipped into the release bundle by `package_full.cmd`. |

## Standalone guards

Not wired into `verify.sh`, but real assertions you can run on demand.

| Script | What it proves |
|---|---|
| [`assert-deliverables.sh`](assert-deliverables.sh) | Named deliverable files exist and are non-empty (AGENTS.md §8 step 2). |
| [`check_package_redlines.py`](check_package_redlines.py) | Packaging red lines — no secrets or local paths baked into a bundle. |
| [`check_visual_redlines.py`](check_visual_redlines.py) | Visual/design red lines. |
| [`check_plan_revision.py`](check_plan_revision.py) | Plan-revision contract. |
| [`check-pyengine-collection.py`](check-pyengine-collection.py) | A bare `pytest` over `pyengine/` collects cleanly (**0 collection errors**), and the un-migrated-scaffolding skip list has not quietly become a place where broken tests hide. Exits `2` — never `0` — when it cannot verify. Deliberately **not** in `verify.sh`: a full `--collect-only` imports *every* test module, whereas the gate confines itself to `pyengine/tests/unit`. See AGENTS.md §2. |

## Subdirectories

| Directory | Contents |
|---|---|
| [`demo/`](demo/) | Reproducible screenshot pipeline. [`capture_console.py`](demo/capture_console.py) (Playwright) driven by [`mock_llm.py`](demo/mock_llm.py), a local OpenAI-compatible stub — **no API key required**; this produces the console images in `docs/assets/`. [`render_tui.py`](demo/render_tui.py) produces the **terminal** image: it runs [`core/examples/tui_frame.rs`](../core/examples/tui_frame.rs), which renders a real frame through ratatui's `TestBackend` (no pty needed, so it also works on Windows and in CI) and re-emits every cell as ANSI; the script converts that to PNG. It also cross-checks each row against the terminal width, which validates the Rust and Python width tables against each other. |
| [`dev/`](dev/) | Historical one-off diagnostics. See [`dev/README.md`](dev/README.md). **Not maintained, not run by any gate.** |
| `qa/`, `launch/` | Local, **untracked** scratch (ad-hoc QA probes, personal Windows launchers). Not part of the repository; absent from a fresh clone. |
