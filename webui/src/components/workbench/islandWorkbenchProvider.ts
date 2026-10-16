import type { WorkbenchItem, WorkbenchPanelDefinition } from '@/workbench/types'
import IslandPanel from './IslandPanel.vue'

/**
 * Registers the `island` workbench panel — the 3D run visualization embedded
 * in the workbench as a session-scoped, keep-alive tab. The heavy lifting
 * (runtime reduction → world → Island3D) lives in `@/components/island/*`;
 * this panel only wires props and lifecycle.
 */
export function createIslandWorkbenchDefinition(): WorkbenchPanelDefinition {
  return {
    kind: 'island',
    component: IslandPanel,
    supports: item => item.kind === 'island',
    getHeader: item => ({ title: item.title, icon: 'blocks' }),
    getProps: (item: WorkbenchItem) => ({ item }),
  }
}