# Marguerite File Intelligence Workbench

A self-contained, dependency-light **FastAPI** backend (port **8011**) that
extends the live Marguerite/UnivAgent Rust core with a runnable File Intelligence
Workbench. Every endpoint is a *real, tested* implementation — no stubs, no
shells. The system is fully callable **with zero API keys**: every LLM- and
network-dependent path has a deterministic fallback.

## Run

```bash
pip install -r requirements.txt
uvicorn marguerite_workbench.api:app --port 8011
```

Then open the Swagger UI at <http://localhost:8011/docs>.

> Run tests (offline, no keys) with:
> ```bash
> python -m pytest marguerite_workbench/tests -q
> ```

## CLI

Every capability is also reachable from the command line — the CLI talks to the
same engine objects as the HTTP app (`engine.Workbench`), so results are
identical without an HTTP hop:

```bash
python -m marguerite_workbench doctor                 # probe deps/dirs/bridges
python -m marguerite_workbench serve                  # FastAPI on 127.0.0.1:8011
python -m marguerite_workbench files load --path src/app.py
python -m marguerite_workbench files display --id src/app.py
python -m marguerite_workbench files modify --id src/app.py --instruction "rename variable x to total"
python -m marguerite_workbench memory store L3 --content "prefers concise answers" --ref chat-42
python -m marguerite_workbench memory query L3 --q "answer style" -k 5
python -m marguerite_workbench kb ingest --path notes/bloom.md --doc-id bloom-notes
python -m marguerite_workbench kb search --q "quality gates"
python -m marguerite_workbench experts analyze security_officer --payload '{"code": "eval(x)"}'
python -m marguerite_workbench harness dispatch "inspect sample.py and summarise its structure" --adapter internal
python -m marguerite_workbench research "bloom mode quality gates"

# --- UPGRADE_PLAN_V2 pillars ---
python -m marguerite_workbench llmlab tokenize --text "attention is all you need" --vocab 264
python -m marguerite_workbench llmlab forward  --text "hello world" --dim 32 --heads 4   # + attention telemetry
python -m marguerite_workbench llmlab sample   --text "abc" --temperature 0.8 --top-p 0.95 --max-new 16
python -m marguerite_workbench ttc run   --strategy self_consistency --prompt "..." -n 5
python -m marguerite_workbench ttc budget "prove concurrency correctness"
python -m marguerite_workbench agent run "read note.txt"          # plan→act→observe→repair
python -m marguerite_workbench agent log s_20260824_155236_4047bb # replay a session transcript
```

Output contract (mirrors the Marguerite core CLI invariants):

- `--json` emits `{"ok": bool, "data": …, "meta": {cmd, duration_ms}}`;
  failures emit `{"ok": false, "error": {message, exit_code}, "meta"}` on stderr.
- Text mode uses ASCII `[OK] / [WARN] / [FAIL]` markers — never emoji.
- `NO_COLOR` is honoured; ANSI colour only on a TTY.
- Exit codes: `0` ok · `1` runtime · `2` usage · `3` not found · `4` rejected.

## What's inside

| Module | Responsibility |
|---|---|
| `api.py` | FastAPI app + every contract endpoint; CORS; startup `Workbench`. |
| `engine.py` | The `Workbench` context (workspace sandbox + memory/kb/file engines) shared verbatim by the API and the CLI. |
| `cli.py` / `__main__.py` | Full command-line interface (`python -m marguerite_workbench …`) over the same engines; `{ok,data,meta}` JSON envelope. |
| `_compat.py` | Version-guarded adapter for fastapi 0.115 × starlette ≥1.0 (translates legacy `on_startup/on_shutdown`, backfills `max_body_size`). No-op on older Starlette. |
| `llmlab/` | 支柱A — first-principles lab: byte-level BPE (`tokenizer.py`, minbpe semantics), pure-Python TinyGPT forward (`transformer.py`: causal SDPA + GELU MLP + weight-tied LM head, exports per-head attention), full sampling family (`sampler.py`). Zero dependencies. |
| `ttc.py` | 支柱B — test-time compute: self-consistency voting, best-of-N with pluggable scorer, difficulty→budget table aligned with DeepSeek thinking-mode constraints; honest `engine="fallback"` tagging without keys. |
| `agentloop.py` | 支柱C — Codex-style PLAN→ACT→OBSERVE→REPAIR→FINALIZE state machine over a table-driven sandboxed tool registry; `max_rounds`/`max_repairs` hard bounds; unresolved failures fail loudly. |
| `events.py` | EventBus (ring replay) + append-only SessionLog JSONL transcripts under `data/sessions/`. |
| `memory.py` | 7-layer memory engine (L1 sensory → L7 meta) + 256-dim FNV-1a `LocalEmbedding` (mirrors `core/src/memory/long_term.rs`). |
| `file_intel.py` | `load` / `display` / `modify` / `collapse` / `ask` / `interact` over files. |
| `kb.py` | BM25 + local-vector hybrid knowledge base (mirrors `core/src/kb/store.rs`). |
| `experts.py` | 5 distilled expert roles with a real `analyze()` (not a label). |
| `harness.py` | Codex-class harness bridge (codex/opencode/aider detection + internal plan→act→observe loop). |
| `research.py` | 联网 search via Serper, graceful local-KB fallback. |
| `llm.py` | Optional OpenAI-compatible client (`UNIV_LLM_*`) with `llm_or_fallback`. |
| `security.py` | `safe_resolve` — rejects `..` traversal, symlink escape, absolute paths outside the workspace (mirrors `core/src/tools/fs_path.rs`). |
| `config.py` | Env-driven `workspace` / `data` directories. |

> Dependency note: the environment resolves `fastapi==0.115.*` together with
> `starlette>=1.0`, a pair fastapi 0.115 cannot construct against unmodified
> (its router still passes removed `on_startup` kwargs). `_compat.install()`
> bridges exactly that gap at import time; upgrading to a fastapi release that
> targets starlette ≥1.0 natively makes the shim a documented no-op.

## Endpoints (contract)

- **Files**: `POST /api/files/load` · `GET /api/files/{id}` · `POST /api/files/{id}/modify`
  · `POST /api/files/{id}/collapse` · `POST /api/files/{id}/ask` · `POST /api/files/{id}/interact`
- **Memory**: `GET /api/memory/layers` · `POST /api/memory/{layer}/store` · `GET /api/memory/{layer}/query`
- **KB**: `POST /api/kb/ingest` · `GET /api/kb/search`
- **Experts**: `GET /api/experts` · `POST /api/experts/{role}/analyze`
- **Harness**: `GET /api/harness/adapters` · `POST /api/harness/dispatch`
- **Research**: `POST /api/research`

## Safety

- All file paths flow through `security.safe_resolve(workspace, path)`; escapes
  are rejected with HTTP 400.
- `modify` **always** writes a timestamped backup before mutating, and only
  applies a change when a safe rule/LLM edit actually succeeds.
- No file content is ever `eval`/`exec`'d; HTML display output is escaped.

## Quality gates (Bloom Mode R-rules)

- No empty `except`; failures are logged and degrade gracefully (R3).
- No bare magic strings/numbers without named constants (R5).
- Dispatch logic is **table-driven**, not nested `if` ladders (R1).
- Deterministic without keys; offline-first.
