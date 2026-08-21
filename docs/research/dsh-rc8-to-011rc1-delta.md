# DSH v0.1.0-rc.8 → v0.1.1-rc.1 Functional Delta Report

*Source repo:* `third_party/harness/harness-core` (clone of `deepseek-ai/deepseek-harness`).
*Scope:* functional delta only — tests, CI, docs, and `.agents/notes/` noise are excluded from the functional analysis but quantified where they distort the raw diff.

---

## 0. Version Facts

| Fact | Value |
|---|---|
| `dsh-v0.1.0-rc.8` | `141eb6fef83422698aef7a981029e843e8161534` (2026-08-19 23:11:50 +0800, PR #2783) |
| `dsh-v0.1.1-rc.1` | `528c682e061696f5a160f363f236ecbf53cbd006` (2026-08-21 14:21:44 +0800, PR #2890, = `origin/master` HEAD) |
| Ancestry | rc.8 **is** the merge-base of rc.1 — a linear first-parent span of **28 merge commits** |
| Root package | `@deepseek-ai/dsh-root` `0.1.1-rc.1` |
| Full commit span | 13,100 commits (side-branch history folded in by the 28 merges) |
| Raw tree diff | 2,368 files, +23,679 / −11,723 |
| Functional source span | **88 files** under `packages/*/*/src/**` |

**Critical read on the raw numbers.** The raw diff is dominated by documentation, not code. `git diff --dirstat` attributes roughly **37%** of changed files to `.agents/notes/**` (the repo's agent-authored "implemented" notes) and another ~15% to `docs/**` + `website/**`. Three of the 28 PRs (`#2739 speed-up-doc-sync`, `#2847 doc-site-md-projection`, `#2889 docs-ci-release-control`) are bulk doc-sync/CI release-control work. The *actual* product code change is 88 source files, concentrated in **credentials/authorization**, **llm**, **session**, **host/webserver**, and **client UI**.

---

## 1. Package Inventory Delta

### 1.1 Top-level `packages/` dirs (the `git ls-tree -d <tag> packages/` view)

Both tags list the **identical set of 50** group directories:

`acp api attachment boot bundle client code-runtime compaction context core credentials e2b examples experimental extensions feedback fs goal guard hooks host identity interaction jobs llm lsp mcp plan preset runtime-diagnostics sandbox schedule sdk session-query session settings shell skill spill storage subagent subprocess terminal test-support todo typert util web workflow workspace`

→ **No new, removed, or renamed top-level package groups.**

### 1.2 Real packages (leaves `packages/<group>/<leaf>`)

The task's `dsh-*` naming is an abstraction; the repo uses `packages/<group>/<leaf>`. Mapping to the task's names:

| Task name | Actual leaf(s) |
|---|---|
| dsh-base | `packages/core/*` (agent, agent-loop, system-prompt, tools, …) |
| dsh-agent | `packages/core/agent`, `packages/core/agent-loop` |
| dsh-session | `packages/session/*` (14 leaves) |
| dsh-system-prompt | `packages/core/system-prompt` |
| dsh-persona | `packages/preset/persona` |
| dsh-token-meter | `packages/llm/token-meter` |
| dsh-compaction-* | `packages/compaction/*` |
| dsh-goal* | `packages/goal/*` |
| dsh-plan-mode | `packages/plan/plan-mode` |
| dsh-schedule | `packages/schedule/schedule` |
| dsh-skill* | `packages/skill/*` |
| dsh-workflow* | `packages/workflow/*` |
| dsh-tool-* | `packages/*/tool-*` (tool-bash, tool-ralph, tool-subagent, …) |
| dsh-subagent* | `packages/subagent/*` |
| dsh-cordis* | `packages/extensions/*` (`cordis-client-runner`, `cordis-host-runner`, `tool-cordis`, `ui-cordis`) + `packages/typert/*` |
| dsh-client-ui-* | `packages/client/ui-*` |
| dsh-llm* | `packages/llm/*` |

Leaf counts: **226 → 227**.

| Class | Result |
|---|---|
| NEW | **`packages/credentials/authorization`** (the only new leaf) |
| REMOVED | none |
| RENAMED | none (a *file* rename inside `ui-subagent`, see §4.5) |

---

## 2. Changelog / Release Notes

There is **no `CHANGELOG.md`** at the repo root, and no release-notes files under `website/` or `docs/` that mention `0.1.1`. Release narrative lives in (a) PR titles and (b) `.agents/notes/**` (committed here in bulk, not newly authored). The 28 first-parent merge PRs group into these themes:

| Theme | PRs |
|---|---|
| **Docs / CI / release tooling** (bulk of the volume) | #2889 docs-ci-release-control, #2739 speed-up-doc-sync, #2847 doc-site-md-projection, #2862 translation-link-locale, #2708 localized-chinese-doc-links, #2798 release-check-panel, #2768 split-master-workflows, #2758 knip-eval, #2878 fix-pnpm-win |
| **LLM auth & routing** | #2509 pi-ai-auth-alignment, #2726 deepseek-vision-model-catalog |
| **Session snapshots / projection schema** | #2702 stable-session-snapshots, #2730 projection-state-schema |
| **Web/webserver** | #2725 bootserver, #2808 frontend-static-miss-404, #2794 web-new-session-first |
| **Client UI** | #2856 subagent-header-switcher, #2776 table-optimize, #2820 multiline-question-answer, #1687 (same), #2749 cache-hit-decimal-v2, #2796 mention-backdrop-perf, #1373 remove-cordis-catalog-line-numbers |
| **Robustness fixes** | #2760 continuation-cleanup-temp-handles, #2814 composer-edit-range-attribution, #2844 turn-error-survives-retry-exhaustion, #1798 bwrap-proc-root-escape, #2608 permission-copy-and-default |

The functional themes that matter for a port are: **authorization seam (§4.1)**, **session-projection state/view split (§4.3)**, **webserver index injections (§4.4)**, and the smaller UI/robustness items (§4.5–4.6).

---

## 3. Agent Presets — NO CHANGE

The task expected `config/agent-presets/`; the real location is **`apps/cli/config/agent-presets/{minimal,standard,code,cordis}/`** (each with `preset.yml` + `agent.cordis.yml`).

I hash-compared both files across all four presets between the tags:

| Preset | `preset.yml` identical | `agent.cordis.yml` identical |
|---|---|---|
| minimal | ✅ | ✅ |
| standard | ✅ | ✅ |
| code | ✅ | ✅ |
| cordis | ✅ | ✅ |

Only `packages/preset/agent-presets/{README.i18n.yaml, README.zh.md, package.json}` changed (version bump). **No persona sections, plan-mode text, anchoring knobs, tool sets, or skill catalogs changed in this release.** The `.agents/notes` files about "per-session-agent-presets" and "host-plane-ownership-after-presets" are *documentation* of work done before rc.8, not new code in this delta.

---

## 4. Functional Changes by Package

Legend: only `packages/*/*/src/**` files are considered. Packages listed "unchanged" had **zero** source changes.

### 4.1 `credentials/*` — NEW authorization seam (the headline feature)

- **`packages/credentials/authorization` (NEW)** — a complete capability seam `ctx.authorization` (`AuthorizationService`). It is a registry of *credential-obtaining flows*: a flow (`registerFlow({key,label,methods,run})`) owns an interactive conversation ("open this page, paste that code, pick an account") through a neutral `notify`/`prompt` vocabulary, one attempt per key (`begin()`), with cancellation and a commit contract (`NOT_COMMITTED` if the flow resolves without writing a credential record). Emits `authorization/settled`. Error taxonomy: `AuthorizationError` (`NO_FLOW`, `UNKNOWN_METHOD`, `ALREADY_IN_FLIGHT`, `NOT_COMMITTED`) and `AuthorizationDeclinedError` (a human "no" is a *cancellation*, not a failure).
- **`packages/credentials/credentials`** — adds `CredentialKey` (scoped `<scope>/<id>`), `CredentialRecord` (`api-key` | `grant`), and guards `isCredentialRefName` / `isCredentialKeySegment` so out-of-grammar inputs read as "not set" rather than throwing.
- **`packages/credentials/credentials-local`** (+471/−32) — local store adapter for the new record shapes (persists OAuth grants verbatim as opaque JSON).

**Net:** the framework gains a generic, protocol-agnostic interactive-credential-authorization mechanism (OAuth / device-code / api-key login), separate from any one provider.

### 4.2 `llm/*` — pi-ai auth alignment + vision model + token-meter refactor

- **`packages/llm/llm-pi-ai`** — new `src/auth.ts` (translates pi-ai's auth model ↔ harness `CredentialRecord`) and `src/login.ts` (registers `ctx.authorization` flows for catalog providers, relaying pi-ai `AuthEvent`s — `auth_url`, `device_code` — into the seam's `notify`/`prompt`). `src/adapter.ts`/`catalog.ts`/`index.ts` updated. This is PR #2509.
- **`packages/llm/llm-deepseek`** — adds **`deepseek-v4-flash-vision-exp`** (`DeepSeek-V4-Flash-Vision-Exp`) with `inputModalities: ['text','image']` to the default catalog (PR #2726).
- **`packages/llm/token-meter`** — `usage-projection.ts` + `breakdown-projection.ts` refactored to the new projection API (below). The persisted state schema now explicitly carries the `claim` (shadow price) field; the wire view is split out under `wire.viewSchema`.

### 4.3 `session/*` — the cross-cutting projection API refactor (biggest mechanism change)

`packages/session/session-projection/src/index.ts` + `types.ts`, plus `session-projection-cache`, and every downstream unit:

- `ProjectionDefinition` splits the single `schema`/`view` pair into **`stateSchema`** (validates the *persisted* state that seeds a fold) and an optional **`wire: { viewSchema, view }`** (the *client* view). A unit may now be **host-only** (fold + checkpoint, no client view).
- A new module-augmented type table **`SessionProjectionStateMap`** (persisted state) is separated from **`SessionProjectionMap`** (client view), registered via `declare module '@deepseek-ai/dsh-session-projection/types'`.
- New `stateOf(session, key)` reads raw host state without computing views.
- Downstream units updated to the new shape (mechanical, no semantic change): `goal`, `todo/tool-todo`, `plan/plan-mode`, `interaction/permission-presets`, `session/session-stats`, `session/session-title`, `subagent/subagent`, `llm/token-meter`.

**Net:** a cleaner state-machine contract — pure synchronous fold (`init`/`apply`) over committed events, versioned (`stateVersion`) persisted state, and an optional validated client projection — with a single zod schema as the source of truth for each. This is PR #2730 + #2702 ("stable-session-snapshots").

### 4.4 `host/*` — structured index injections

- **`packages/host/webserver/src/injections.ts` (NEW)** — replaces ad-hoc `tapIndex(html)` string transforms with typed `IndexInjection` rows: `{kind:'global'|'script'|'script-src'|'style'|'html'}` with `placement: 'head'|'body'`. Rows are pure JSON-serializable data so one table feeds both the served HTML renderer (`renderIndexInjections`) and a static-worker boot payload. New event `webserver/index-inject` collects rows. `src/index.ts` wires it.
- `packages/host/frontend-static/src/index.ts` — updated to the injection table (fixes the static 404/worker path, PR #2808).
- `packages/client/ui-theme` — `injectBootTheme(html)` → `bootThemeInjection()` returning an `IndexInjection` row (no theming *capability* change; see §5).

### 4.5 `client/*` — UI (the largest bucket, 39 files)

- **`ui-subagent`** — `SubagentCatalogAction.tsx` **renamed → `SubagentHeaderLineage.tsx`** (+285/−43). The subagent header now renders a **lineage** dropdown (parent/child tree via `subagentsByParent` / `indexSubagentDescendants`) instead of a plain catalog action. PR #2856.
- **`ui-conversation`** —
  - `chat/StatsLine.tsx`: `cacheHitPercent()` now returns a **string with decimal precision** (`99.9…%`) instead of rounding near-full cache hits up to `100`. Exact integer/decimal arithmetic avoids float drift. PR #2749.
  - `conversation-nodes/turn-error.ts`: removed the `hidden` flag and retry-suppression logic — a terminal turn error is **always** rendered; retries live inside the failing turn and are shown separately. PR #2844.
  - `skeleton/InputBar.tsx`, `skeleton/PermissionSelect.tsx`, `apply.ts`, `contract/slots.ts`, CSS: table styling, composer edit-range attribution (#2814), permission selector wiring.
- **`ui-user-questions`** — `QuestionComposer.tsx` (+65/−13) + CSS: **multiline question/answer** support (PR #2820/#1687).
- **`ui-permission-presets`** — `PermissionRow.tsx`, `presentation.ts`, `settings-store.ts`, `locales.ts`: UI reflects the new permission `origin` provenance and blank-reuse refresh (§4.6).
- **`ui-workspace`** — new `WorkspaceBrowser.tsx` (workspace browse).
- **`ui-primitives/markdown/render.tsx`** + CSS: markdown **table** rendering optimization (PR #2776).
- `runtime`, `connection`, `modules`, `ui-settings-models`, `ui-settings-plugins`, `web/boot.ts`: session-manager / workspace-service and boot changes tied to stable snapshots and "web new session first" (PR #2794).

### 4.6 `interaction/permission-presets` — permission provenance

`src/index.ts` (+74/−18):
- `permission/preset` event gains `origin: 'default' | 'selection' | 'inferred'` (provenance of *why* a preset was selected; older origin-less logs never read as refreshable defaults).
- New `refreshDefaultForReuse(session)`: on confirmed Web "blank session reuse", advances only a still-effective default-origin selection to the new default preset. PR #2608 + #2794.

### 4.7 `sandbox`, `util`, `extensions`, `api`

- **`sandbox/sandbox-local`** — `profiles.ts` bwrap profile adds **`--unshare-pid`** before `--proc /proc`; the probe uses the same profile. Closes the `/proc` root-escape via a private PID namespace (PR #1798).
- **`util/atomic-write`** — `withFileLock(filename, op, options?)` gains `FileLockOptions.waitMs` (default 2000 ms), so a lock holder doing network I/O (credential token refresh) can state a longer wait instead of failing every contender. PR #2760.
- **`extensions/tool-cordis`** — `api-catalog.ts` (+222/−18): expanded Cordis API catalog. `cordis-client-runner/slot-catalog.ts` (+54/−22) and `cordis-host-runner/sandbox.ts` (+33/−6) updated. `typert/generator/cordis-catalog.ts` drops line numbers from catalog records (PR #1373).
- **`api/gateway`, `api/remotes`, `host/apiproxy`** — session API/schema and remote-event changes supporting stable session snapshots and multiline answers.

### 4.8 Explicitly UNCHANGED (no `src` changes)

`core/agent`, `core/agent-loop`, `core/system-prompt`, `core/tools`, `core/agent-default-model`, `preset/persona`, `preset/agent-presets`, **all `compaction/*`**, `schedule`, **all `skill/*`**, **all `workflow/*`** (incl. `tool-ralph`), `guard/*`, `hooks/*`, `lsp/*`, `mcp/*`, `spill/*`, `storage/*`, `shell/*` (incl. `tool-bash`, `tool-pwsh`), `terminal/*`, `code-runtime/*`, `context/*`, `fs/*` (incl. `tool-fs`, `tool-str-replace-editor`).

---

## 5. Theme / Personalization

Searching the functional diff for `theme|skin|token|accent|personal` yields only:

1. **`packages/client/ui-theme/src/{boot-theme,index}.ts`** — pure refactor: the pre-plugin dark/light bootstrap is re-expressed as an `IndexInjection` row over the new `webserver/index-inject` event. The capability is unchanged (still `system|light|dark`, `data-ds-dark-theme` body attribute, `color-scheme`).
2. **`packages/llm/token-meter/*`** — projection-API refactor (not theming).
3. **`packages/client/ui-permission-presets/*`** — permission-row/settings-store UI (not theming).

**Conclusion:** rc.1 introduces **no new theme/skin/accent/personalization system** beyond what rc.8 already shipped in `ui-theme`. The only "theming" delta is a boot-injection mechanism refactor. A Rust port looking for a "theming" feature to back-port will find nothing new here; the existing `ui-theme` (theme preference + `data-ds-dark-theme` + `color-scheme`) is the whole surface.

---

## 6. Anchoring / LiangShen

Searching the diff for `anchor|bootstrap|messageSources|deferred`:

- The only `bootstrap` matches are the **theme boot** (§5) and the "pre-plugin theme bootstrap" `.agents/notes` doc.
- There is **no `messageSources`, `deferred` (prompt-deferral), or two-phase "anchoring" gating code anywhere upstream**, and nothing changed between the tags.
- Preset files (§3) contain **no anchoring knobs**.

**Conclusion:** "anchoring" is entirely an *external* mechanism (the liangshen plugin outside this repo). Upstream has no equivalent to port; the closest upstream primitive is the **per-session agent-preset** mechanism (selected per session, with `preset.yml`/`agent.cordis.yml` mounted) — which is the *seam* a liangshen-style plugin would hook, but that seam did not change in this release.

---

## 7. FUSION IMPLICATIONS (actionable for the Rust port)

Ordered by porting value.

1. **Port the `ProjectionDefinition` contract (state fold + versioned state + optional client view).** This is the cleanest mechanism in the delta: a pure synchronous `init`/`apply` fold over committed session events, `stateVersion`-tagged persisted state, a single schema as source of truth, and an optional validated `wire.view` for the client. In Rust: `trait Projection { type State: Serialize + DeserializeOwned; fn init() -> Self::State; fn apply(&mut Self::State, ev: &SessionEvent) -> bool; }` with `state_version` in a persisted `(session_id, key, ver, seq, state)` row, and a separate `View` type that is `Send`-serialized to the client. This is the backbone DSH uses for goal, todo, plan-mode, token-meter, permissions, subagent lineage, and session stats — porting it once buys all of those subsystems.

2. **Port the `ctx.authorization` seam (credential-flow registry).** Generic interactive-credential acquisition (OAuth / device-code / api-key) with a neutral `notify`/`prompt` vocabulary, one-attempt-per-key, cancellation, and a commit contract (`NOT_COMMITTED`). In Rust: an `AuthorizationFlow` trait + a registry with `begin(key, method, interaction, signal) -> Outcome`, where `interaction` is a callback pair. Reuse it for any provider that needs "open page → paste code".

3. **Port the `CredentialKey`/`CredentialRecord` model.** Scoped `<scope>/<id>` keys, `api-key` vs `grant` records, with grammar guards that return "not set" on out-of-grammar input. This is the storage primitive the authorization seam writes through, and the `withFileLock(waitMs)` change (§4.7) is exactly the durability primitive it needs for multi-process credential files.

4. **Port `IndexInjection` rows (boot HTML).** Typed `{global|script|script-src|style|html}` rows with `head|body` placement, collected via an event and rendered by one renderer that both the served page and a static-worker boot can share. In Rust: an `enum IndexInjection` + `render_injections(html, &[Injection])`, replacing string-splicing transforms. Matters if the Rust harness serves a boot HTML page.

5. **Port permission provenance (`origin: default|selection|inferred`).** Explicit provenance on config-knob application, plus a `refreshDefaultForReuse` rule that only advances still-effective *default*-origin selections on "blank reuse". This is a small, high-value correctness model for any session-scoped settings system: distinguish "the user picked this" from "this was inferred/defaulted".

6. **Port `--unshare-pid` for the bubblewrap profile.** A one-flag change (`--unshare-pid` before `--proc /proc`) that closes a `/proc` PID-namespace escape. Directly applicable if the Rust port spawns sandboxed child processes.

7. **Port the cache-hit decimal display.** The `cacheHitPercent` exact integer/decimal arithmetic (return `99.9…%` rather than rounding to `100`) is a self-contained, float-drift-free display helper worth copying for any token-meter UI.

8. **Do NOT chase these — they are already settled or external:** (a) *anchoring/message-source gating* — no upstream equivalent; it is external (liangshen), so design the Rust "preset mount" seam to be hookable rather than copying an anchor; (b) *agent presets* — unchanged this release, so port the rc.8 preset layout as-is; (c) *theme/skin/accent system* — rc.1 adds nothing new; the `ui-theme` preference + `data-ds-dark-theme` + `color-scheme` triple is the whole surface.

**Bottom line:** the rc.8 → rc.1 delta is a *focused, mechanism-oriented* release, not a feature explosion. The two port-worthy architectural additions are the **state/view projection split** and the **authorization seam**; everything else is a small correctness or UI fix. The headline "features" one might expect (anchoring, theming, compaction, skills, workflow/ralph) are either already in rc.8 or live outside the repo.
