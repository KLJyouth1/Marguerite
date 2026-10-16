export const MODEL_ROUTING_MODES = ['off', 'squilla_router', 'llm_ensemble', 'bloom_ensemble', 'omni_agent'] as const

export type ModelRoutingMode = (typeof MODEL_ROUTING_MODES)[number]

export function isModelRoutingMode(value: unknown): value is ModelRoutingMode {
  return typeof value === 'string' && MODEL_ROUTING_MODES.includes(value as ModelRoutingMode)
}

export function normalizeModelRoutingMode(value: unknown): ModelRoutingMode {
  // 默认对齐「默认 = omni」：未显式选择/非法值时回落到 omni_agent，
  // 而非旧的 off。注意产品实际生效值仍由 ChatView 的特征开关派生。
  return isModelRoutingMode(value) ? value : 'omni_agent'
}

/* ── 与网关的词表桥接（单一事实源） ───────────────────────────────────── */

/**
 * 后端 `models.routing.set` 接受的路由档位（Rust 侧白名单）。
 *
 * 单一事实源指向 `core/src/gateway/mode.rs::models_routing_set` 里的校验集合：
 * ```rust
 * if !matches!(mode.as_str(), "direct" | "router" | "ensemble" | "bloom" | "omni")
 *   → RpcError::new("INVALID_REQUEST", …)
 * ```
 * 前端任何档位都必须映射到这里的一个值，否则该 RPC 直接报错、UI 上表现为
 * 「点了没反应」。
 */
export const GATEWAY_ROUTING_MODES = ['direct', 'router', 'ensemble', 'bloom', 'omni'] as const

export type GatewayRoutingMode = (typeof GATEWAY_ROUTING_MODES)[number]

/**
 * `mode.switch { kind }` 接受的值（联动运行时编排形态时使用）。
 * `'agent'` = 回落到单智能体，与 `core/src/domain/mod.rs::AgentMode` 对齐。
 */
export type RuntimeModeKind = 'agent' | 'bloom' | 'omni-transcend'

/**
 * UI 档位 →（网关档位, 运行时模式 kind）。
 *
 * ⚠️ **为什么必须是一张表**：`useChatFeatureToggles.setModelRoutingMode` 曾把同一套
 * 三元链**手写三遍** —— RPC 负载、运行时 kind、生效回执各一份。三份拷贝一旦漂移，
 * 后果是「界面显示已切换、后端实际没变」的**静默失效**，而这正是本轮要消除的
 * 「模式与工具集成」核心缺陷。现在收敛为唯一事实源，改一处即三处生效。
 *
 * 另外原第三处（生效回执）其实是**恒等函数** —— 每个分支都映射到自身，还留了个死
 * 默认分支，看着像在做事。已直接删除。
 */
export const MODEL_ROUTING_BRIDGE: Record<
  ModelRoutingMode,
  { gateway: GatewayRoutingMode; runtimeKind: RuntimeModeKind }
> = {
  off: { gateway: 'direct', runtimeKind: 'agent' },
  squilla_router: { gateway: 'router', runtimeKind: 'agent' },
  llm_ensemble: { gateway: 'ensemble', runtimeKind: 'agent' },
  bloom_ensemble: { gateway: 'bloom', runtimeKind: 'bloom' },
  omni_agent: { gateway: 'omni', runtimeKind: 'omni-transcend' },
}

/** UI 档位 → 网关档位（`models.routing.set` 的 `mode` 载荷）。 */
export function toGatewayRoutingMode(mode: ModelRoutingMode): GatewayRoutingMode {
  return MODEL_ROUTING_BRIDGE[mode].gateway
}

/** UI 档位 → 运行时模式 kind（`mode.switch` 的 `kind` 载荷）。 */
export function toRuntimeModeKind(mode: ModelRoutingMode): RuntimeModeKind {
  return MODEL_ROUTING_BRIDGE[mode].runtimeKind
}

/**
 * 网关档位 → UI 档位（把服务端权威快照读回成 UI 语言时用）。
 *
 * 返回 `null` 表示该网关档位没有对应 UI 项（例如后端升级出了新档位而前端没跟上）
 * —— 调用方应据此**显式提示**，而不是静默归一化成某个默认项。
 */
export function fromGatewayRoutingMode(value: unknown): ModelRoutingMode | null {
  if (typeof value !== 'string') return null
  const hit = (Object.keys(MODEL_ROUTING_BRIDGE) as ModelRoutingMode[]).find(
    (key) => MODEL_ROUTING_BRIDGE[key].gateway === value
  )
  return hit ?? null
}

/** 运行时 kind → UI 档位（用于把 `mode.get` 的状态反推成 UI 语言）。 */
export function fromRuntimeModeKind(value: unknown): ModelRoutingMode | null {
  if (typeof value !== 'string') return null
  const hit = (Object.keys(MODEL_ROUTING_BRIDGE) as ModelRoutingMode[]).find(
    (key) => MODEL_ROUTING_BRIDGE[key].runtimeKind === value
  )
  return hit ?? null
}
