import type { WorkbenchItem } from './types'

export function createGoalWorkbenchItem(options: {
  goalId?: string
  scopeId?: string
} = {}): WorkbenchItem {
  const goalId = options.goalId || 'current'
  return {
    id: `goal:${goalId}`,
    kind: 'goal',
    title: 'Goal progress',
    scope: options.scopeId ? { type: 'session', id: options.scopeId } : { type: 'app' },
    hostKind: 'dom',
    retention: 'keep-alive',
    payload: { goalId },
  }
}

export function goalIdFromWorkbenchItem(item: WorkbenchItem): string {
  const value = item.payload.goalId
  return typeof value === 'string' && value.trim() ? value : 'current'
}
