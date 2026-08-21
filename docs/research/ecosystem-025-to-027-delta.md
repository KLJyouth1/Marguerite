# Ecosystem Version-Delta Report: 0.2.5 / 0.2.6 → 0.2.7

Scope: `@linxin666/dsh-liangshen` and `@linxin666/dsh-web-ui-all`, analyzed from
npm-published tarballs (expanded and SHA256-compared) against the locally
installed reference copies under
`C:\Users\Administrator\.dsh\profiles\web\node_modules\@linxin666\`.

Method: `npm pack` of 0.2.5 / 0.2.6 / 0.2.7, `tar -xzf` expansion, per-file
SHA256 comparison, plus `npm view` metadata. All preset/cordis payload files
were confirmed byte-identical (not just size-equal) via SHA256.

---

## 1. Version facts

| Package | 0.2.5 published | 0.2.6 published | 0.2.7 published | `latest` |
| --- | --- | --- | --- | --- |
| `@linxin666/dsh-liangshen` | 2026-08-20T05:21:24Z | 2026-08-21T04:10:31Z | 2026-08-21T09:33:59Z | **0.2.7** |
| `@linxin666/dsh-web-ui-all` | 2026-08-20T05:21:47Z | 2026-08-21T04:10:57Z | 2026-08-21T09:34:23Z | **0.2.7** |

- Both packages publish on the same cadence; 0.2.6 and 0.2.7 are ~1 day and
  ~5 h apart respectively.
- Both package `description` strings are byte-identical across 0.2.5/0.2.6/0.2.7
  (no description change).
- Locally installed reference copies are **0.2.6** for both packages (not 0.2.5),
  so 0.2.5 and 0.2.7 were pulled fresh from npm for diffing.
- The `liangshen` preset tree in the installed profile copy is byte-identical to
  the npm 0.2.5/0.2.6/0.2.7 tarballs (an earlier "192-byte" reading was a
  `Format-Table` column-width truncation artifact, not a real difference).

### Tarball sizes (corroboration)

| Package | 0.2.5 | 0.2.6 | 0.2.7 |
| --- | --- | --- | --- |
| `dsh-liangshen` .tgz | 51 763 B | 51 763 B | 56 046 B |
| `dsh-web-ui-all` .tgz | 8 784 B | 8 786 B | 8 783 B |

---

## 2. liangshen delta

### 2.1 0.2.5 → 0.2.6: pure republish (no functional change)

Every payload file is byte-identical (SHA256-equal): `presets/liangshen/*`
(`preset.yml`, `agent.cordis.yml`, `custom-bash.mjs`, `tool-bootstrap.mjs`,
`NOTICE`), `cordis.patch.yml`, `lib/index.js`, `lib/types/*`, `src/*`, `tools/*`,
`README*.md`. The only change is the `version` field in `package.json`
(`"0.2.5"` → `"0.2.6"`). **There is nothing to port between 0.2.5 and 0.2.6.**

### 2.2 0.2.6 → 0.2.7: announcement default flip + settings-surface integration

**The anchoring engine did NOT change.** The preset files that carry the
anchor knobs are SHA256-identical across 0.2.5/0.2.6/0.2.7:

| File (in package `presets/liangshen/` and root) | 0.2.6 → 0.2.7 |
| --- | --- |
| `preset.yml` | unchanged |
| `agent.cordis.yml` (19 219 B) | unchanged |
| `tool-bootstrap.mjs` (25 198 B) | unchanged |
| `custom-bash.mjs` | unchanged |
| `NOTICE` | unchanged |
| `cordis.patch.yml` | unchanged |

What *did* change is the host-half plugin (`src/index.ts` → `lib/index.js`,
18 295 B → 26 608 B) and `lib/types/index.d.ts` (3 822 B → 4 311 B):

1. **`announceToAgent` default flipped `true` → `false`** (issue #839).
   - 0.2.5/0.2.6: `announceToAgent` defaults `true` — the plugin injected a
     system-prompt section announcing itself into every agent's prompt by default.
   - 0.2.7: defaults `false` — the announcement ships **off**; "keep prompts clean
     unless the user opts in" (comment in `src/index.ts`).
   - The model-facing announcement text `LIANGSHEN_GUIDANCE` is unchanged; only
     whether it is injected by default changed.

2. **New live settings-surface integration** via `@deepseek-ai/dsh-settings`:
   - New imports: `installSettingsSection`, `settingsNamespace` from
     `@deepseek-ai/dsh-settings`.
   - New exported constant `LIANGSHEN_SETTINGS_NAMESPACE = settingsNamespace('dsh-liangshen')`.
   - `applyImpl` now keeps a `current()` closure for the config source; it calls
     `installSettingsSection(ctx, LIANGSHEN_SETTINGS_NAMESPACE, Config, config ?? {}, { setSource, onChange: refresh })`.
     A web-settings edit re-runs `refresh()` live (re-sync + re-register the
     announcement section); deployments without the settings service fall back to
     the composition entry.

3. **New `src/announce.test.ts`** (2 512 B, 3 tests): default resolves `false`;
   registers section `plugin:dsh-liangshen` with `LIANGSHEN_GUIDANCE` when on;
   silent when off.

4. **devDependencies**: added `@deepseek-ai/dsh-settings ^0.1.1-rc.1`;
   `@deepseek-ai/dsh-system-prompt` bumped `^0.1.0-rc.8` → `^0.1.1-rc.1`.

5. Unchanged internals worth noting for the port: `inject: ['systemPrompt']`,
   announcement section `name: 'plugin:dsh-liangshen'`, `order: 150`, and the
   `ctx.effect(() => () => { disposeSection?.(); … })` disposer so the section is
   removed on unmount/update.

### 2.3 Anchoring knobs — the frozen spec (unchanged in 0.2.7)

From `presets/liangshen/agent.cordis.yml`, the `tool-bootstrap` row:

```yaml
- id: tool-bootstrap
  name: ./tool-bootstrap.mjs
  config:
    shellTools: [bash]
    commonTools: [str_replace_editor]
    messageSources: [user, goal]
    anchorGate: true
    maxBootstrapSteps: 4
    promoteAfterFirstResponse: true
    bootstrapMaxTokens: 1024
    compactionTools: [read, write, edit, glob, grep, todo_write, ask_user_question]
    deferredSources: [agent-instructions, skill-catalog]
    deferredGraceSteps: 1
    promotedPresentation: code
    instructionHint: true
    # phase1FirstCallInstruction: (commented out — off by default, issue #274)
```

Phase-1 persona (single line) and the phase-1 tool rows are unchanged:
`persona` → `@deepseek-ai/dsh-persona` with
`text: You are a helpful software engineer assistant.`; phase-1 editor
`str-replace-editor` (`maxOutputChars: 16000`) inherits the host file sandbox;
persistent `bash` on POSIX / `custom-bash.mjs` on win32.

### 2.4 Knob → Rust field mapping (existing fusion already consumes this)

The Rust fusion already models these unchanged knobs in
`core/src/mode/spec.rs::BloomAnchoringSpec` (consumed by
`core/src/mode/anchoring.rs::AnchorStage::{Anchored→Ptc→Full}` and declared in
`core/src/mode/modes/bloom.yaml` + `plugins/manifests/dsh-presets.plugin.json`).
Since the preset is byte-identical 0.2.5→0.2.7, this mapping is unchanged and
already at 0.2.7 parity:

| liangshen `agent.cordis.yml` knob (0.2.7) | Rust `BloomAnchoringSpec` field | Value |
| --- | --- | --- |
| `shellTools` + `commonTools` | `minimal_tools` | `[bash, str_replace_editor]` → `[bash_persistent, str_replace_editor]` |
| `promotedPresentation: code` (PTC) | `ptc_tool` | `run_code` |
| `anchorGate` (we/let me heuristic) | `prefix_gate` | `deepseek` |
| `maxBootstrapSteps` | `fallback_steps` | `4` |
| `messageSources` | `message_sources` | `[user, goal]` |
| `bootstrapMaxTokens` | `bootstrap_max_tokens` | `Some(1024)` |
| `deferredSources` | `deferred_sources` | `[agent-instructions, skill-catalog]` |
| `deferredGraceSteps` | `deferred_grace_steps` | `1` |
| `promoteAfterFirstResponse` | `promote_after_first_response` | `true` |
| `compactionTools` | *(deliberately not ported)* | red line #2 — see §4 |

`compactionTools` is the one knob the fusion intentionally omits: liangshen's
"post-compaction fallback to a shrunken tool set" violates AGENTS.md red line #2
(monotonic tool-surface expansion, no mid-session shrink/replace). This
deliberate divergence is already recorded in `docs/dsh-fusion-v2-2026-08-21.md`
§5 and is **unchanged** by 0.2.7.

---

## 3. web-ui-all delta

### 3.1 Aggregate payload is byte-identical across 0.2.5 / 0.2.6 / 0.2.7

SHA256-equal for every shipped file: `cordis.patch.yml`, `lib/index.js`
(host: `apply` = no-op, `inject = []`), `lib/client.js` + `lib/client.js.map`
(the "compat shim"), `README.md`, `README.zh.md`, `README.i18n.yaml`, `LICENSE`.
The aggregate plugin has **no code change** in this range.

The only change in `package.json` is the `dependencies` map: every bundled
sub-package version pin is bumped in lockstep (`0.2.5` → `0.2.6` → `0.2.7`).

### 3.2 Package roster: stable at 17, nothing added/removed

| # | Package | 0.2.5 | 0.2.6 | 0.2.7 |
| --- | --- | --- | --- | --- |
| 1 | `dsh-better-sidebar` (external) | 0.14.0 | 0.14.0 | 0.14.0 |
| 2 | `@linxin666/dsh-client-ui-plugin-manager` | 0.2.5 | 0.2.6 | 0.2.7 |
| 3 | `@linxin666/dsh-client-ui-community-plugins` | 0.2.5 | 0.2.6 | 0.2.7 |
| 4 | `@linxin666/dsh-client-ui-aionui-panel` | 0.2.5 | 0.2.6 | 0.2.7 |
| 5 | `@linxin666/dsh-client-ui-task-board` | 0.2.5 | 0.2.6 | 0.2.7 |
| 6 | `@linxin666/dsh-pet` | 0.2.5 | 0.2.6 | 0.2.7 |
| 7 | `@linxin666/dsh-remote-web-ui` | 0.2.5 | 0.2.6 | 0.2.7 |
| 8 | `@linxin666/dsh-client-ui-git-graph` | 0.2.5 | 0.2.6 | 0.2.7 |
| 9 | `@linxin666/dsh-ssh` | 0.2.5 | 0.2.6 | 0.2.7 |
| 10 | `@linxin666/dsh-tool-describe-image` | 0.2.5 | 0.2.6 | 0.2.7 |
| 11 | `@linxin666/dsh-liangshen` | 0.2.5 | 0.2.6 | 0.2.7 |
| 12 | `@linxin666/dsh-chat-recovery` | 0.2.5 | 0.2.6 | 0.2.7 |
| 13 | `@linxin666/dsh-desktop-launcher` | 0.2.5 | 0.2.6 | 0.2.7 |
| 14 | `@linxin666/dsh-client-ui-web-ui-settings` | 0.2.5 | 0.2.6 | 0.2.7 |
| 15 | `@linxin666/dsh-client-ui-skill-explorer` | 0.2.5 | 0.2.6 | 0.2.7 |
| 16 | `@linxin666/dsh-skins` | 0.2.5 | 0.2.6 | 0.2.7 |
| 17 | `@linxin666/dsh-client-ui-skin-center` | 0.2.5 | 0.2.6 | 0.2.7 |

- No package was **added** or **removed** between 0.2.5 and 0.2.7.
- `dsh-better-sidebar` is the sole external dep and stays pinned at **0.14.0**
  (never bumped in this range).

### 3.3 Notable structural facts (unchanged, but the port surface)

- `cordis.patch.yml` inserts 17 namespaced rows (`web-ui-*`) so the bundle
  coexists with standalone installs; `web-ui-compat` points at the bundle itself
  (`@linxin666/dsh-web-ui-all`).
- `lib/client.js` (the "compat shim") stamps column attributes — `data-pane`
  on `[class*="sidebarCol"]` / `[class*="centerCol"]` / `[class*="detailsCol"]`
  and `data-dsh-frame` on the frame parent — coalesced through a
  `requestAnimationFrame` + `MutationObserver` pass. Byte-identical across the
  range.

### 3.4 Theming / personalization subsystem (supplementary check)

`@linxin666/dsh-skins` was expanded at 0.2.5 and 0.2.7: **byte-identical**
(all 28 files, same sizes). It bundles **11 built-in skins**, each a pure
asset directory (`skins/<name>/package.json` + `lib/client.js` + `lib/index.js`):

`blue-fantasy`, `dragon-heir`, `harbor`, `maid-atelier`, `matrix`, `miku`,
`minecraft`, `trading`, `whale-mom`, `whale-song`, `xp`.

`@linxin666/dsh-client-ui-skin-center` and
`@linxin666/dsh-client-ui-web-ui-settings` descriptions are unchanged between
0.2.5 and 0.2.7. Skin-center description confirms the theming model: skins are
pure asset directories (built-in + `$DSH_HOME/skins`), loaded and rendered only
by the skin center, with **live try-on and one-click atomic apply (no reload)**.

---

## 4. FUSION IMPLICATIONS (Rust port)

Concrete mechanisms to port / decisions to record (cross-referenced to the
existing fusion state in `docs/dsh-fusion-v2-2026-08-21.md`):

1. **Anchoring parity is already reached — 0.2.7 adds nothing.** The preset is
   byte-identical 0.2.5→0.2.7, and the Rust side already models the full knob set
   in `core/src/mode/spec.rs::BloomAnchoringSpec`
   (`minimal_tools`/`ptc_tool`/`prefix_gate`/`fallback_steps`/`message_sources`/
   `bootstrap_max_tokens`/`deferred_sources`/`deferred_grace_steps`/
   `promote_after_first_response`), consumed by `core/src/mode/anchoring.rs`
   (`AnchorStage::{Anchored→Ptc→Full}`). **No new anchor field, value, or gate
   rule is introduced in 0.2.7** (see the mapping in §2.4). The only still-open
   item from the prior fusion plan is assembly wiring, not any 0.2.7 delta.

2. **The one real semantic change: `announceToAgent` default flips `true`→`false`**
   (issue #839). Upstream 0.2.5/0.2.6 injected a `plugin:dsh-liangshen`
   system-prompt section (`LIANGSHEN_GUIDANCE`, order `150`) by default; 0.2.7
   ships it **off** and opt-in. The Rust fusion currently has **no** liangshen
   self-announcement section (absent from the §2 mapping table), so there is
   nothing to flip; if one is later added for parity, it must default **off**.

3. **New upstream integration seam to mirror: live settings binding.** 0.2.7
   introduces `settingsNamespace('dsh-liangshen')` + `installSettingsSection`
   (from `@deepseek-ai/dsh-settings`), so `enabled`/`announceToAgent` are
   editable live in the settings UI and re-run the mount `refresh()` on change,
   with a graceful fallback to the composition entry when the settings service is
   absent. The Rust equivalent maps to the existing mode-config override path
   (`~/.marguerite/modes/*.yaml` / `ModeSpec`): bind a plugin's enable/announce
   flags to a live, reloadable source with an idempotent, disposer-clean
   `refresh()` — clear the old section before re-registering — and fall back to
   the static spec when the settings surface is absent. Optional; not a blocking
   parity gap.

4. **Disposable announcement lifecycle.** The announcement section registration is
   reversible (`ctx.effect` returning a disposer that calls `disposeSection()`)
   because a live settings edit re-registers it. Any Rust analog must be equally
   reversible.

5. **`compaction_tools` divergence is unchanged — keep it unported.** 0.2.7 does
   not alter the post-compaction "shrink the tool surface" behavior. The existing
   deliberate divergence (AGENTS.md red line #2, monotonic expansion; compaction
   routed through context/session instead) remains correct and needs no change.

6. **No preset/cordis YAML changes to port.** `preset.yml`, `agent.cordis.yml`,
   `tool-bootstrap.mjs`, `custom-bash.mjs`, and `cordis.patch.yml` are unchanged;
   a 0.2.7 upgrade of the liangshen half is purely the `lib/index.js`/`src/index.ts`
   host-plugin code, not the preset data.

7. **web-ui-all: bump-only, no structural port.** The aggregate adds/removes
   nothing and changes no code. Reaching 0.2.7 = re-pin the 17 sub-package version
   strings to 0.2.7 (leave `dsh-better-sidebar` at 0.14.0). There is no new
   plugin to fuse and no compat-shim change.

8. **Theming/personalization: no new skins or behavior.** `dsh-skins` is
   byte-identical (same 11 built-in skins: `blue-fantasy`, `dragon-heir`,
   `harbor`, `maid-atelier`, `matrix`, `miku`, `minecraft`, `trading`,
   `whale-mom`, `whale-song`, `xp`); skin-center still uses the "asset dirs +
   live try-on + one-click atomic apply, no reload" model with `$DSH_HOME/skins`
   overrides. Nothing new to port from 0.2.7.

9. **Scope caveat for the sub-packages.** This report deep-diffed `dsh-liangshen`
   and the `dsh-web-ui-all` aggregate (plus a supplementary check of
   `dsh-skins`). The other 15 sub-packages (`task-board`, `ssh`, `git-graph`,
   `aionui-panel`, `pet`, `community-plugins`, `plugin-manager`,
   `remote-web-ui`, `chat-recovery`, `skill-explorer`, `desktop-launcher`,
   `tool-describe-image`, `web-ui-settings`, `skin-center`, plus external
   `dsh-better-sidebar`) each carry their own independent 0.2.5/0.2.6→0.2.7
   deltas that are **not** covered here — the aggregate only proves they were
   re-published under a bumped version, not what changed inside them. Diff those
   individually if feature-level parity in panels/tool-cards/settings is required.

### Bottom line

`0.2.5/0.2.6 → 0.2.7` is a **near-no-op for the Rust fusion**: the anchor
engine, preset data, web-ui-all aggregate code, and theming assets are all
byte-identical across the range. The only functional change is liangshen's
`announceToAgent` default flip (`true`→`false`) plus a new live-settings
integration seam, both confined to the liangshen host-plugin (`src/index.ts`,
`lib/index.js`), with `devDependencies` gaining `@deepseek-ai/dsh-settings`
and bumping `@deepseek-ai/dsh-system-prompt` `^0.1.0-rc.8` → `^0.1.1-rc.1`.
