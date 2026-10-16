# Marguerite File Intelligence Workbench — Security Audit & Hardening Spec

**Date**: 2026-08-14
**Auditor**: gstack-security-officer (CSO)
**Method**: OWASP Top 10 + STRIDE, grounded against live Marguerite code + the `api-contract-marguerite-file-intel-2026-08-14.md` contract.
**Status of code under audit**: backend (`fastapi-pro`) / frontend (`frontend-developer`) still in build → this is a **design/pre-ship** audit; findings cite the live secure pattern each rule must mirror and the contract clauses that already mandate it.

---

## 1. Executive Summary

Severity distribution (design-phase, pre-ship): 🔴 0 / 🟠 6 / 🟡 5 / 🟢 (baseline patterns exist).

The contract (lines 9–11) already encodes the right security posture (mandatory `safe_resolve`, env-only keys, deterministic no-key fallback, SSRF-aware 联网). The risks are in **enforcement gaps** and **missing details**:

1. **Symlink escape not closed by `safe_resolve`** — live `fs_path.rs` canonicalizes the *root* but not the *resolved* path, so a symlink inside the workspace pointing outward still passes the prefix check. 🟠
2. **Live builtin tools bypass `safe_resolve`** — `ReadFileTool`/`EditFileTool` call raw `std::fs::read_to_string`/`write` (`builtin.rs:49`, `builtin.rs:81`). If the workbench reuses these via the MCP bridge, traversal slips through. 🟠
3. **`/harness/dispatch` subprocess risk** — external CLIs (codex/opencode/aider) invoked with a user `task` string; must forbid `shell=True`, confine cwd, timeout, cap output. 🟠
4. **Prompt injection** — file/KB/research content flows into the LLM for `/modify`, `/ask`, `/interact`; malicious content could hijack instructions or trigger unsandboxed writes. 🟠
5. **SSRF in `/research`** — 联网 fetch must block internal IP ranges and metadata endpoint, cap size/redirects. 🟠
6. **XSS** — `/files/{id}` returns an HTML artifact; Bloom R14 requires DOMPurify on frontend + escaping on backend. 🟠

Plus 🟡 items: change-ledger stores full `before/after` of edited files (secret leakage to frontend RPC), CSRF on state-changing ops if bound beyond localhost, missing input-size/rate limits (DoS), secret-in-logs risk.

---

## 2. STRIDE Table

| Element | Threat | Mitigation | Severity |
|---|---|---|---|
| `safe_resolve` (`fs_path.rs:11`) | **Tampering / Escape** — symlink inside workspace → outside; prefix check passes | Canonicalize the *resolved* path (`std::fs::canonicalize`/Python `os.path.realpath`) and re-verify `starts_with(root)`; reject if final target differs from resolved | 🟠 |
| `/api/files/load` (`{path}`) | **Spoofing/Elevation** — path traversal (`../`, absolute, symlink) → workspace escape | Backend MUST route every path through `safe_resolve` equivalent before any fs op; reject on error; never forward raw path to raw fs calls | 🟠 |
| `/api/files/{id}/modify` | **Elevation** — prompt injection makes agent write outside scope / to `.git`/secrets | `safe_resolve` on target; write only inside workspace; record in change ledger (`file_change.rs:45`); require backup + explicit apply flag; LLM output is instructions, never direct exec | 🟠 |
| `/api/harness/dispatch` (`task`) | **Injection/Elevation** — command injection via `task` string; sandbox breakout | `subprocess.run([...], shell=False, cwd=workspace, timeout=...)`; never string-concatenate into shell; cap stdout/stderr bytes; run external CLIs as `internal` fallback when not sandboxed | 🟠 |
| `/api/research` (联网) | **Information Disclosure / SSRF** — fetch internal service / cloud metadata | Allow-list scheme `http(s)`; block `127.0.0.0/8`, `10/8`, `172.16/12`, `192.168/16`, `169.254.169.254`, `::1`, `fc00::/7`; timeout; disable/limit redirects; cap response size | 🟠 |
| `/api/files/{id}` (HTML) | **Tampering/XSS** — file content rendered as HTML | Backend escapes content (HTML-escape, don't inject raw); frontend DOMPurify before `dangerouslySetInnerHTML` (Bloom R14 `bloom...md:72`) | 🟠 |
| `/api/kb/ingest` + `/ask`/`/interact` | **Spoofing** — prompt injection from ingested/asked content | Delimit untrusted content (`<document>…</document>`), system instructions outside; never let content invoke tools/writes without approval; sanitize LLM output as text, not code | 🟠 |
| `FileChange` ledger (`file_change.rs:19-32`) | **Information Disclosure** — `before`/`after` of a secret file persisted & exposed via RPC | Skip recording full content for paths matching secret patterns (`*.env`, `*.key`, `credentials*`, `.git/`); or store diff-only + redact | 🟡 |
| State-changing POSTs (`modify/dispatch/ingest/store`) | **Spoofing (CSRF)** — if bound beyond localhost | Bind to `127.0.0.1`; if `0.0.0.0` needed, require CSRF/session token + `SameSite=strict`; JWT via HttpOnly cookie (Bloom §6 `bloom...md:114`) | 🟡 |
| Env keys (`UNIV_LLM_KEY` etc.) | **Info Disclosure** — keys logged or returned | Keys read from env only (contract:10); never log; never serialize in API responses/errors; scrub from tracebacks | 🟡 |
| `/api/memory/{layer}/store`, `/kb/ingest` | **DoS** — unbounded text / query | Enforce max body size (e.g. 1 MB ingest, 8 KB query); per-IP/per-session rate limit; cap chunk count | 🟡 |
| `workspace.file_changes.list` RPC | **Repudiation** — no actor/audit trail | Attach session/actor id + ts to each change record; log security-relevant events (auth fail, denied path, blocked SSRF) | 🟡 |

---

## 3. OWASP Top 10 Coverage Checklist (mapped to endpoints)

| OWASP | Control | Endpoints / Where | Status |
|---|---|---|---|
| **A01 Broken Access Control** | Workspace confinement + auth for state-changing ops | all `/files/*`, `/harness/*`, `/kb/*`, `/memory/*` | 🟠 enforce `safe_resolve` + bind localhost / token |
| **A02 Cryptographic Failures** | TLS if remote; secrets in env; no cleartext keys | `/research` (https only), env keys | 🟢 contract mandates env-only keys |
| **A03 Injection** | No `shell=True`; parameterized; path not interpolated | `/harness/dispatch`, `/files/load`, `/kb/ingest` | 🟠 subprocess + path rules still needed |
| **A04 Insecure Design** | Rate limit, input caps, approval gate on writes | `/modify`, `/kb/ingest`, `/research` | 🟡 missing size/rate limits |
| **A05 Security Misconfiguration** | No debug banner; safe error messages; CSP | whole app, frontend render | 🟠 set CSP + escape; avoid leaking internals |
| **A06 Vulnerable Components** | Pin deps; audit (Gitleaks/dep-audit per Bloom L2) | Py/TS deps | 🟢 process gate, verify in CI |
| **A07 Auth Failures** | Deterministic no-key fallback; token if remote | `/research`, `/experts/{role}/analyze` | 🟡 add token when non-localhost |
| **A08 Integrity Failures** | Sandbox external CLIs; sign/verify if fetching adapters | `/harness/dispatch` | 🟠 sandbox + no untrusted exec |
| **A09 Logging Failures** | Don't log keys/prompts/file contents; log denials | all handlers | 🟠 add structured security logs |
| **A10 SSRF** | Block internal ranges + metadata; scheme allow-list | `/research` (联网) | 🟠 explicit blocklist required |

---

## 4. Hardened Rules — Backend MUST Follow (with live citations)

> Each rule has a corresponding live Marguerite implementation to mirror. New backend (FastAPI) must re-implement the *equivalent* in Python; do **not** forward raw paths/strings to raw OS calls.

**R-PATH-1 (Path safety).** Every file path from any endpoint MUST pass a `safe_resolve(root, rel)` step *before* `open`/`read`/`write`. Mirror `fs_path.rs:11-38`. **Critical fix**: after building the resolved path, canonicalize it and re-check `starts_with(root_can)` — the live version (fs_path.rs:34-36) only checks the *string* prefix and thus **misses symlink escape**. Python: `real = os.path.realpath(os.path.join(root, rel)); assert real.startswith(os.path.realpath(root))`.

**R-PATH-2 (No raw fs in bridge).** Do not reuse `ReadFileTool`/`EditFileTool` directly — they call `std::fs::read_to_string(path)` (`builtin.rs:49`) and `std::fs::write(path, content)` (`builtin.rs:81`) **without** `safe_resolve`. The Python backend is the single enforcement point.

**R-PATH-3 (Record & backup).** Every real write must (a) record in a change ledger mirroring `record_file_change` (`file_change.rs:45`) with bounded retention (`MAX_CHANGES=200`, `file_change.rs:15`), and (b) produce a `backup_ref` (contract `/modify` returns `backup_ref`). Never record full `before/after` for secret paths.

**R-SUB-1 (Subprocess).** `/harness/dispatch` MUST call the CLI via `subprocess.run([cmd, *args], shell=False, cwd=workspace_root, timeout=300, capture_output=True)`. The `task` string is **one argv element**, never interpolated into a shell command line. Cap `stdout`/`stderr` to e.g. 512 KB; truncate with marker.

**R-SUB-2 (Sandbox).** Prefer the adapter's own kernel/Docker sandbox; if none available, fall back to `internal` Marguerite loop (contract §harness). Never grant the subprocess network write to internal ranges.

**R-XSS-1 (Backend).** `/files/{id}` HTML must be produced by escaping file content (`html.escape`) and only injecting trusted highlight spans; never echo raw file bytes into the markup.

**R-XSS-2 (Frontend, Bloom R14).** Before `dangerouslySetInnerHTML`, pass through DOMPurify; CSP `script-src 'self'`. (Cite `bloom-mode-engineering-standard-v2.0.md:72` and `:114`.)

**R-SSRF-1.** `/research` allow-list `http`/`https` only; reject on resolve to `127.0.0.0/8`, `10/8`, `172.16/12`, `192.168/16`, `169.254.0.0/16` (incl. `169.254.169.254`), `::1`, `fc00::/7`; `timeout=15s`; `max_redirects<=3` and re-validate each hop; cap response to 1 MB.

**R-SECRET-1.** Keys via env (`UNIV_LLM_KEY` etc., contract:10). Read once at startup; never `print`/`log`/`repr` them; strip from exception messages and API envelopes.

**R-INJECT-1 (Prompt).** Wrap untrusted file/KB content in clear delimiters and keep system instructions outside that region; treat LLM output as *data* — never auto-execute; gate any write behind explicit user approval + `safe_resolve`.

**R-DOS-1.** Enforce body caps (ingest ≤1 MB, query ≤8 KB, `instruction` ≤4 KB) and per-session rate limits on `/modify`, `/research`, `/kb/ingest`.

---

## 5. Prioritized Action List (enforce before ship)

**P0 (block ship)**
1. Implement Python `safe_resolve` with post-canonicalize prefix re-check (closes symlink escape, R-PATH-1). Wire `/files/load` + `/files/{id}/modify` to it.
2. `/harness/dispatch`: `shell=False` + argv list + cwd confinement + timeout + output cap (R-SUB-1/2).
3. `/research` SSRF blocklist + scheme allow-list + redirect re-validation (R-SSRF-1).
4. `/files/{id}` XSS: backend escape + frontend DOMPurify + CSP (R-XSS-1/2).

**P1 (before public exposure)**
5. Prompt-injection delimiters + approval gate on LLM-driven writes (R-INJECT-1).
6. Secret handling: env-only, no logs, scrub from errors (R-SECRET-1).
7. Change ledger: skip/redact secret-path content; bounded retention (R-PATH-3).
8. Input-size + rate limits on ingest/modify/research (R-DOS-1).
9. Bind to `127.0.0.1` by default; add CSRF/session token if `0.0.0.0` required (A07/A01).
10. Structured security logging for denials/SSRF-blocks/auth-failures (A09).

---

*Audit grounded in: `core/src/.../fs_path.rs`, `file_change.rs`, `builtin.rs`, `api-contract-marguerite-file-intel-2026-08-14.md`, `bloom-mode-engineering-standard-v2.0.md` §6 + R14. No external LLM APIs were invoked; findings are design-level and cite reproducible live-pattern gaps.*

---

## 6. Cross-Team Verification Addendum (2026-08-14, post `fastapi-pro` integration)

The backend (`marguerite_workbench/`) was built against this audit. CSO performed **code-level active verification** of every P0 rule against the delivered source (runtime pytest could not be executed in the audit sandbox — no Python interpreter on PATH — so confirmation is by reading the exact implementation, which is conclusive for these surfaces).

### 6.1 CONFIRMED CLOSED (verified in source)

| Audit item | Where verified | Evidence |
|---|---|---|
| #1 symlink escape | `security.py:25-58` | `candidate.resolve()` (lines 43,47) follows symlinks; `candidate.relative_to(root_p)` (line 51) raises `ValueError` if the *real* target escapes. This is exactly the post-canonicalize recheck `fs_path.rs` lacked. **Symlink gap is NOT repeated.** |
| #1 traversal + absolute | `security.py:40-43, 51-56` | absolute outside root and `..` both rejected via `relative_to`. |
| #2 no raw-fs bridge | `api.py:84,107,201-203`, `harness.py:161` | every file op funnels through `safe_resolve` before `read_text`/`write_text`; no raw `open`/`write` on user path. |
| #3 subprocess safety | `harness.py:110-117` | argv list `[cmd, *args, task]` (line 112); `shell=False`, `cwd=str(workspace)`, `capture_output=True`, `timeout=DISPATCH_TIMEOUT_S` (120). `task` is one argv element → **no command injection**. |
| #3 internal-edit safety | `harness.py:158-182` | edit target routed through `safe_resolve`; missing/unsafe path → `status="error"`, no mutation. |
| #6 XSS (backend half) | `file_intel.py:261-273` | `highlight()` uses `html.escape(text)` fallback (line 273); pygments path escapes text by default (line 269). No raw file bytes injected. |
| Secret handling | `llm.py:30` | key read from `os.environ.get("UNIV_LLM_KEY")` only; no `print`/`logger` of key; `conftest.py:26` purges key vars in tests. |

### 6.2 RESIDUAL — must fix before any non-localhost exposure

**R-1 (🟠, CORS + bind).** `api.py:60-64` sets `allow_origins=["*"]` **with** `allow_credentials=True`, and `api.py:283` binds `uvicorn` to `host="0.0.0.0"`. The `*`+credentials combo lets **any website** make credentialed cross-origin calls to the workbench (CSRF / localhost-CSRF / DNS-rebinding). This is worse than the audit assumed (audit rated CSRF 🟡 on a localhost-only assumption). **Action**: default bind `127.0.0.1`; restrict CORS origins to `http://localhost:*` / `http://127.0.0.1:*`; only widen when an explicit opt-in flag is set.

**R-2 (🟡, harness output cap).** `harness.py:118` captures `proc.stdout` unbounded; a verbose CLI can bloat process memory / response. `steps[:OBSERVE_LIMIT]` caps *lines returned* but not the captured buffer. **Action**: truncate stdout/stderr to a fixed ceiling (e.g. 512 KB) before splitting.

### 6.3 LOW-RISK-BY-DESIGN (no action unless feature grows)

**#5 SSRF.** `research.py:40-42` POSTs only to a **hardcoded** `https://google.serper.dev/search` with the user `query` as JSON body — it never fetches a user-supplied URL. The classic "fetch arbitrary internal URL" SSRF path therefore **does not exist** in the current code. Keep `R-SSRF-1` (internal-IP blocklist + scheme allow-list + redirect re-check) ready to wire in *before* any future "fetch this URL" capability is added; do not ship such a capability without it.

**#4 prompt injection / #7 CSRF** remain design-level recommendations; the backend has no write-without-confirmation path for `/modify` via LLM (rule-engine fallback, `file_intel.py:397`), which is good. Frontend XSS half (DOMPurify) is `frontend-developer`'s outstanding item — see §4 R-XSS-2.

### 6.4 Updated severity tally (backend-verified)
Originally 🔴 0 / 🟠 6 / 🟡 5. After `fastapi-pro` integration + CSO verification:
- #1, #2, #3 → **CLOSED** (backend).
- #6 backend half → **CLOSED**; frontend half → outstanding (frontend-developer).
- #5 → **low-risk by design** (verify before URL-fetch feature).
- **NEW R-1 (CORS+bind)** → 🟠, P1 (fix before non-localhost).
- #4, #7, R-2, secret/ledger/DoS → carry as P1 design guidance.
