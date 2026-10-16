/**
 * 桌宠契约层（v3）— 枚举单一事实源 + 脑→渲染映射 + 能力边界元数据。
 *
 * 设计动机（engineering-core 铁律 R5 契约三同步）：
 * - 旧版 `PetMood`（petBrain，9 种）与渲染层 `PetMood3D`（6 种）两处手写映射，
 *   新增心情必漏同步；本模块把「单一来源」收口于此，任何心情都必须给出表情表达。
 * - 能力元数据（PET_CAPABILITIES）诚实声明桌宠真实能力边界：当前渲染层是
 *   **2D SVG 矢量 + 伪 3D 光照**（`petSvg.ts`），brain 的 9 种情绪经映射收敛表达。
 */

import type { PetMood } from './petBrain'

/** 渲染层可表达的心情（真实表情骨骼表现力上限，诚实地少于 brain 的 9 种） */
export const PET_MOOD_3D = ['normal', 'happy', 'sleepy', 'working', 'excited', 'sad'] as const
export type PetMood3D = (typeof PET_MOOD_3D)[number]

/**
 * 后端 `pet.status`/`pet.triggerAction` 返回的 mood 字符串 → 渲染层表情枚举。
 * 契约单一来源：仅接受 PET_MOOD_3D 内的值，未知/非法值收敛为 'normal'
 * （绝不虚构表现力；调用方据此驱动渲染层，绕过本函数即视为契约破坏）。
 */
export function toPetMood3D(raw: unknown): PetMood3D {
  return typeof raw === 'string' && (PET_MOOD_3D as readonly string[]).includes(raw)
    ? (raw as PetMood3D)
    : 'normal'
}

/** brain 心情（9 种）→ 3D 心情（6 种）唯一映射。新增 PetMood 必须在此补一行。 */
export const moodMap: Record<PetMood, PetMood3D> = {
  idle: 'normal',
  curious: 'normal',
  attentive: 'normal',
  working: 'working',
  celebrate: 'excited',
  alert: 'normal',
  sleepy: 'sleepy',
  thinking: 'normal',
  laughing: 'happy',
}

/**
 * 桌面宠能力边界（诚实声明，与 `petSvg.ts` 的真实实现**一一对应，不多不少**）。
 * 旧值 `'3d'`/`toon` 曾是 WebGL + 卡通着色，随 WebGL 路径被 SVG 取代已不成立。
 */
export const PET_CAPABILITIES = {
  /** 2D SVG 矢量渲染（`PetSvg` 以 `createElementNS` 手绘，零 WebGL 依赖） */
  svg: true,
  /** 伪 3D 光照（统一光照约定：径向渐变体积 + 轮廓光 + 环境遮蔽 + 接触投影） */
  pseudo3d: true,
  /** 表情骨骼（眉/嘴/耳/瞳/眼睑，逐帧由 `computePose` 驱动） */
  emotionBones: true,
  /** 粒子特效（`burst()` 用 Web Animations API，播完即移除，不常驻 DOM） */
  particles: true,
} as const

export type PetCapabilityKey = keyof typeof PET_CAPABILITIES

/**
 * 渲染档位（三档质量）。档位**只调特效预算**（柔光模糊半径 / 粒子数），
 * 不改变形象完整度：SVG 渲染器在任何档位、任何（含无 WebGL）环境下都完整成形。
 */
export type PetRenderTier = 'high' | 'medium' | 'low'

/**
 * 当前渲染档位（由 detectPetRuntime 判定）。`fallback` 仅表示 WebGL/硬件信号弱，
 * 并非能力缺失——SVG 渲染器无 WebGL 也能完整渲染，故调用方据此降的只是特效开销。
 */
export type PetQuality = PetRenderTier | 'fallback'

export interface PetRuntimeInfo {
  quality: PetQuality
  /** 判定依据摘要：webgl2 / webgl / memory / cores / none（仅硬件信号，非能力） */
  reason: string
  /** 人类可读的档位说明（如「完整渲染」「轻量模式」）；仅作展示，不代表形象缺失 */
  label: string
}