// PSI 意识引擎（aris_brain `brain.status` RPC）响应解包——纯函数，便于单测守护。
//
// 历史 bug：DesktopPet 曾把 `psi.value = res`（整包 `{ ok, brain }`）后去读顶层
// `psi.phi` / `psi.emotions`，而真实字段全在 `res.brain` 下（且 `phi_existence` 是
// 格式化字符串，不是数字）。这导致八维情感恒回落 0.5、Φ 显示 0.00，且因无测试守护
// 长期潜伏。此处的 `unpackPsiBrain` 是**唯一正确**的解包入口，由单测锁定字段位置。

export interface PsiSnapshot {
  emotion?: Record<string, number>
  dominant_mood?: string
  circadian?: string
  /** 注意：aras_brain 返回的是格式化字符串（"0.123"），不是 number */
  phi_existence?: string | number
  [k: string]: unknown
}

const SNAPSHOT_KEYS = ['emotion', 'circadian', 'dominant_mood', 'phi_existence'] as const

/**
 * 从 `brain.status` 的 `{ ok, brain }` 包裹（或 `psi_broadcast` 直接投递的快照）解包出 PSI 快照。
 *
 * - `brain.status` RPC 返回 `{ ok, brain }` → 取 `res.brain`。
 * - `psi_broadcast` 可能直接投递快照（无 `brain` 包装）→ 原样返回。
 * - 载荷非法 / 引擎未就绪（null、无 brain、无任一已知字段）→ 返回 `null`，调用方应静默跳过。
 *
 * 返回类型始终为 `PsiSnapshot | null`，调用方据此判断是否有真实八维情感可驱动桌宠外观。
 */
export function unpackPsiBrain(res: unknown): PsiSnapshot | null {
  if (!res || typeof res !== 'object') return null
  const r = res as Partial<{ brain?: PsiSnapshot }> & Record<string, unknown>
  // brain.status RPC 包裹：{ ok, brain }
  if (r.brain && typeof r.brain === 'object') return r.brain as PsiSnapshot
  // psi_broadcast 可能直接投递 snapshot（无 brain 包装）
  const looksLikeSnapshot = SNAPSHOT_KEYS.some((k) => k in r)
  return looksLikeSnapshot ? (res as PsiSnapshot) : null
}
