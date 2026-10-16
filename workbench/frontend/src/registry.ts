import type { ComponentType } from 'react';
import type { IconName } from './components/ui/icons';
import { OverviewPanel } from './features/OverviewPanel';
import { FilesPanel } from './features/FilesPanel';
import { MemoryPanel } from './features/MemoryPanel';
import { ExpertPanel } from './features/ExpertPanel';
import { KnowledgePanel } from './features/KnowledgePanel';
import { HarnessPanel } from './features/HarnessPanel';
import { ResearchPanel } from './features/ResearchPanel';
import { OrchestratePanel } from './features/OrchestratePanel';
import { ModelLabPanel } from './features/ModelLabPanel';

export type PanelId =
  | 'overview'
  | 'files'
  | 'memory'
  | 'experts'
  | 'knowledge'
  | 'harness'
  | 'research'
  | 'orchestrate'
  | 'modellab';

export interface PanelMeta {
  id: PanelId;
  label: string;
  description: string;
  icon: IconName;
}

/** Ordered navigation metadata — drives the sidebar without hardcoding branches. */
export const PANELS: PanelMeta[] = [
  { id: 'overview', label: 'Overview', description: 'Live workbench telemetry', icon: 'pulse' },
  { id: 'orchestrate', label: 'Orchestrate', description: 'Agent loop runner', icon: 'route' },
  { id: 'modellab', label: 'Model Lab', description: 'BPE · attention math', icon: 'atom' },
  { id: 'files', label: 'Files', description: 'Load & act on files', icon: 'file' },
  { id: 'memory', label: 'Memory', description: '7-layer explorer', icon: 'layers' },
  { id: 'experts', label: 'Experts', description: 'Distilled analysis', icon: 'brain' },
  { id: 'knowledge', label: 'Knowledge', description: 'KB search & ingest', icon: 'search' },
  { id: 'harness', label: 'Harness', description: 'Codex adapters', icon: 'cpu' },
  { id: 'research', label: 'Research', description: 'Online (联网)', icon: 'globe' },
];

/**
 * Extensibility backbone: the active view is resolved from this registry.
 * Adding a panel = append a PANELS entry + a registry entry; the shell and
 * sidebar pick it up with no branching changes.
 */
export const panelRegistry: Record<PanelId, ComponentType> = {
  overview: OverviewPanel,
  orchestrate: OrchestratePanel,
  modellab: ModelLabPanel,
  files: FilesPanel,
  memory: MemoryPanel,
  experts: ExpertPanel,
  knowledge: KnowledgePanel,
  harness: HarnessPanel,
  research: ResearchPanel,
};
