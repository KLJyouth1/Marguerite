import { defineAsyncComponent } from 'vue'
import type { WorkbenchPanelDefinition } from '@/workbench/types'
import { editorPathFromWorkbenchItem, editorScopeFromWorkbenchItem } from '@/workbench/editorItems'

/**
 * 内置编辑器面板（IDE 式单文件标签）。
 *
 * 面板异步加载：highlight.js 与 diff 计算只在真的打开文件时进包，
 * 不拖慢工作台首屏（主题包/首屏体积守卫都会因此受益）。
 */
const EditorPanel = defineAsyncComponent(() => import('./EditorPanel.vue'))

export function createEditorWorkbenchDefinition(): WorkbenchPanelDefinition {
  return {
    kind: 'file',
    component: EditorPanel,
    supports: item => item.kind === 'file' && Boolean(editorPathFromWorkbenchItem(item)),
    getHeader: (item) => {
      const path = editorPathFromWorkbenchItem(item)
      const projectPath = typeof item.payload.projectPath === 'string' ? item.payload.projectPath : ''
      return {
        icon: 'fileCode',
        title: item.title,
        subtitle: projectPath ? `${projectPath} / ${path}` : path,
      }
    },
    getProps: (item, state) => ({
      path: editorPathFromWorkbenchItem(item),
      scopeId: editorScopeFromWorkbenchItem(item),
      projectPath: typeof item.payload.projectPath === 'string' ? item.payload.projectPath : '',
      active: state.active,
    }),
  }
}
