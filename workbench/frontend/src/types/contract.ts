/**
 * Typed mirror of the Marguerite File Intelligence Workbench API contract.
 * Source of truth: api-contract-marguerite-file-intel-2026-08-14.md
 * Every shape below is consumed by the typed API client (src/api/client.ts).
 */

export type SectionKind = 'heading' | 'function' | 'class' | 'block' | 'raw';

export interface FileSection {
  id: string;
  title: string | null;
  start_line: number;
  end_line: number;
  kind: SectionKind;
  collapsed: boolean;
  /** First ~12 lines, shown while the section is folded. */
  preview: string;
  /** Full body; null when the section is collapsed. */
  full: string | null;
}

export interface FileCard {
  file_id: string;
  path: string;
  language: string | null;
  size_bytes: number;
  sha256: string;
  sections: FileSection[];
  meta: Record<string, unknown>;
}

export interface FileDisplay {
  /** Pre-rendered, syntax-highlighted HTML from the backend. */
  html: string;
  language: string;
}

export interface ModifyResult {
  new_content: string;
  diff: string;
  backup_ref: string;
  applied: boolean;
}

export interface Citation {
  text?: string;
  source?: string;
  line?: number;
  [key: string]: string | number | undefined;
}

export interface AskResult {
  answer: string;
  citations: Citation[];
}

export interface InteractResult {
  reply: string;
  session_id: string;
}

export interface MemoryLayerInfo {
  layer: string;
  name: string;
  count: number;
  capacity: number;
}

export interface MemoryItem {
  id: string;
  layer: string;
  kind: string;
  content: string;
  embedding: number[] | null;
  created_at: string;
  refs: string[];
}

export interface MemoryItemInput {
  layer: string;
  kind: string;
  content: string;
  embedding?: number[] | null;
  refs?: string[];
}

export interface ExpertSkill {
  name: string;
  description: string;
  procedure: string[];
}

export interface ExpertRole {
  role: string;
  title: string;
  team?: string | null;
  distilled_from: string[];
  problem_solving_template: string;
  skills: ExpertSkill[];
  experience_notes: string;
}

/** One of the 9 canonical expert modes: 3 intensities × 3 working modes. */
export interface ExpertMode {
  id: string;
  intensity: 'minimal' | 'standard' | 'extreme';
  working: 'ask' | 'plan' | 'doit';
  label_zh: string;
  label_en: string;
  description: string;
}

/** Grouped expert team for the selector page. */
export interface ExpertTeamMember {
  role: string;
  title: string;
  kind: string; // lead | member | core
}

export interface ExpertTeam {
  team: string;
  name: string;
  category: string;
  lead_role: string | null;
  members: ExpertTeamMember[];
}

/** A plan step produced by the Plan working mode. */
export interface ExpertPlanStep {
  step: number;
  action: string;
  milestone: string | null;
}

/** The (side-effect-free) execution intent produced by the DoIt working mode. */
export interface ExpertExecution {
  role: string;
  intent: Array<{ action: string }>;
  dispatched: boolean;
  note: string;
}

export type Verdict = 'pass' | 'conditional' | 'fail';

export interface ExpertAnalysis {
  role: string;
  verdict: Verdict;
  findings: string[];
  recommendations: string[];
  confidence: number;
  mode?: string;
  intensity?: 'minimal' | 'standard' | 'extreme';
  working?: 'ask' | 'plan' | 'doit';
  notes?: string[];
  plan?: ExpertPlanStep[];
  execution?: ExpertExecution;
}

/** Result of the chat endpoint: resolves @mentions and analyzes each under a mode. */
export interface ExpertChatResult {
  mentioned: string[];
  mode: string;
  analyses: ExpertAnalysis[];
  note?: string;
}

export interface HarnessAdapter {
  id: string;
  name: string;
  available: boolean;
  command: string | null;
  best_practices: string[];
}

export interface HarnessResult {
  adapter: string;
  task: string;
  steps: string[];
  diffs: string[];
  status: string;
  duration_ms: number;
}

export interface KBChunk {
  text: string;
  score: number;
  source: string;
}

export interface ResearchEntry {
  title: string;
  url: string;
  snippet: string;
}

export interface ResearchResult {
  query: string;
  results: ResearchEntry[];
  synthesized: string;
}

export interface KBIngestInput {
  text: string | null;
  path: string | null;
  doc_id: string;
}

export interface KBIngestResult {
  indexed_chunks: number;
}

/* ---------- Overview dashboard (real backend introspection) ---------- */

export interface HealthStatus {
  status: string;
  service: string;
}

export interface MemoryBackendInfo {
  active: 'core' | 'local';
  is_core_store: boolean;
  fallback: boolean;
  db_path: string | null;
  backend: string;
}

export interface NativeCapabilities {
  memory_7layer: boolean;
  file_intel: boolean;
  knowledge_base: boolean;
  distilled_experts: boolean;
  research: boolean;
  expert_count: number;
  bridge_version: string;
}

export interface CapabilitiesInfo {
  service: string;
  memory_backend: MemoryBackendInfo;
  layers: Record<string, { name: string; capacity: number }>;
  native_capabilities: NativeCapabilities;
}

/* ---------- Agent loop (支柱C) / TTC (支柱B) / LLMLab (支柱A) ---------- */

export interface StepRecord {
  index: number;
  state: string;
  tool: string | null;
  args: Record<string, unknown>;
  ok: boolean;
  detail: Record<string, unknown>;
  error: string | null;
  duration_ms: number;
  is_repair: boolean;
  repaired_index: number | null;
}

export interface AgentRunResult {
  task: string;
  status: 'completed' | 'failed';
  answer: string;
  steps: StepRecord[];
  session_id: string;
  events_file: string;
  rounds_used: number;
}

export interface AgentToolInfo {
  name: string;
  required_args: string[];
  safe_resolve_guarded: boolean;
}

export interface TtcResult {
  strategy: string;
  engine: string;
  answer: string;
  candidates: string[];
  votes: Record<string, number>;
  scores: number[];
  agreement: number;
  meta: Record<string, unknown>;
}

export interface BudgetPlan extends Record<string, unknown> {
  difficulty: string;
  n_candidates: number;
  max_rounds: number;
  effort: string;
}

export interface TokenizeResult {
  ids: number[];
  n_tokens: number;
  trace: Array<{ pair: [number, number]; new_id: number; tokens_before: number; tokens_after: number }>;
  compression: number;
  vocab_size: number;
  decoded_roundtrip_ok: boolean;
}

export interface ForwardResultDTO {
  config: { vocab_size: number; dim: number; n_heads: number; n_layers: number; block_size: number };
  tokens: number[];
  loss: number | null;
  logits_head: number[];
  attention: number[][][][]; // [layer][head][query][key]
}
