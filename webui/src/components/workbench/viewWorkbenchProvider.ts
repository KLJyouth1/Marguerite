import type { WorkbenchItem, WorkbenchPanelDefinition } from '@/workbench/types'
import ViewPanel from './ViewPanel.vue'

/**
 * The generic dock panel (`view`).
 *
 * Any registered view can be mounted as a keep-alive workbench tab, which is how
 * a route-owned view stays inspectable while the main area navigates elsewhere.
 * Docking opens an additional instance rather than stealing the route slot, so
 * the route-exclusion model is untouched.
 *
 * There is deliberately no `createRuntime`: a view panel owns no non-serializable
 * resource, so the default DOM lifecycle (mount / unmount with the panel layer) is
 * exactly right.
 */
export function createViewWorkbenchDefinition(): WorkbenchPanelDefinition {
  return {
    kind: 'view',
    component: ViewPanel,
    supports: item => item.kind === 'view',
    getHeader: item => ({ title: viewTitle(item) }),
    getProps: (item: WorkbenchItem) => ({
      view: String(item.payload.view ?? ''),
      title: viewTitle(item),
    }),
  }
}

/** Payload title wins; the item title is the dock tab label. */
function viewTitle(item: WorkbenchItem): string {
  const title = item.payload.title
  return typeof title === 'string' && title ? title : item.title
}