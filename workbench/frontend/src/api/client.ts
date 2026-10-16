import type {
  AgentRunResult,
  AgentToolInfo,
  AskResult,
  CapabilitiesInfo,
  ExpertAnalysis,
  ExpertChatResult,
  ExpertMode,
  ExpertRole,
  ExpertTeam,
  FileCard,
  FileDisplay,
  ForwardResultDTO,
  HarnessAdapter,
  HarnessResult,
  HealthStatus,
  InteractResult,
  KBChunk,
  KBIngestInput,
  KBIngestResult,
  MemoryItem,
  MemoryItemInput,
  MemoryLayerInfo,
  ModifyResult,
  ResearchResult,
  TokenizeResult,
  TtcResult,
} from '../types/contract';

/**
 * Base URL for the Marguerite backend.
 * Defaults to the contract port (8011). Override with VITE_API_BASE for
 * production deployments or to route through the Vite dev proxy (set to '/api').
 */
const API_BASE: string = import.meta.env.VITE_API_BASE ?? 'http://localhost:8011/api';

export class ApiError extends Error {
  public readonly status: number;
  public readonly body: unknown;

  constructor(status: number, message: string, body?: unknown) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.body = body;
  }
}

function buildQuery(params: Record<string, string | number | undefined>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== '') {
      search.append(key, String(value));
    }
  }
  const qs = search.toString();
  return qs.length > 0 ? `?${qs}` : '';
}

async function http<T>(path: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`${API_BASE}${path}`, {
      method: init?.method ?? 'GET',
      headers: {
        'Content-Type': 'application/json',
        ...(init?.headers ?? {}),
      },
      body: init?.body,
      signal: init?.signal,
    });
  } catch (err) {
    if (err instanceof DOMException && err.name === 'AbortError') {
      throw err;
    }
    throw new ApiError(0, `Network error reaching backend at ${API_BASE}${path}`);
  }

  const isJson = response.headers.get('content-type')?.includes('application/json');
  const payload = isJson ? await response.json().catch(() => undefined) : await response.text().catch(() => undefined);

  if (!response.ok) {
    const message =
      typeof payload === 'object' && payload !== null && 'detail' in payload
        ? String((payload as { detail: unknown }).detail)
        : typeof payload === 'string' && payload.length > 0
          ? payload
          : `Request failed (${response.status} ${response.statusText})`;
    throw new ApiError(response.status, message, payload);
  }

  return payload as T;
}

function enc(id: string): string {
  return encodeURIComponent(id);
}

export const api = {
  /* ---------- Files ---------- */
  loadFile: (path: string): Promise<FileCard> =>
    http<FileCard>('/files/load', { method: 'POST', body: JSON.stringify({ path }) }),

  getFileDisplay: (fileId: string): Promise<FileDisplay> =>
    http<FileDisplay>(`/files/${enc(fileId)}`),

  modifyFile: (fileId: string, instruction: string): Promise<ModifyResult> =>
    http<ModifyResult>(`/files/${enc(fileId)}/modify`, {
      method: 'POST',
      body: JSON.stringify({ instruction }),
    }),

  collapseFile: (fileId: string, sectionIds: string[] | null): Promise<FileCard> =>
    http<FileCard>(`/files/${enc(fileId)}/collapse`, {
      method: 'POST',
      body: JSON.stringify({ section_ids: sectionIds }),
    }),

  askFile: (fileId: string, question: string): Promise<AskResult> =>
    http<AskResult>(`/files/${enc(fileId)}/ask`, {
      method: 'POST',
      body: JSON.stringify({ question }),
    }),

  interactFile: (fileId: string, message: string, sessionId: string | null): Promise<InteractResult> =>
    http<InteractResult>(`/files/${enc(fileId)}/interact`, {
      method: 'POST',
      body: JSON.stringify({ message, session_id: sessionId }),
    }),

  /* ---------- 7-layer memory ---------- */
  getMemoryLayers: (): Promise<MemoryLayerInfo[]> => http<MemoryLayerInfo[]>('/memory/layers'),

  storeMemory: (layer: string, item: MemoryItemInput): Promise<MemoryItem> =>
    http<MemoryItem>(`/memory/${enc(layer)}/store`, {
      method: 'POST',
      body: JSON.stringify(item),
    }),

  queryMemory: (layer: string, q: string): Promise<MemoryItem[]> =>
    http<MemoryItem[]>(`/memory/${enc(layer)}/query${buildQuery({ q })}`),

  /* ---------- Knowledge base ---------- */
  ingestKB: (body: KBIngestInput): Promise<KBIngestResult> =>
    http<KBIngestResult>('/kb/ingest', { method: 'POST', body: JSON.stringify(body) }),

  searchKB: (q: string, k = 5): Promise<KBChunk[]> =>
    http<KBChunk[]>(`/kb/search${buildQuery({ q, k })}`),

  /* ---------- Distilled experts ---------- */
  getExperts: (): Promise<ExpertRole[]> => http<ExpertRole[]>('/experts'),

  /** The 9 canonical modes (3 intensities × 3 working modes) for the selector. */
  getExpertModes: (): Promise<ExpertMode[]> => http<ExpertMode[]>('/experts/modes'),

  /** The eight expert teams (+ core group) with their member role ids. */
  getExpertTeams: (): Promise<ExpertTeam[]> => http<ExpertTeam[]>('/experts/teams'),

  analyzeExpert: (
    role: string,
    payload: Record<string, unknown>,
    mode = 'standard_ask',
  ): Promise<ExpertAnalysis> =>
    http<ExpertAnalysis>(`/experts/${enc(role)}/analyze`, {
      method: 'POST',
      body: JSON.stringify({ payload, mode }),
    }),

  /** Resolve @mentions in chat text and analyze each referenced expert under a mode. */
  chatExperts: (
    text: string,
    mode = 'standard_ask',
    defaultRole?: string | null,
  ): Promise<ExpertChatResult> =>
    http<ExpertChatResult>('/experts/chat', {
      method: 'POST',
      body: JSON.stringify({ text, mode, default_role: defaultRole ?? null }),
    }),

  /* ---------- Codex-class harness ---------- */
  getHarnessAdapters: (): Promise<HarnessAdapter[]> => http<HarnessAdapter[]>('/harness/adapters'),

  dispatchHarness: (task: string, adapter: string | null): Promise<HarnessResult> =>
    http<HarnessResult>('/harness/dispatch', {
      method: 'POST',
      body: JSON.stringify({ task, adapter }),
    }),

  /* ---------- Online research (联网) ---------- */
  research: (query: string): Promise<ResearchResult> =>
    http<ResearchResult>('/research', { method: 'POST', body: JSON.stringify({ query }) }),

  /* ---------- Service introspection (overview dashboard) ----------
     NOTE: /harness/capabilities and the health endpoint live OUTSIDE the
     /api prefix on the backend, so they are addressed relative to API_BASE's
     parent directory. */
  getHealth: (): Promise<HealthStatus> => http<HealthStatus>('../health'),

  getCapabilities: (): Promise<CapabilitiesInfo> => http<CapabilitiesInfo>('../harness/capabilities'),

  /* ---------- Agent loop / TTC / LLMLab (UPGRADE_PLAN_V2 pillars) ---------- */
  runAgent: (task: string, maxRounds = 8, maxRepairs = 3): Promise<AgentRunResult> =>
    http<AgentRunResult>('/agent/run', {
      method: 'POST',
      body: JSON.stringify({ task, max_rounds: maxRounds, max_repairs: maxRepairs }),
    }),

  getAgentTools: (): Promise<{ tools: AgentToolInfo[] }> => http('/agent/tools'),

  recentEvents: (n = 20): Promise<{ events: Array<{ type: string; ts: number; payload: Record<string, unknown> }> }> =>
    http(`/events/recent${buildQuery({ n })}`),

  ttcRun: (strategy: string, prompt: string, n = 3): Promise<TtcResult> =>
    http<TtcResult>('/ttc/run', {
      method: 'POST',
      body: JSON.stringify({ strategy, prompt, n }),
    }),

  llmlabTokenize: (text: string, vocabSize = 276): Promise<TokenizeResult> =>
    http<TokenizeResult>('/llmlab/tokenize', {
      method: 'POST',
      body: JSON.stringify({ text, vocab_size: vocabSize }),
    }),

  llmlabForward: (text: string, opts?: { dim?: number; heads?: number; layers?: number }): Promise<ForwardResultDTO> =>
    http<ForwardResultDTO>('/llmlab/forward', {
      method: 'POST',
      body: JSON.stringify({ text, dim: opts?.dim ?? 32, n_heads: opts?.heads ?? 4, n_layers: opts?.layers ?? 2 }),
    }),
};
