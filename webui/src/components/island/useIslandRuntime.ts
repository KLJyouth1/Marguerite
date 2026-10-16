/**
 * 岛屿运行时接入（Vue 薄封装）—— 只做三件事：订阅、过滤、暴露响应式快照。
 *
 * 全部归约逻辑在纯函数岛 islandRuntime.ts（可单测）；这里保持零业务判断，
 * 避免「事件语义」散落在组件里。
 */

import { computed, onScopeDispose, shallowRef, watch, type ComputedRef, type Ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'
import {
  applyIslandEvent,
  createIslandRuntimeState,
  ISLAND_TRACKED_EVENTS,
  type IslandRuntimeState,
} from './islandRuntime'

export interface UseIslandRuntimeOptions {
  /** 目标会话 key 的 getter（空字符串 = 浏览态，不过滤） */
  sessionKey?: () => string
}

export interface UseIslandRuntime {
  state: Ref<IslandRuntimeState>
  /** 网关连接状态（未连接时岛屿保持静止，UI 诚实提示） */
  connected: ComputedRef<boolean>
  reset: () => void
}

export function useIslandRuntime(options: UseIslandRuntimeOptions = {}): UseIslandRuntime {
  const rpc = useRpcStore()
  const state = shallowRef<IslandRuntimeState>(createIslandRuntimeState(options.sessionKey?.() ?? ''))

  const disposers = ISLAND_TRACKED_EVENTS.map((event) =>
    rpc.on(event, (payload: unknown) => {
      const next = applyIslandEvent(state.value, event, payload, {
        sessionKey: options.sessionKey?.() ?? state.value.sessionKey,
        now: Date.now(),
      })
      // 引用不变 = 事件与本会话无关，跳过赋值（省掉整棵场景的重算）
      if (next !== state.value) state.value = next
    }),
  )

  if (options.sessionKey) {
    // 会话切换必须重置：岛屿绝不能残留上一个会话的建筑（诚实可视化）
    watch(options.sessionKey, (key) => {
      state.value = createIslandRuntimeState(key || '')
    })
  }

  onScopeDispose(() => {
    for (const off of disposers) off()
  })

  return {
    state,
    connected: computed(() => rpc.isConnected),
    reset: () => {
      state.value = createIslandRuntimeState(options.sessionKey?.() ?? '')
    },
  }
}