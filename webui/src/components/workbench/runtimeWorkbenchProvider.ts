import type { WorkbenchItem, WorkbenchPanelDefinition } from '@/workbench/types'
import { runtimeScopeFromWorkbenchItem } from '@/workbench/runtimeItems'
import RuntimePanel from './RuntimePanel.vue'

export interface RuntimeWorkbenchProviderOptions {
  /** 已本地化的头部副标题（缺省留空，不显示未翻译文案） */
  subtitle?: string
}

/**
 * 「运行态」面板：只读聚合 Harness 侧的运行信息，不发执行。
 *
 * 与其它 provider 的差异：没有 runtime class —— 面板自己是只读观测面，
 * 数据由组件按需拉取（`RuntimePanel` 内部负责刷新与降级）。
 */
export function createRuntimeWorkbenchDefinition(
  options: RuntimeWorkbenchProviderOptions = {},
): WorkbenchPanelDefinition {
  return {
    kind: 'runtime',
    component: RuntimePanel,
    supports: (item: WorkbenchItem) =>
      item.kind === 'runtime' && Boolean(runtimeScopeFromWorkbenchItem(item)),
    getHeader: item => ({
      icon: 'gauge',
      title: item.title,
      ...(options.subtitle ? { subtitle: options.subtitle } : {}),
    }),
    getProps: (item, state) => ({
      scopeId: runtimeScopeFromWorkbenchItem(item),
      active: state.active,
    }),
  }
}
