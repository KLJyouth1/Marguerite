# Marguerite File Intelligence Workbench (web)

Frontend SPA for the Marguerite File Intelligence capabilities. It consumes the
FastAPI backend (port **8011**) defined in `api-contract-marguerite-file-intel`
and conforms to Bloom Mode visual gates (brand blue `#3B82F6` anchor, design
tokens, SVG-only icons, no indigo→pink gradient, no bounce easing).

## Stack
- React 18 + TypeScript (strict, no `any`) + Vite
- Zustand for UI state
- Typed `fetch` client against `http://localhost:8011/api`
- Component registry (`Record<PanelId, ComponentType>`) for extensibility

## Run

```bash
npm install
npm run dev        # http://localhost:5173
```

The dev server proxies `/api` → `http://localhost:8011`, so no CORS setup is
required locally. For a production build, set `VITE_API_BASE` (e.g. `/api` or
`http://localhost:8011/api`) — the backend must have CORS enabled when the SPA
calls it cross-origin.

```bash
npm run build      # tsc -b && vite build  (must be clean)
npm run preview    # serve the production build
```

## Panels → endpoints

| Panel | Endpoint(s) |
|---|---|
| **Overview** (landing) | `GET /health` (outside `/api`), `GET /harness/capabilities`, `GET /memory/layers`, `GET /experts`, `GET /harness/adapters` — live telemetry, zero forms |
| **Orchestrate** | `POST /agent/run`, `GET /events/recent` — agent-loop step timeline (plan→act→observe→repair) |
| **Model Lab** | `POST /llmlab/tokenize`, `POST /llmlab/forward` — BPE merge trace + real causal attention heatmaps |
| **Files** | `POST /files/load`, `GET /files/{id}` (display), `POST /files/{id}/modify`, `POST /files/{id}/collapse`, `POST /files/{id}/ask`, `POST /files/{id}/interact` |
| **Memory** | `GET /memory/layers`, `GET /memory/{layer}/query`, `POST /memory/{layer}/store` |
| **Experts** | `GET /experts`, `POST /experts/{role}/analyze` |
| **Knowledge** | `GET /kb/search`, `POST /kb/ingest` |
| **Harness** | `GET /harness/adapters`, `POST /harness/dispatch` |
| **Research** | `POST /research` |

The two non-`/api` endpoints are reached via `../health` /
`../harness/capabilities` relative to the configured API base; the Vite dev
server proxies them alongside `/api`.

## Project layout
```
src/
  api/client.ts        typed fetch wrapper + ApiError
  components/          FileCard, FileSection (+ section registry), DiffView, ExpertAnalysisView, ui/*
  features/           one panel per capability (OverviewPanel = live telemetry dashboard)
  hooks/useAsync.ts   loading/error/data lifecycle
  registry.ts         PanelId → component registry (overview is the landing view)
  store/workbenchStore.ts  active panel + toasts + theme
  styles/tokens.css   Bloom design tokens (only place raw hex lives)
  types/contract.ts   shapes mirrored from the API contract
```

## Notes
- Every panel renders honest empty / loading / error states.
- Icon-only buttons carry `aria-label`; no paste interception anywhere.
- The rendered file artifact is sanitized with DOMPurify before injection (Bloom R14).
