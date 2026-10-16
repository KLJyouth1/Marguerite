import type { WorkbenchItem, WorkbenchPanelDefinition } from '@/workbench/types'
import TaskPanel from './TaskPanel.vue'

/**
 * Registers the `task` workbench panel. The panel body is an empty shell in
 * this change set; T05 wires it to the live task contract
 * (`decision.subtasks` + `step_start`/`step_finish` + `artifact.updated`).
 */
export function createTaskWorkbenchDefinition(): WorkbenchPanelDefinition {
  return {
    kind: 'task',
    component: TaskPanel,
    supports: item => item.kind === 'task',
    getHeader: item => ({ title: item.title, icon: 'listChecks' }),
    getProps: (item: WorkbenchItem) => ({ item }),
  }
}
