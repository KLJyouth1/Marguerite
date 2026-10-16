// Pet RPC 前端契约层（V2.1-P1c · R5 契约三同步守卫）
// 字段与枚举与 Rust core/src/gateway/pet.rs 一一对应。
// 任何后端字段/枚举变更都必须在此同步，并跑 Playwright 握手断言「字段集合对称」。

import { useRpcStore } from '@/stores/rpc'

/** ───────────────────────────────────────────────────────────────
 *  枚举（与 Rust `PetSkinId` / `PetMood` / `PetKind` / `PetAction` 对齐）
 *  ─────────────────────────────────────────────────────────────── */

export const PET_SKIN_IDS = ['cat', 'blue', 'pink', 'purple', 'green'] as const
export type PetSkinId = (typeof PET_SKIN_IDS)[number]

export const PET_MOODS = ['normal', 'happy', 'sleepy', 'working', 'excited', 'sad'] as const
export type PetMood = (typeof PET_MOODS)[number]

export const PET_KINDS = ['beast', 'humanoid'] as const
export type PetKind = (typeof PET_KINDS)[number]

export const PET_ACTIONS = ['pet', 'feed', 'celebrate', 'wake', 'doze'] as const
export type PetAction = (typeof PET_ACTIONS)[number]

/** ───────────────────────────────────────────────────────────────
 *  DTO（与 Rust PetSkinEntry / PetStatus 结构对齐；字段名 1:1）
 *  ─────────────────────────────────────────────────────────────── */

export interface PetSkinEntry {
  id: PetSkinId | string
  label: string
  kind: PetKind
  accent_hex: string
}

export interface PetStatus {
  skin: PetSkinId | string
  kind: PetKind
  mood: PetMood | string
  energy: number
  affection: number
  running: boolean
  accent_hex: string
}

export interface PetListResponse {
  ok: boolean
  skins: PetSkinEntry[]
}

export interface SetSkinResponse {
  ok: boolean
  skin: PetSkinId | string
  error?: string
}

export interface SetMoodResponse {
  ok: boolean
  mood: PetMood | string
  error?: string
}

export interface TriggerActionResponse {
  ok: boolean
  action?: PetAction | string
  mood?: PetMood | string
  energy?: number
  affection?: number
  error?: string
}

/** ───────────────────────────────────────────────────────────────
 *  契约守卫：运行时字段对称性断言（R5：字段漂移动态捕获）
 *  ─────────────────────────────────────────────────────────────── */

/** 皮肤目录应正好 5 款（猫 + 4 精灵） */
export const PET_EXPECTED_SKIN_COUNT = 5

/** 校验后端返回 `pet.status` 字段集合与 TS 契约对称；缺失则抛出，
 *  用于 R5 契约三同步门禁 & Playwright e2e 握手断言。 */
export function assertPetStatusSymmetry(payload: Record<string, unknown>): PetStatus {
  const required: (keyof PetStatus)[] = [
    'skin', 'kind', 'mood', 'energy', 'affection', 'running', 'accent_hex',
  ]
  const missing = required.filter((k) => !(k in payload))
  if (missing.length) {
    throw new Error(
      `[Pet R5 契约守卫] pet.status 缺少字段: ${missing.join(', ')}；` +
      '请同步 core/src/gateway/pet.rs 与 webui/src/rpc/petContract.ts。',
    )
  }
  return payload as unknown as PetStatus
}

export function assertPetListSymmetry(payload: Record<string, unknown>): PetListResponse {
  if (!payload.ok) throw new Error('[Pet R5 契约守卫] pet.list 返回 !ok')
  const u = payload as unknown as PetListResponse
  const skins = u.skins
  if (!Array.isArray(skins)) throw new Error('[Pet R5 契约守卫] pet.list.skins 不是数组')
  for (const s of skins) {
    const req: (keyof PetSkinEntry)[] = ['id', 'label', 'kind', 'accent_hex']
    const miss = req.filter((k) => !(k in s))
    if (miss.length) throw new Error(`[Pet R5 契约守卫] PetSkinEntry(${s.id}) 缺字段 ${miss.join(',')}`)
  }
  return u
}

/** ───────────────────────────────────────────────────────────────
 *  便捷 RPC 调用封装（单一事实源 · 与 stores/rpc 解耦）
 *  ─────────────────────────────────────────────────────────────── */

export function usePetRpc() {
  const rpc = useRpcStore()
  return {
    status: async () => {
      const raw = await rpc.call<Record<string, unknown>>('pet.status', {})
      return assertPetStatusSymmetry(raw)
    },
    list: async () => {
      const raw = await rpc.call<Record<string, unknown>>('pet.list', {})
      return assertPetListSymmetry(raw)
    },
    setSkin: async (skin: PetSkinId | string) =>
      rpc.call<SetSkinResponse>('pet.setSkin', { skin }),
    setMood: async (mood: PetMood | string) =>
      rpc.call<SetMoodResponse>('pet.setMood', { mood }),
    triggerAction: async (action: PetAction | string) =>
      rpc.call<TriggerActionResponse>('pet.triggerAction', { action }),
    /** 兼容入口（保留旧 PetView 调用语义，内部转发到 triggerAction） */
    actionCompat: async (action: 'pet' | 'feed') =>
      rpc.call<{ mood: string; energy: number; affection: number }>('pet.action', { action }),
  }
}
