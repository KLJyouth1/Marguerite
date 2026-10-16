/**
 * 状态卡文案模型 —— 把运行图相位翻译成「现在做什么 / 下一步做什么」的 i18n 键与色调。
 *
 * 这层是**纯函数**：输入列车当前/下一状态，输出应渲染的 i18n 键位。
 * EntityStateCard.vue 只负责把键位解析成文案并排版，不做业务判断 ——
 * 这样同一套卡片能复用到任务书 A 的设备节点与岛屿 agent 节点。
 *
 * 不依赖 vue / DOM，可在 node 单测中跑。
 */
import type { TrainPhase, TrainState } from './schedule'

/** 状态卡语气（与全台 accent/ok/warn/info 语义色同一口径）。 */
export type CardTone = 'accent' | 'ok' | 'warn' | 'info' | 'muted'

/** 状态卡需要的全部文案键（视图用 t() 解析后传给 EntityStateCard）。 */
export interface EntityCardCopy {
  /** 实体种类标签的 i18n 键（如“列车”） */
  kindLabelKey: string
  /** “现在做什么”短语的 i18n 键 */
  nowKey: string
  /** “现在”补充细节（车站名）的 i18n 键；无则 undefined */
  nowDetailKey?: string
  /** “下一步做什么”短语的 i18n 键 */
  nextKey: string
  /** “下一步”补充细节（车站名）的 i18n 键；无则 undefined */
  nextDetailKey?: string
  /** 当前状态色调 */
  tone: CardTone
}

const PHASE_KEY: Record<TrainPhase, string> = {
  dwell: 'sandbox.phase.dwell',
  departing: 'sandbox.phase.departing',
  cruise: 'sandbox.phase.cruise',
  arriving: 'sandbox.phase.arriving',
}

const PHASE_TONE: Record<TrainPhase, CardTone> = {
  dwell: 'ok',
  departing: 'accent',
  cruise: 'info',
  arriving: 'warn',
}

function stationKey(station: TrainState['station']): string | undefined {
  return station ? `sandbox.station.${station}` : undefined
}

/**
 * 由列车当前/下一状态生成状态卡文案。
 * @param now  trainStateAt(...)
 * @param next nextTrainState(...)
 */
export function describeTrainCard(now: TrainState, next: TrainState): EntityCardCopy {
  return {
    kindLabelKey: 'sandbox.kind.train',
    nowKey: PHASE_KEY[now.phase],
    nowDetailKey: stationKey(now.station),
    nextKey: PHASE_KEY[next.phase],
    nextDetailKey: stationKey(next.station),
    tone: PHASE_TONE[now.phase],
  }
}
