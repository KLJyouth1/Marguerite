import type { WorkbenchItem } from './types'

/**
 * Builds the single workbench tab that surfaces the current task's steps and
 * deliverables. Mirrors `createGoalWorkbenchItem`: the tab keeps a stable id so
 * reopening reuses the same panel instead of spawning duplicates, and the scope
 * follows the active session when one is known.
 */
export function createTaskWorkbenchItem(options: { scopeId?: string } = {}): WorkbenchItem {
  return {
    id: 'task:current',
    kind: 'task',
    title: 'Task progress',
    scope: options.scopeId ? { type: 'session', id: options.scopeId } : { type: 'app' },
    hostKind: 'dom',
    retention: 'keep-alive',
    payload: {},
  }
}

/**
 * Production open path for the task workbench panel.
 *
 * Gap4: `createTaskWorkbenchItem` had no production caller — the panel was dead
 * code. `AppWorkbench` calls this (via the {@link TASK_WORKBENCH_OPEN_EVENT}
 * window event) so the delivery-pipeline panel opens from real UI instead of
 * being constructed only in tests. Returns `openItem`'s result so the caller can
 * surface the item-limit toast.
 */
export function openTaskWorkbenchItem(
  store: { openItem: (item: WorkbenchItem) => boolean },
  scopeId?: string,
): boolean {
  return store.openItem(createTaskWorkbenchItem({ scopeId }))
}

/** Window event used to request the task workbench panel from anywhere. */
export const TASK_WORKBENCH_OPEN_EVENT = 'marguerite:open-task'

export interface TaskWorkbenchOpenEventDetail {
  scopeId?: string
}

/** Dispatch the open request; returns false outside a DOM host (SSR/tests). */
export function requestTaskWorkbenchOpen(scopeId?: string): boolean {
  if (typeof window === 'undefined') return false
  window.dispatchEvent(new CustomEvent<TaskWorkbenchOpenEventDetail>(
    TASK_WORKBENCH_OPEN_EVENT,
    { detail: { scopeId } },
  ))
  return true
}
