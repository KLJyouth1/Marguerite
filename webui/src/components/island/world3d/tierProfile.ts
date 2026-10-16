/**
 * 四档模型档位 → 岛屿「神力」表现映射（纯函数，可 vitest）。
 *
 * 设计依据 `_island_3d_game_design.md` §12：四档模型（旗舰推理 / 高效推理 /
 * 极速产出 / 全模态帕累托）在岛上对应四种「神力」域——粒子特效密度、道具
 * 生成速度、AI NPC 对话复杂度、昼夜/天象风格。
 *
 * 为什么是纯映射表而不是散落 if：
 * - 四档 × 多表现维度是笛卡尔积，手写三遍就会漂移（与 modelRouting.ts 的
 *   MODEL_ROUTING_BRIDGE 同样的教训——见该文件的单一事实源注释）；
 * - 纯函数可单测：档位升/降方向性、数值边界（粒子数/生成间隔）都可断言。
 *
 * 诚实性红线：档位只改变**表现层预算**（粒子数、生成节律、台词复杂度），
 * 绝不冒充引擎能力差异——引擎真实状态仍由 useEngineBridge 的 EngineSnapshot
 * 提供；本表在「引擎未知」时以 reasoningEffort ?? 'normal' 保守推导。
 *
 * ── 为什么程序化几何体而非外部素材（全模块通用）────────────────────────
 * P0 的玩法道具、地形、特效全部由 three 程序化几何体生成，不依赖任何
 * glTF/贴图素材：
 * 1. 「游戏内容全面由 AI 自主设计」——AI 产出的是**代码契约**（propsSpec /
 *    tierProfile），不是二进制资产；每次修改即热更新，无需素材管线；
 * 2. 零素材依赖 = 零版权风险、零网络加载失败面、桌面端离线可用；
 * 3. 程序化几何体天然参数化（粒子密度、生成速度按档位缩放），二进制素材
 *    做不到「同一份内容随档位重参数化」。
 * Blender 管线（blender/generate_island.py）是 P2 的**升级出口**而非依赖：
 * glTF 一旦可导出即可替换对应程序化件，P0 不阻塞在其上。
 */

import type { EngineSnapshot } from '../game/gameLogic'
import type { ModelRoutingMode } from '@/types/modelRouting'

/** 岛屿神力档（与设计文档 §12 的四档一一对应） */
export type World3dTier = 'flagship' | 'efficient' | 'sprint' | 'omni'

export const WORLD3D_TIERS: readonly World3dTier[] = ['flagship', 'efficient', 'sprint', 'omni']

/**
 * 档位 →「神力」表现预算。
 * - particleBudget：场景常驻粒子上限（拾取辉光/风场/神力气旋共用一池）
 * - spawnIntervalMs：程序化道具的再生成节律（越小生成越快）
 * - npcDialogueDepth：AI NPC 台词复杂度档（1 = 单句，3 = 多轮深谈）
 * - auroraIntensity：夜空极光/天象辉光强度 0..1（旗舰档专属域）
 * - timeScale：岛屿时间流速倍率（极速产出档的「时间加速」神力）
 */
export interface TierProfile {
  tier: World3dTier
  particleBudget: number
  spawnIntervalMs: number
  npcDialogueDepth: 1 | 2 | 3
  auroraIntensity: number
  timeScale: number
}

const PROFILES: Record<World3dTier, TierProfile> = {
  // 旗舰推理：深邃夜空 + 极光，粒子全开，NPC 深谈（thinking_depth 高）
  flagship: { tier: 'flagship', particleBudget: 720, spawnIntervalMs: 4000, npcDialogueDepth: 3, auroraIntensity: 1, timeScale: 1 },
  // 高效推理：均衡生产档，粒子适中，工坊流水线节律
  efficient: { tier: 'efficient', particleBudget: 360, spawnIntervalMs: 6000, npcDialogueDepth: 2, auroraIntensity: 0.4, timeScale: 1 },
  // 极速产出：时间加速 + 疾风赛道，粒子少而快，台词精简
  sprint: { tier: 'sprint', particleBudget: 180, spawnIntervalMs: 2500, npcDialogueDepth: 1, auroraIntensity: 0.15, timeScale: 1.6 },
  // 全模态帕累托：万象花园，帕累托最优的中位预算
  omni: { tier: 'omni', particleBudget: 480, spawnIntervalMs: 5000, npcDialogueDepth: 2, auroraIntensity: 0.7, timeScale: 1.1 },
}

export function tierProfile(tier: World3dTier): TierProfile {
  return PROFILES[tier]
}

/**
 * UI 档位（modelRouting.ModelRoutingMode）→ 岛屿神力档。
 *
 * 五档 UI 词表收敛到四档神力域（off 是「直连」而非独立算力档，归入高效推理；
 * 保持一一映射的方向性：路由越重 → 粒子越多/生成越快/台词越深）。
 */
export function tierForRoutingMode(mode: ModelRoutingMode): World3dTier {
  switch (mode) {
    case 'omni_agent':
      return 'omni'
    case 'bloom_ensemble':
      return 'flagship'
    case 'llm_ensemble':
      return 'efficient'
    case 'squilla_router':
      return 'efficient'
    case 'off':
    default:
      return 'sprint'
  }
}

/**
 * 从真实引擎快照推导神力档（Agent→游戏方向的档位来源）。
 *
 * 优先用 modeKind（bloom/omni-transcend → 重档），其次 reasoningEffort。
 * 快照未连接/字段缺失 → 保守回退 'efficient'（不假装「极速」也不冒充「旗舰」）。
 */
export function tierForSnapshot(engine: EngineSnapshot): World3dTier {
  const kind = engine.modeKind ?? ''
  if (kind === 'omni-transcend') return 'omni'
  if (kind === 'bloom') return 'flagship'
  if (kind === 'ensemble') return 'efficient'
  const effort = (engine.reasoningEffort ?? '').toLowerCase()
  if (effort === 'max' || effort === 'deep') return 'flagship'
  if (effort === 'off' || effort === 'low') return 'sprint'
  return 'efficient'
}

/**
 * 档位切换的方向性：粒子预算随档位「重→轻」单调不增。
 * （单测用来钉死映射表不会被改坏：旗舰 ≥ 全模态 ≥ 高效 ≥ 极速。）
 */
export function particleBudgetOrder(): number[] {
  return [PROFILES.flagship.particleBudget, PROFILES.omni.particleBudget, PROFILES.efficient.particleBudget, PROFILES.sprint.particleBudget]
}
