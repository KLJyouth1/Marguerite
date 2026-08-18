export const MODEL_ROUTING_MODES = ['off', 'squilla_router', 'llm_ensemble', 'bloom_ensemble'] as const

export type ModelRoutingMode = (typeof MODEL_ROUTING_MODES)[number]

export function isModelRoutingMode(value: unknown): value is ModelRoutingMode {
  return typeof value === 'string' && MODEL_ROUTING_MODES.includes(value as ModelRoutingMode)
}

export function normalizeModelRoutingMode(value: unknown): ModelRoutingMode {
  return isModelRoutingMode(value) ? value : 'off'
}

export const BLOOM_SUBMODES = ['deep', 'auto', 'swarm'] as const

export type BloomSubmode = (typeof BLOOM_SUBMODES)[number]

export function isBloomSubmode(value: unknown): value is BloomSubmode {
  return typeof value === 'string' && BLOOM_SUBMODES.includes(value as BloomSubmode)
}

export function normalizeBloomSubmode(value: unknown): BloomSubmode {
  return isBloomSubmode(value) ? value : 'deep'
}
