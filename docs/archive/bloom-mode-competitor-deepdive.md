# Competitor Architecture Deep-Dive — Lessons for UnivAgent

**Scope:** Pure research (no files modified). Three local codebases analyzed for what to **adopt** into UnivAgent and what is an **anti-pattern to avoid**.

| Project | Path | Shape | Lang | Key takeaway |
|---|---|---|---|---|
| **openclaw** | `external/openclaw/` | Multi-platform agent **gateway** + runtimes | TypeScript/Node | Best-in-class *extensibility & multi-surface* design |
| **hermes-agent** | `external/hermes-agent/` | Self-evolving agent, 5-layer memory + routines | Python | Best *memory/learning-loop* design |
| **deepseek-tui** | `external/deepseek-tui/` | Full agent **TUI** client | Rust (ratatui) | Best *streaming TUI* + OS-sandbox design |

UnivAgent layers referenced in the final table: **TUI core / desktop GUI / API backend / skills / memory**.

---

## 1. openclaw — Multi-platform Agent Gateway

### Architecture
- **Entry chain:** `openclaw.mjs` (199-line launcher → Node version check, compile-cache, help fast-path) → `src/entry.ts` → `runCli()` (`src/cli/run-main.ts`). The launcher is deliberately tiny and only bootstraps the built `dist/entry.js`.
- **Core decomposition:** `src/gateway/` is the heart — a long-running **gateway server** (`gateway-server-agent`) that owns sessions, auth, channels, and tool execution. Platform "runtimes" (CLI, `apps/android`, `apps/ios`, `apps/macos`, `node-host`, `canvas-host`) are *clients* that connect to the gateway over a protocol (ACP + WebSocket) via `GatewayClient` (`src/gateway/client.ts`). The gateway is the single source of truth; runtimes are thin front-ends.
- **Control flow:** command → `agent-command.ts` → `resolveAgentRuntimeConfig` / `resolveEffectiveModelFallbacks` → session → model call → tool loop. `src/gateway/call.ts` is the RPC boundary between a runtime client and the gateway server.
- **Notable:** `src/gateway/boot.ts` implements a *boot-check* pattern (BOOT.md script run on startup). `RuntimeEnv` (`src/runtime.ts`) abstracts output so the same agent code runs under CLI vs gateway vs tests.

### Memory
- **Canonical file-based memory:** `MEMORY.md` at the workspace root (`src/memory/root-memory-files.ts`, legacy `memory.md` also supported). Resolution handles canonical-vs-legacy and a repair dir.
- **Context engine abstraction:** `src/context-engine/` defines a `ContextEngine` interface (`registry.ts`) with methods `bootstrap / maintain / ingest / afterTurn / assemble / compact` — a pluggable pipeline for assembling context each turn and compacting it. Legacy compat shim keeps session-key-based callers working.
- **Vector/embeddings path exists:** `src/gateway/embeddings-http.ts` (HTTP embeddings) + an `active-memory` extension → memory is a mixture of flat files + pluggable retrieval, not a single DB.
- **Session store:** JSONL/session files under a state dir (`resolveStateDir`), plus a `session-store`.

### Tools & Skills
- **Skills = folders with a `SKILL.md`** (frontmatter: `name`, `description`, `requires.bins`, `install`). Loader: `src/agents/skills/local-loader.ts` reads `SKILL.md`, **verifies the file path is within root and refuses symlinks** (`openVerifiedFileSync` + `isPathWithinRoot`) — good injection defense.
- **Discovery:** bundled skills (`src/agents/skills/bundled-dir.ts`) + local workspace skills + optional refresh (`refresh.ts`). Invocation policy parsed from frontmatter.
- **MCP + extensions:** `src/mcp/` bridges MCP servers (`channel-server.ts`, `tools-stdio-server.ts`). `extensions/` is a huge catalog of providers (anthropic, deepseek, google, github, browser, discord, …) — each extension adds tools/providers via a plugin SDK (`packages/plugin-sdk`, `packages/plugin-package-contract`).
- **ClawHub extension mechanism:** `skills/clawhub/SKILL.md` documents a **registry + CLI** (`clawhub search/install/update/publish`). Registry default `https://clawhub.com`, override via `CLAWHUB_REGISTRY`. This is the publish/install distribution layer for skills — skills are versioned, hash-checked on update.

### Multi-model Orchestration
- **Model catalog as a mergeable manifest:** `src/model-catalog/manifest-planner.ts` merges `ModelCatalog` rows from *plugins* (each extension can contribute providers/discovery), detects `mergeKey` **conflicts** between plugins, and normalizes provider IDs. `provider-index/openclaw-provider-index.ts` has fallback/route logic.
- **Per-session overrides:** `resolveEffectiveModelFallbacks` (`src/agents/agent-scope.ts`), `applyModelOverrideToSessionEntry`. Fallback chains are first-class, not hardcoded.
- **Least-privilege model access** is scoped per method (`src/gateway/method-scopes.ts` → `resolveLeastPrivilegeOperatorScopesForMethod`).

### Sandbox & Security
- **Container sandbox:** `Dockerfile.sandbox` builds a `debian:bookworm-slim` image that runs as a non-root **`sandbox`** user with only `bash/curl/git/jq/python3/ripgrep`. `Dockerfile.sandbox-common` layers on node/bun/brew toolchains. This is the isolation boundary for risky exec.
- **Exec approval gate:** `src/gateway/exec-approval-manager.ts` + `node-invoke-system-run-approval.ts` — risky commands produce an `ExecApprovalRequest` with an id, timeout, and the *requesting* conn/device/client id (replay protection). iOS push approval path (`exec-approval-ios-push.ts`) lets a phone approve a desktop exec.
- **Auth & transport:** TLS fingerprint pinning (`loadGatewayTlsRuntime`), token resolution, rate limiting (`auth-rate-limit.ts`), and least-privilege scopes. `openVerifiedFileSync` blocks symlink escapes everywhere files are read.

### Gateway + Runtimes (specific ask)
- **One gateway, many runtimes:** the gateway process is the brain; `apps/*` and `node-host`/`canvas-host` are runtime surfaces that reconnect. This cleanly separates *agent logic* from *UI surface* — exactly the UnivAgent split (API backend vs TUI/desktop GUI).
- **Container-target abstraction:** `src/cli/container-target.ts` supports `runtime: "podman" | "docker"` and forwards env/ports (`-e` vs `--env`) per runtime. Lets the gateway itself run containerized while runtimes attach remotely.
- **Plugin contract is explicit:** `packages/plugin-sdk` + `plugin-package-contract` define how a third party ships a provider/skill. This is the piece UnivAgent's "skills" layer should copy.

### ADOPT
- **Gateway/runtime split** — put all agent logic in one API backend; TUI + desktop GUI are stateless clients. (Maps to API backend + TUI + desktop GUI.)
- **Skills-as-folders + `SKILL.md` frontmatter + ClawHub-style registry** for distribution/versioning.
- **Symlink-verified skill/file loading** (`openVerifiedFileSync`) — cheap, high-value injection defense.
- **Mergeable model-catalog manifest** with plugin-contributed providers + explicit conflict detection.
- **Exec-approval with requestor-scoped ids + mobile push approval.**

### AVOID
- **Monorepo sprawl:** thousands of files, `extensions/` is enormous — high maintenance cost. UnivAgent should keep a *curated* skill/provider set, not fork every vendor integration.
- **`dist/` build requirement + version-fast-path hacks** in the launcher — fragile bootstrap; keep UnivAgent's entry simpler.
- **Over-centralized gateway auth** can become a single point of complexity; isolate concerns early.

---

## 2. hermes-agent — Self-Evolving Agent (5-layer memory + routines)

### Architecture
- **Entry points:** `run_agent.py::main()` (direct CLI, `max_turns` default 10) and `cli.py` (large), plus a **multi-platform gateway** (`gateway/run.py`, `gateway/platforms/` for telegram/discord/dingtalk/feishu/qqbot/yuanbao). `hermes` launcher script.
- **Main loop:** classic tool-calling loop in `run_agent.py` — send messages → receive `tool_calls` → `_execute_tool_calls` (with concurrent batch support, `_should_parallelize_tool_batch`) → repair malformed JSON args (`_repair_tool_call_arguments`) → loop until `max_turns`. Supports both structured `tool_calls` and free-form `<tool_call>` XML tags (`run_agent.py:~2798-2911`).
- **Adapter layer:** `agent/*_adapter.py` (anthropic, bedrock, gemini_native, gemini_cloudcode, codex_responses) — one adapter per backend, isolating provider quirks.

### Memory (the 5 layers)
Hermes has no single "5-layer" enum; the design realizes **five memory tiers** plus pluggable vector stores:
1. **Working/context memory** — the live prompt window, assembled by `prompt_builder.py` + `context_compressor.py` (token-budgeted compaction).
2. **Session memory** — `hermes_state.py`: **SQLite with FTS5 full-text search** (`state.db`, `SCHEMA_VERSION=9`, WAL mode). Stores sessions, full message history, tool calls, token/cost accounting; session chains via `parent_session_id` for compression-split sessions. FTS5 gives cross-session search.
3. **Declarative long-term memory** — `MEMORY.md` / `USER.md` files (`memory_provider.py` builtin provider).
4. **User modeling** — `plugins/memory/honcho` implements a **two-layer dialectic** (base context every `contextCadence` turns + dialectic supplement every `dialecticCadence` turns), truncated to a token budget.
5. **Procedural memory** — **skills** (`skill_manager_tool.py`): the agent can *create/edit/delete its own skills*.
- **Pluggable providers:** `MemoryProvider` ABC (`agent/memory_provider.py`) with lifecycle `initialize / system_prompt_block / prefetch / sync_turn / get_tool_schemas / handle_tool_call`, orchestrated by `MemoryManager` (`agent/memory_manager.py`) — **builtin always first, only ONE external provider at a time** (mem0, supermemory, byterover, hindsight, honcho, holographic, openviking, retaindb). This prevents tool-schema bloat and backend conflicts.
- **Injection defense:** recalled context is wrapped in `<memory-context>` with a system note ("NOT new user input") and `sanitize_context()` strips injected fence tags (`memory_manager.py`).

### Tools & Skills
- **Self-registering tool registry:** `tools/registry.py` uses AST scan (`_is_registry_register_call`) to discover modules that call `registry.register()` at top level — zero central manifest, new tools just drop a file.
- **Toolset system:** predefined sets (`research`, `development`, `safe`, `web`, `terminal`, `vision`, `creative`, `reasoning`, `analysis`, …) composed of individual tools; enabled/disabled per run.
- **Autonomous skill creation (self-evolution):** `tools/skill_manager_tool.py` — actions `create/edit/patch/delete/write_file/remove_file` under `~/.hermes/skills/`. New skills are *procedural memory* captured from proven successes. External hub installs are security-scanned (`tools/skills_guard`); agent-created skills scanned only if `skills.guard_agent_created` is on (off by default, because the agent can already run the same code via `terminal()`). Skills Hub uses ClawHub (`tests/tools/test_skills_hub_clawhub.py`).

### Multi-model Orchestration
- **Credential pool:** `agent/credential_pool.py` holds multiple provider credentials and **fallback chains** (e.g. auxiliary client fallback so one provider outage doesn't kill the run). `nous_rate_guard.py` throttles.
- **Model metadata/routing:** `agent/model_metadata.py` + `models_dev.py` resolve model capabilities; adapters select by configured provider. Default backend OpenRouter (`provider/model` format).

### Sandbox & Security
- **Write denylist:** `agent/file_safety.py` — `build_write_denied_paths` blocks `.ssh/*`, `.env`, shell rc files, `/etc/sudoers`, `/etc/passwd|shadow`; `build_write_denied_prefixes` blocks `.aws`, `.gnupg`, `.kube`, `.docker`, `.azure`. Optional `HERMES_WRITE_SAFE_ROOT` restricts writes to one tree.
- **Skill scanning:** `skills_guard.py` scans hub/agent-created skills.
- **ACP permissions:** `acp_adapter/permissions.py` gates actions in the Agent-Client-Protocol path.
- **Notably absent:** no OS-level container sandbox like openclaw/deepseek — isolation is *policy/denylist* only. Risky shell still runs locally.

### ADOPT
- **MemoryManager = builtin + exactly one pluggable external provider** — avoids schema bloat and conflicting backends. (Maps to memory layer.)
- **`<memory-context>` fence + sanitize** on recalled memory — cheap prompt-injection defense.
- **Self-registering tool discovery via AST** — zero-boilerplate extension. (Maps to skills/API backend.)
- **Autonomous skill creation from successful runs** — turns experience into reusable procedural memory. (Maps to skills.)
- **SQLite + FTS5** for cross-session search with cost/token accounting. (Maps to memory.)

### AVOID
- **No OS-level sandbox** — file denylist is bypassable; pair with container isolation.
- **`guard_agent_created` default OFF** — self-modifying skills run unscanned by default; unsafe default, should be opt-out-able but audited.
- **`run_agent.py` is 12.8k lines** — the agent loop, adapters, and orchestration are tangled in one file. Keep UnivAgent's loop modular.
- **Max-turns loop is simplistic** (no sub-agent depth budget, no explicit compaction trigger in the loop) — compaction is external.

---

## 3. deepseek-tui — Full Agent TUI (Rust)

### Architecture
- **Workspace:** Rust crates — `tui` (UI), `core` (engine), `agent`, `tools`, `mcp`, `execpolicy`, `sandbox`, `secrets`, `state`, `config`, `protocol`, `hooks`, `cli`, `app-server`. Clean separation of concerns per crate.
- **Entry:** `cli/src/main.rs` → `tui/ui.rs::run_tui()` → `run_event_loop()`.
- **Control flow (the key pattern):** the **engine** (`core/engine.rs`) runs as an async task and communicates with the UI **purely through mpsc channels**:
  - UI → engine: `Op` (`SendMessage`, `ApproveToolCall`, `DenyToolCall`, `CancelRequest`, `SpawnSubAgent`, `ChangeMode`, …) via `rx_op`.
  - engine → UI: `EngineEvent` (`MessageStarted/Delta/Complete`, `ThinkingStarted/Delta/Complete`, `ToolCallStarted/Complete`, `TurnStarted/Complete`, `CompactionStarted/…`, `Error`, `Status`, `AgentList`) via `tx_event`.
  - This **decouples the agent loop from rendering entirely** — the UI never blocks on the model, and the model never touches the terminal.

### How the TUI renders streaming output (specific ask)
- **Producer/consumer render loop:** `run_event_loop` (`tui/ui.rs:610`) polls the engine channel **non-blocking** each frame (`rx.try_recv()`), updates an in-memory `App`/`history`, and redraws.
- **Frame coalescing:** a **120 FPS draw cap** (`frame_rate_limiter::FrameRateLimiter`) — without it, every SSE chunk triggers a full redraw. This is the single most important TUI-perf lesson.
- **Incremental markdown streaming:** `tui/streaming/` (`chunking.rs`, `commit_tick.rs`, `line_buffer.rs`, `MarkdownStreamCollector`) splits streamed text into **grapme-aligned chunks**, drips them into the transcript via commit ticks, and only commits *complete lines* — so partial markdown never renders broken. `sanitize_stream_chunk()` cleans each chunk.
- **Separate thinking vs text streams:** reasoning deltas go to a `reasoning_buffer` rendered as a thinking block; assistant prose and tool groups are flushed in the right order (Codex pattern: streamed prose always lands *after* the committed tool group).
- **Layout:** ratatui + crossterm (`CrosstermBackend`), `color_compat.rs` adapts colors, `frame_rate_limiter.rs`, `live_transcript.rs`, `transcript_cache.rs` (per-cell render cache keyed by revision — only changed rows re-render).

### Memory
- **File-based user memory:** `memory.rs` loads `~/.deepseek/memory.md` (configurable `memory_path` / `DEEPSEEK_MEMORY_PATH`), wraps in `<user_memory source="…">`, prepends to system prompt. **Opt-in** (`[memory] enabled` or `DEEPSEEK_MEMORY=on`). Truncates with a `<truncated bytes=N>` marker past 100 KB.
- **Fast capture:** `# foo` in composer appends a timestamped bullet; `remember` tool lets the model append durable preferences; `/memory edit` opens `$EDITOR`.
- **Context management:** `core/capacity.rs` + `core/capacity_memory.rs` (token budgeting) and `core/coherence.rs` (coherence state) manage the live window; `compaction.rs` triggers compaction at `compaction_threshold_for_model_and_effort` (per-model 80% discipline).

### Tools & Skills
- **Tools crate:** `tools/shell.rs` (poll-based subprocess via `poll()`, cross-platform), `tools/plan.rs`, `tools/todo.rs`, `tools/subagent.rs`, `tools/spec.rs` (tool schema + `RuntimeToolServices`). Engine dispatch in `core/engine/tool_execution.rs`.
- **MCP + ACP:** `mcp/` crate + `mcp_server.rs` + `acp_server.rs`; routing in `tui/mcp_routing.rs`, `tui/tool_routing.rs`, `tui/subagent_routing.rs`.
- **Skills:** `skills/` + `skill_state.rs` — skill state tracked; provider/model pickers in `tui/provider_picker.rs`, `tui/model_picker.rs`.

### Multi-model Orchestration
- **`models.rs`:** OpenAI-compatible + DeepSeek request/response models; `reasoning_effort` tiers (`off|low|medium|high|max`); `auto_model`; per-model context windows (`DEEPSEEK_V4_CONTEXT_WINDOW_TOKENS = 1_000_000`).
- **Overrides:** `session.auto_model`, `subagent_model_overrides`, `reasoning_effort_auto`; `resolve_subagent_assignment_route` picks model per sub-agent. Provider picker UI.

### Sandbox & Security (specific strength)
- **OS-level sandbox backends:** `sandbox/` crate — `backend.rs` (trait `SandboxBackend`), `landlock.rs` (Linux Landlock), `seatbelt.rs` (macOS Seatbelt), `windows.rs` (Windows), `opensandbox.rs`, `policy.rs`. The engine creates a sandbox backend (`core/engine.rs:523 create_backend(api_config)`) — **real kernel-enforced isolation**, not just a denylist.
- **Command safety / arity allow-list:** `command_safety.rs` + `execpolicy/bash_arity.rs` — `classify_command()` maps a token slice to a canonical **command prefix** using a `COMMAND_ARITY` dictionary (e.g. `git status` arity 1 matches `git status -s` but NOT `git push`). Ported from opencode. This is the auto-allow/approval gate.
- **Approval modes:** `tui/approval.rs` (`ApprovalMode`), `YOLO`/auto-approve vs `Agent` mode; sub-agents **don't inherit YOLO** (engine: "Sub-agents don't inherit YOLO mode").
- **Trust & network:** `workspace_trust.rs` (directory trust gate in onboarding), `network_policy.rs`.

### ADOPT
- **Engine↔UI over mpsc channels + non-blocking `try_recv` render loop** — full decoupling; reuse for UnivAgent TUI core and for streaming to the desktop GUI / API backend (SSE/WebSocket). (Maps to TUI core + API backend.)
- **120 FPS draw cap + per-cell render cache** — essential TUI smoothness. (TUI core.)
- **`MarkdownStreamCollector` grapheme-aligned chunking + commit ticks** — correct incremental markdown. (TUI core.)
- **OS-level sandbox (Landlock/Seatbelt/Windows) selected by `create_backend`** — the right isolation model; far stronger than denylists. (API backend / skills execution.)
- **Command arity allow-list** (`git status` matches flags, not `git push`) — precise auto-approve. (skills / API backend.)
- **Sub-agents don't inherit YOLO** — secure-by-default privilege scoping.

### AVOID
- **`DEEPSEEK_MEMORY=on` opt-in default-off** — memory is off until enabled; UnivAgent should default memory *on* but clearly scoped.
- **Streaming logic is intricate** (`flush_active_cell`, reasoning ordering) — powerful but easy to regress; isolate it behind one well-tested `StreamingState` (they do — keep it that way, don't inline).
- **Onboarding trust gate is a separate state machine** — fine, but don't let trust checks leak into the hot render path.

---

## Consolidated: Adopt vs Avoid (mapped to UnivAgent layers)

| UnivAgent Layer | ADOPT (from) | AVOID (from) |
|---|---|---|
| **TUI core** | Engine↔UI mpsc channel + non-blocking render loop (deepseek); 120 FPS draw cap + per-cell cache (deepseek); grapheme-aligned `MarkdownStreamCollector` chunking/commit-ticks (deepseek); `sanitize_stream_chunk` (deepseek) | Inlining streaming/ordering logic into the draw path (deepseek caveat); memory off-by-default (deepseek) |
| **desktop GUI** | Gateway/runtime split: GUI is stateless client of one backend (openclaw); stream engine `EngineEvent`s over WebSocket/SSE to the GUI using the same event enum (deepseek) | Tightly coupling UI surface to agent logic (anti-pattern both avoid successfully — don't reintroduce it) |
| **API backend** | Single gateway owning sessions/auth/tools; runtimes reconnect (openclaw); OS-level sandbox via `create_backend` (Landlock/Seatbelt/Windows) (deepseek); exec-approval with requestor-scoped ids + mobile push (openclaw); container sandbox as non-root user (openclaw) | File-denylist-only isolation (hermes); 12.8k-line monolithic agent loop (hermes); fragile `dist/` bootstrap (openclaw) |
| **skills** | Skills = folder + `SKILL.md` frontmatter, symlink-verified load (openclaw); ClawHub-style versioned registry (openclaw/hermes); self-registering tool discovery via AST (hermes); autonomous skill creation from successes (hermes); command-arity allow-list for auto-approve (deepseek) | Forking every vendor integration into the repo (openclaw `extensions/` sprawl); self-scan default-off for agent-created skills (hermes) |
| **memory** | 5-tier model: context window / SQLite+FTS5 sessions / file memory (MEMORY.md,USER.md) / user-modeling / procedural skills (hermes); pluggable providers, builtin + 1 external (hermes); `<memory-context>` fence + sanitize (hermes); per-model compaction threshold + capacity budgeting (deepseek); opt-in file memory with truncation marker (deepseek) | Single undifferentiated memory store; no cross-session FTS; un-fenced recalled context (injection risk) |

### Cross-cutting recommendations for UnivAgent
1. **One backend, many surfaces** (openclaw gateway + deepseek channel pattern): TUI, desktop GUI, and API all speak the same `EngineEvent`/`Op` protocol.
2. **Memory = layered, not monolithic** (hermes): keep working/session/long-term/user-model/procedural tiers; fence recalled context.
3. **Isolation = OS sandbox, not denylist** (deepseek > hermes): use Landlock/Seatbelt/container; denylists are a secondary guard.
4. **Streaming = decoupled + coalesced** (deepseek): never block render on the model; cap FPS; chunk markdown at grapheme boundaries.
5. **Skills = folders + registry + self-improvement** (openclaw + hermes): versioned distribution (ClawHub-style) and let the agent author its own skills, but **scan by default**.
