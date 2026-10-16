# Marguerite Unified Workbench Merge Plan

> Version 1.0 | 2026-09-01
> Status: COMPLETED — 7/7 gates green
> Betting Agreement: 100 points, any defect deducts

## 1. Executive Summary

Merge `D:\Downloads\Marguerite\` (root) and `D:\Downloads\Marguerite\marguerite\` (Rust workspace) into a single unified project root. The `marguerite/` subdirectory becomes the single project root, with all root-level assets moved in and deprecated projects archived.

## 2. Four-End Merge Verdict

Based on FINAL-DESIGN.md, cli-workbench/spec.md, design-workbench-v2.md, and WorkBuddy memory (2026-08-30/31, 2026-09-01):

| End | Current State | Merge Verdict | Rationale |
|-----|--------------|---------------|-----------|
| **CLI** | `core/src/cli/` - 19 routes, first citizen | KEEP AS-IS | CLI is first citizen, zero UI, script-driven. Already in Rust workspace. |
| **TUI** | `tui/` - ratatui three-column | KEEP AS-IS | TUI shares 36-slot contract, same slash commands, same Envelope event flow. Already in Rust workspace. |
| **GUI** | `gui/src-tauri/` (Tauri+Vue3) + `marguerite-desktop/` (Electron) | UNIFY TO TAURI. Archive Electron. | Tauri is Cargo workspace member, shares build with core. Electron is 942MB standalone, duplicates Tauri role. FINAL-DESIGN.md confirmed: Tauri is the desktop shell. |
| **Web** | `webui/` (Vue3, 190+ components) + `marguerite-agent-web/` (React) | UNIFY TO VUE3 WEBUI. Archive React. | webui has workbench/ registry/runtime/store, 3129 tests passing. React app is 1.1GB, TDesign React, separate Express server. FINAL-DESIGN.md confirmed: Vue3 webui is the primary frontend. |

### Deprecated Projects (Archive, Don't Delete)
1. `marguerite-agent-web/` (1.1GB) → `_archive/marguerite-agent-web/`
2. `marguerite-desktop/` (942MB) → `_archive/marguerite-desktop/`
3. `marguerite-agent-web-v1.0.tar.gz` + `.zip` → `_archive/`
4. `bloom-mode-shader-prototype/` → `experiments/bloom-shader/`
5. Root `node_modules/` (315MB) → remove (not needed)
6. Root `package.json` + `package-lock.json` → remove (placeholder)

## 3. Directory Merge Map

### 3.1 Assets to Move INTO marguerite/

| Root Asset | Destination | Action |
|-----------|-------------|--------|
| `.trae/` (skills, specs) | `marguerite/.trae/` | MOVE - contains agent-engineering-core skill, cli-workbench specs |
| `.workbuddy/` (memory, artifacts) | `marguerite/.workbuddy/` | MOVE - development journal, pitfalls |
| `.design/` | `marguerite/.design/` | MOVE - if exists |
| `.dsh-home/` | `marguerite/.dsh-home/` | MOVE - DSH plugin home |
| `.dumate/` | `marguerite/.dumate/` | MOVE - DuMate integration |
| `deliverables/` (opsdev-workbench) | merge with `deliverables/` | MERGE - root has opsdev-workbench PRD/Architecture/FINAL-DESIGN |
| `docs/` | merge with `docs/` | MERGE - root has bloom-mode, univagent docs |
| `workbench/` (backend, frontend, packaging) | `marguerite/workbench/` | MOVE - workbench design assets |
| `scripts/` | merge with `scripts/` | MERGE - verify.sh, QA probes |
| `third_party/` | merge with `third_party/` | MERGE - external repos, computer_use, harness |
| `dsh-data/` | `marguerite/dsh-data/` | MOVE |
| `mgt_reuse/` | `marguerite/mgt_reuse/` | MOVE |
| `pages/` | `marguerite/pages/` | MOVE |
| `patches/` | `marguerite/patches/` | MOVE |
| `dist/` | REMOVE | Build artifacts, regenerated on build |
| Root `.md` files (overview, INTEGRATION, bloom-mode-*, univagent-*) | `docs/` | MOVE - consolidate documentation |
| Root QA scripts (`qa_*.cjs`) | `scripts/qa/` | MOVE - consolidate QA tools |
| `启动.bat` / `启动.ps1` | `scripts/launch/` | MOVE - launcher scripts |

### 3.2 Consolidation Rules

1. **No overwrite**: If a file exists in both locations, keep the marguerite/ version (it's in git)
2. **Merge directories**: For docs/, deliverables/, scripts/, third_party/ - copy unique files from root into marguerite/
3. **Preserve .git**: The marguerite/.git remains the single source of truth
4. **Archive don't delete**: Deprecated projects go to `_archive/` for rollback safety

## 4. Unified Architecture (Post-Merge)

### 4.1 Layered Architecture (L0-L5)

```
L0 Access Layer
  ├── CLI (core/src/cli/) - first citizen, zero UI
  ├── TUI (tui/) - ratatui three-column
  ├── GUI (gui/src-tauri/) - Tauri + Vue3 webui
  └── Web (webui/) - Vue3, 190+ components, workbench registry
         ↓
L1 Transport Layer
  ├── gateway/ (axum HTTP + WS JSON-RPC, loopback only)
  ├── methods.rs (dispatch table)
  ├── events.rs (Envelope event stream + SSE)
  └── long_running.rs (async task management)
         ↓
L2 Orchestration Layer
  ├── intent/ (NL → intent classification)
  ├── plan/ (ChainPlan step decomposition)
  ├── orchestration/ (DAG executor)
  ├── router/ (model routing: deepseek-v4-flash/pro, Omni score)
  └── batch/ (batch host execution)
         ↓
L3 Capability Layer
  ├── Ops: ssh_pool, ssh_transport, panel, file, ops/
  ├── Dev: codeintel, context (L0-L3 budget, 32k cap), kb, cache, dev/
  └── Cross: skills/, plugins/ (DSH 10 plugins, JIT skill packs)
         ↓
L4 Governance Layer (cross-cutting)
  ├── rbac/ (6 roles, policy engine)
  ├── audit_store/ (append-only hash chain, audit.db)
  ├── risk/ (risk assessment)
  ├── limiter/ (rate limiting, 3-level circuit breaker)
  └── rollback/ (side-git snapshots, .versions/)
         ↓
L5 Storage Layer
  ├── credentials/ (AES-256-GCM, BYOK)
  ├── persistence/ (SQLite WAL)
  ├── audit.db (append-only, trigger-based)
  └── codeintel.db (FTS5 full-text search)
```

### 4.2 Agent Calling Logic

```
User Input
    ↓
CLI/TUI/GUI/Web → Envelope event
    ↓
Intent Classification (intent/)
    ↓
Mode Selection: plan | agent | bloom
    ├── plan: read-only, no side effects
    ├── agent: tool cycle with guards
    └── bloom: bloom/bloom:auto/bloom:swarm
    ↓
Model Router (router/)
    ├── Complexity prediction → model tier
    ├── Omni score (omni.rs, 233 lines, 12 tests)
    ├── Bloom failover (bloom_failover)
    └── Provider preflight (deepseek-v4-flash/pro)
    ↓
Agent Runtime (agent/)
    ├── Tool cycle (tools/, MCP)
    ├── Guard rails (guard/, 4 layers)
    │   ├── L1: input validation
    │   ├── L2: rule engine DSL
    │   ├── L3: anti-jailbreak
    │   └── L4: output hallucination filter
    ├── Context engine (context/, L0-L3 budget)
    │   ├── L0: system prompt
    │   ├── L1: static zone
    │   ├── L2: interjection
    │   └── L3: dynamic history
    └── Memory (memory/, 3-layer)
        ├── MEMORY.md (index)
        ├── topics/ (subject files)
        └── daily logs
    ↓
Token Reduction (7 strategies)
    1. Prefix-cache friendly layout
    2. Layered context (L0-L3, 32k hard cap)
    3. 4-level result cache
    4. Model 3-tier routing
    5. Template output
    6. Bounded retry + 3-level circuit breaker
    7. Run-level token ledger
    ↓
Output → Envelope → CLI/TUI/GUI/Web
```

### 4.3 Frontend Design (Vue3 webui)

```
webui/src/
  ├── App.vue (1780 lines, was 2033, B2 split done)
  ├── workbench/
  │   ├── registry/ (36-slot contract)
  │   ├── runtime/ (panel lifecycle)
  │   └── store/ (Pinia state)
  ├── components/ (190+ Vue components)
  ├── composables/ (Vue composition API)
  ├── views/ (page-level components)
  ├── router/ (vue-router)
  ├── stores/ (Pinia)
  ├── styles/ (CSS variables, --ow-* tokens)
  ├── themes/ (light/dark, TDesign bridge)
  ├── i18n/ + locales/ (vue-i18n, setup.* prefix)
  ├── platform/ (Tauri IPC bridge)
  └── app/ (app-level config)
```

### 4.4 Design Tokens

- Icon system: Lucide (lucide-vue-next), NO emoji, NO hardcoded colors
- Color: TDesign variable bridge `--td-*` → `--ow-*`
- Motion: NO bounce easing, NO purple-pink gradients
- Layout: Three-column 1:1 TRAE style
- i18n prefix: `setup.<panel-id>.*` (NOT `settings.*`)
- Emoji detection: exact match `{0x200D, 0x20E3}` (NOT range)

### 4.5 Backend Design (Rust core)

```
core/src/ (65 modules)
  ├── Gateway: gateway/ (axum, loopback), methods.rs, events.rs
  ├── Intent: intent/ (NL classification)
  ├── Agent: agent/ (runtime, tool cycle)
  ├── Model: model/ (BYOK, omni.rs, router)
  ├── MCP: mcp/ (Model Context Protocol)
  ├── Memory: memory/ (3-layer ledger)
  ├── Guard: guard/ (4-layer rails)
  ├── Context: context/ (L0-L3 budget)
  ├── Skills: skills/ (self-evolution)
  ├── Plugins: plugins/ (DSH 10, manifests)
  ├── Ops: ops/ (remote ops, 20 tests)
  ├── Dev: dev/ (code intelligence, 8 tests)
  ├── RBAC: rbac/ (6 roles, 5 tests)
  ├── Audit: audit_store/ (hash chain, 5 tests)
  ├── Credentials: credentials/ (AES-256-GCM, 23 tests)
  └── Usage: usage/ (token ledger, 40 tests)
```

### 4.6 JSON-RPC Method Families

```
ops.hosts.*      - host management
ops.terminal.*   - remote terminal
ops.files.*      - file operations
ops.monitor.*    - monitoring
panel.*           - Baotao proxy (experimental, default off)
agent.task.*     - task execution
agent.context.*  - context management
agent.cache.*    - cache management
team.rbac.*       - role-based access
team.shared.*    - shared resources
team.audit.*      - audit trail
```

## 5. Milestone Status (from WorkBuddy memory)

| Milestone | Status | Tests |
|-----------|--------|-------|
| M0 Foundation | DONE | p0-gate: emoji 25/hex 36/gradient 0/easing 4/placeholder 3 |
| M1 Security Base | DONE | RBAC 6, audit.db 5, credentials 23, seam 5, rate_limit 8 |
| M2 Cost Reduction | IN PROGRESS | usage 40, prompt_layout 6, dev 8, hitcache 6, selfcheck 3 |
| M3 Remote Ops | CORE DONE | ops 20, dispatch 4, rollback 8 (real SSH/WS pending) |
| M4 Smart Coding | PARTIAL | GoalPanel 4, Hunyuan archive (real SSH pending) |
| M5 Integration | NOT STARTED | - |

Full regression: `cargo test --lib` = 2412 passed / 0 failed / 17 ignored
Frontend: vitest = 3148 passed / 305 files / 0 regression

## 6. Execution Steps

### Step 1: Archive Deprecated Projects
- Move marguerite-agent-web/ → _archive/
- Move marguerite-desktop/ → _archive/
- Move marguerite-agent-web-v1.0.* → _archive/
- Move bloom-mode-shader-prototype/ → experiments/
- Remove root node_modules/, package.json, package-lock.json, dist/

### Step 2: Move Root Assets INTO marguerite/
- .trae/ → marguerite/.trae/
- .workbuddy/ → marguerite/.workbuddy/
- deliverables/ → merge with deliverables/
- docs/ → merge with docs/
- workbench/ → marguerite/workbench/
- scripts/ → merge with scripts/
- third_party/ → merge with third_party/
- Root .md files → docs/
- Root QA scripts → scripts/qa/
- dsh-data/, mgt_reuse/, pages/, patches/ → marguerite/

### Step 3: Update Configuration
- Update AGENTS.md with new unified structure
- Update Cargo.toml workspace paths if needed
- Update gui/src-tauri/tauri.conf.json with new paths
- Update webui build scripts if needed

### Step 4: Verify Build
- `cargo check -p marguerite-core` → zero errors ✓
- `cargo test --lib` → 2412 passed / 0 failed / 17 ignored ✓
- `cd webui && npm run typecheck` → zero errors ✓ (10 architecture guards passed)
- `cd webui && npm run test:unit` → 3148 passed / 305 files ✓
- `cargo build --workspace` → zero errors ✓ (core + gui + tui)
- `cargo clippy --workspace --all-targets` → zero warnings ✓
- `bash scripts/verify.sh --all` → 7/7 PASS ✓

## 7. Quality Gates (agent-engineering-core R2)

- R2 Quality Gate: cargo check zero errors + full test green + new code zero warnings + contract guards + architecture guards + clean full gate
- R8 Delivery Completeness: no placeholders, no TODOs in shipped code
- R9 Orchestration: dispatch ≠ complete, sub-agent returns must be independently verified
