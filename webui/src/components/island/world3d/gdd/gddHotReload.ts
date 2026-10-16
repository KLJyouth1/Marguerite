/**
 * GDD 热加载管理器（契约层 + 订阅 API；P1 不改 scene 渲染）。
 *
 * ── 为什么「热加载」必须是原子替换 + 保留旧配置 ──────────────────────────
 * 设计依据 `_island_3d_game_design.md` §7.1 第 4 阶段「运行时装配：热加载配置
 * （版本化 manifest）」。AI 自改进闭环（§7.2）里，GDD patch 会反复到达运行时：
 * - **绝不半更新**：loadFromJson 先整份 validateGdd，全部通过才原子替换当前
 *   配置；任一 error → 旧配置原样继续跑（世界不能塌一半）；
 * - **revision 递增**：每次成功替换 revision+1，订阅方可按版本增量消费
 *   （scene.ts/IslandWorld3D.vue 用它判断是否需要重建场景树）；
 * - **错误透传**：失败结果把 errors 原样交给 UI（islandGame.gdd.* 词条渲染），
 *   「校验失败」是可见的产品状态而非控制台噪声；
 * - **exportCurrent 回灌**：导出当前生效 GDD 作为 AI 下一轮改进的输入——
 *   输入输出同型，闭环可迭代（自由文本做不到无损往返）。
 *
 * 订阅是普通 Set 回调（不绑 vue 响应式）：契约层保持纯 TS，P1 的消费方
 * （scene.ts / IslandWorld3D.vue）自行桥接 shallowRef。
 */

import type { Gdd, GddIssue } from './gddSchema'
import { validateGdd } from './gddSchema'

/** 一次热加载的结果（成功/失败都带完整上下文，供 UI 与测试断言） */
export interface GddLoadResult {
  ok: boolean
  /** 成功后的生效 revision（失败为 null） */
  revision: number | null
  /** 阻断性错误（失败时非空） */
  errors: GddIssue[]
  /** 非阻断提示（未知字段等，成功/失败都可能有） */
  warnings: GddIssue[]
}

/** 变更通知：只在成功替换后触发（失败保持旧配置，不打扰订阅方） */
export interface GddChangeEvent {
  gdd: Gdd
  revision: number
  /** 本次加载的非阻断提示（新旧字段差异对 UI 有意义） */
  warnings: GddIssue[]
}

export type GddListener = (event: GddChangeEvent) => void

/**
 * 热加载管理器。可 new 多个实例（测试/多世界），也可用模块级
 * gddHotReload 单例接 scene。
 */
export class GddHotReloadManager {
  private current: Gdd | null = null
  private revision = 0
  private listeners = new Set<GddListener>()
  /** 最近一次失败的错误明细（UI 常驻显示「校验失败」直到下次成功） */
  private lastErrors: GddIssue[] = []

  /**
   * 从 JSON 字符串或已解析对象热加载 GDD。
   *
   * 原子性保证：只有 validateGdd 全部通过才替换 current/revision；
   * 任何失败（JSON 解析失败、schema 违例）→ 旧配置继续跑，绝不半更新。
   */
  loadFromJson(raw: string | object): GddLoadResult {
    let parsed: unknown = raw
    if (typeof raw === 'string') {
      try {
        parsed = JSON.parse(raw)
      } catch (err) {
        const issue: GddIssue = {
          level: 'error',
          code: 'invalid_json',
          path: '',
          message: `JSON 解析失败：${err instanceof Error ? err.message : String(err)}`,
        }
        this.lastErrors = [issue]
        return { ok: false, revision: null, errors: [issue], warnings: [] }
      }
    }

    const validation = validateGdd(parsed)
    if (!validation.ok || !validation.value) {
      // 失败保持旧配置继续跑：current/revision 一个字节都不动
      this.lastErrors = validation.errors
      return { ok: false, revision: null, errors: validation.errors, warnings: validation.warnings }
    }

    // 原子替换：一次赋值完成世界配置切换，revision 递增作为版本锚点
    this.current = validation.value
    this.revision += 1
    this.lastErrors = []
    const event: GddChangeEvent = { gdd: validation.value, revision: this.revision, warnings: validation.warnings }
    for (const fn of this.listeners) fn(event)
    return { ok: true, revision: this.revision, errors: [], warnings: validation.warnings }
  }

  /** 当前生效配置（未加载任何 GDD 时为 null） */
  getCurrent(): Gdd | null {
    return this.current
  }

  /** 当前生效版本号（0 = 尚未成功加载） */
  getRevision(): number {
    return this.revision
  }

  /** 最近一次校验失败的错误明细（成功加载后清空） */
  getLastErrors(): GddIssue[] {
    return this.lastErrors
  }

  /**
   * 订阅变更（成功替换时通知）。返回取消订阅函数。
   * 只通知成功的替换——失败时世界没变，订阅方无需重绘。
   */
  subscribe(fn: GddListener): () => void {
    this.listeners.add(fn)
    return () => {
      this.listeners.delete(fn)
    }
  }

  /**
   * 导出当前生效配置（JSON 文本）——AI 下一轮自改进的输入。
   * 与 loadFromJson 构成无损往返（export → load 得到同一份 GDD）。
   * 未加载时导出 null（调用方决定降级，不伪造默认 GDD）。
   */
  exportCurrent(): string | null {
    if (!this.current) return null
    return JSON.stringify(this.current)
  }
}

/** 模块级单例（scene/IslandWorld3D 消费方共用一份世界配置） */
export const gddHotReload = new GddHotReloadManager()
