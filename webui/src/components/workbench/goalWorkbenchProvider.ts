import type { WorkbenchItem, WorkbenchPanelDefinition } from '@/workbench/types'
import GoalPanel from './GoalPanel.vue'
import { goalIdFromWorkbenchItem } from '@/workbench/goalItems'

export function createGoalWorkbenchDefinition(): WorkbenchPanelDefinition {
  return {
    kind: 'goal',
    component: GoalPanel,
    supports: item => item.kind === 'goal',
    getHeader: item => ({ title: item.title, subtitle: goalIdFromWorkbenchItem(item), icon: 'gauge' }),
    getProps: (item: WorkbenchItem) => ({ item }),
  }
}
