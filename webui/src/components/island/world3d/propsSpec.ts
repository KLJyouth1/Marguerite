/**
 * 3D 岛屿「AI 自主设计」道具契约（纯函数，可 vitest）。
 *
 * P0 要求 ≥5 种可交互道具。这里的五种不是随便凑数——每一种都对应
 * 「智能体工作台」里一类真实对象，拾取语义就是 Agent 任务的触发器：
 *
 *   代码结晶（code_crystal）  ← 工具/代码产物   → 拾取 = 委托「分析这份产出」
 *   文档卷轴（doc_scroll）    ← 交付文档/产物   → 拾取 = 委托「讲解这份文档」
 *   数据果实（data_fruit）    ← 运行数据/指标   → 拾取 = 委托「洞察这些数据」
 *   任务灯塔（quest_beacon）  ← 挂起的委托     → 拾取 = 认领并推进委托
 *   灵感花（inspiration_bloom）← 记忆/创意     → 拾取 = 委托「头脑风暴」
 *
 * 几何全部是**程序化参数**（见 shape），渲染层用 three 基础几何体拼装——
 * 设计理由见 tierProfile.ts 文件头（AI 自主设计=代码契约、零素材、可热更新）。
 * 这里的形状参数故意保持极简（primitive + 顶点数），渲染层可自由做低多边形化。
 *
 * 诚实性红线：道具的**存在**不凭空捏造——buildPropPlacements 只接受真实来源
 * 计数（工具数/产物数/记忆召回数……全部来自 IslandRuntimeInput 归约），
 * 无数据就少道具，绝不「假装 Agent 干了很多活」。
 */

import type { IslandRuntimeInput } from '../islandWorld'

/** 道具种类（5 种，均可交互/拾取） */
export type PropKind = 'code_crystal' | 'doc_scroll' | 'data_fruit' | 'quest_beacon' | 'inspiration_bloom'

export const PROP_KINDS: readonly PropKind[] = [
  'code_crystal',
  'doc_scroll',
  'data_fruit',
  'quest_beacon',
  'inspiration_bloom',
]

/** 拾取道具后触发的 Agent 委托类别（翻译成 chat.send 提示词） */
export type PropInteraction = 'analyze' | 'explain' | 'insight' | 'quest' | 'brainstorm'

/**
 * 程序化几何描述（低多边形拼装参数）。
 * 每种形状由「主 primitive + 次 primitive」组合，顶点数刻意压低，
 * 渲染层直接映射到 three 的 ConeGeometry / OctahedronGeometry / SphereGeometry 等。
 */
export interface PropGeometry {
  /** 主体 primitive */
  main: 'octahedron' | 'cone' | 'sphere' | 'tower' | 'flower'
  /** 附加 primitive（可选，渲染层可省略） */
  accent?: 'prism' | 'scroll' | 'berry' | 'lamp' | 'petal'
  /** 低多边形面数档（1=极简 8 面，2=标准 32 面，3=精细 64 面） */
  detail: 1 | 2 | 3
  /** 主体缩放基数（渲染层再乘视距/档位系数） */
  scale: number
}

export interface PropSpec {
  kind: PropKind
  /** 交互语义（拾取后向 Agent 委托的类别） */
  interaction: PropInteraction
  geometry: PropGeometry
  /** 同屏道具上限（防刷屏；也是粒子预算之外的第二个内存闸门） */
  maxInstances: number
  /** i18n key 后缀：islandGame.world3d.prop.<kind>.{name,desc} */
  labelKey: string
  /** 拾取奖励的 EXP 基数（游戏侧微奖励，不影响引擎） */
  expReward: number
}

const SPECS: Record<PropKind, PropSpec> = {
  // 代码结晶：八面体 + 棱柱尖——「工具产出凝成晶体」的具象化
  code_crystal: {
    kind: 'code_crystal',
    interaction: 'analyze',
    geometry: { main: 'octahedron', accent: 'prism', detail: 2, scale: 0.8 },
    maxInstances: 6,
    labelKey: 'code_crystal',
    expReward: 12,
  },
  // 文档卷轴：圆锥卷轴 + 纸轴——「交付文档的卷宗」
  doc_scroll: {
    kind: 'doc_scroll',
    interaction: 'explain',
    geometry: { main: 'cone', accent: 'scroll', detail: 1, scale: 0.7 },
    maxInstances: 6,
    labelKey: 'doc_scroll',
    expReward: 10,
  },
  // 数据果实：球体果实 + 浆果串——「运行数据结出的果实」
  data_fruit: {
    kind: 'data_fruit',
    interaction: 'insight',
    geometry: { main: 'sphere', accent: 'berry', detail: 2, scale: 0.6 },
    maxInstances: 8,
    labelKey: 'data_fruit',
    expReward: 8,
  },
  // 任务灯塔：细塔 + 灯——「待认领委托的路标」
  quest_beacon: {
    kind: 'quest_beacon',
    interaction: 'quest',
    geometry: { main: 'tower', accent: 'lamp', detail: 2, scale: 1.2 },
    maxInstances: 3,
    labelKey: 'quest_beacon',
    expReward: 20,
  },
  // 灵感花：花朵 + 花瓣——「记忆/创意的绽放」
  inspiration_bloom: {
    kind: 'inspiration_bloom',
    interaction: 'brainstorm',
    geometry: { main: 'flower', accent: 'petal', detail: 3, scale: 0.9 },
    maxInstances: 4,
    labelKey: 'inspiration_bloom',
    expReward: 6,
  },
}

export function propSpec(kind: PropKind): PropSpec {
  return SPECS[kind]
}

/** 道具实例（世界内的一次摆放） */
export interface PropPlacement {
  id: string
  kind: PropKind
  /** 世界坐标（y 由渲染层贴地，契约只给 x/z） */
  x: number
  z: number
  /** 偏航角（rad）——确定性布局用，避免同款道具朝向一致显得复制粘贴 */
  rotation: number
  /** 命名（如工具名/产物名摘要；无名来源用序号） */
  name: string
}

/** 道具布置依据：全部来自真实运行时归约，来源缺失则该类道具不出现 */
export interface PropSourceCounts {
  tools: number
  artifacts: number
  recalls: number
  quests: number
  ensembleMembers: number
}

export function propSourceCountsOf(input: IslandRuntimeInput): PropSourceCounts {
  return {
    tools: input.tools.length,
    artifacts: input.artifacts.length,
    recalls: input.recall ? 1 : 0,
    quests: input.planSteps.length,
    ensembleMembers: input.ensembleMembers.length,
  }
}

/** FNV-1a 字符串哈希（与 worldSpec.hashSeed 同算法，保证布局确定性） */
function hashSeed(text: string): number {
  let h = 0x811c9dc5
  for (let i = 0; i < text.length; i += 1) {
    h ^= text.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return h >>> 0
}

/**
 * 确定性道具布局：同一份 (counts, seed) 永远得到同一套坐标。
 * 每类道具按种子散布在环带上（避开岛心建筑区），角度步长固定 →
 * 新增道具不会让已有道具瞬移，场景可按 id 增量 diff。
 */
export function buildPropPlacements(counts: PropSourceCounts, seed: string): PropPlacement[] {
  const placements: PropPlacement[] = []
  const base = hashSeed(seed || 'island-world3d')

  const plan: { kind: PropKind; count: number; radius: number; nameOf: (i: number) => string }[] = [
    // 工具 → 代码结晶（每个工具一枚，按工具数限量）
    {
      kind: 'code_crystal',
      count: Math.min(counts.tools, SPECS.code_crystal.maxInstances),
      radius: 16,
      nameOf: (i) => `tool-${i + 1}`,
    },
    // 产物 → 文档卷轴
    {
      kind: 'doc_scroll',
      count: Math.min(counts.artifacts, SPECS.doc_scroll.maxInstances),
      radius: 19,
      nameOf: (i) => `artifact-${i + 1}`,
    },
    // 记忆召回 → 灵感花（最多 1 朵：一次召回一朵花，诚实对应）
    {
      kind: 'inspiration_bloom',
      count: Math.min(counts.recalls, SPECS.inspiration_bloom.maxInstances),
      radius: 13,
      nameOf: () => 'memory',
    },
    // 计划步 → 任务灯塔（每 3 步一盏，最多 maxInstances 盏）
    {
      kind: 'quest_beacon',
      count: Math.min(Math.ceil(counts.quests / 3), SPECS.quest_beacon.maxInstances),
      radius: 22,
      nameOf: (i) => `quest-${i + 1}`,
    },
    // 编队成员/常驻数据 → 数据果实（每成员一枚 + 至少 1 枚岛果）
    {
      kind: 'data_fruit',
      count: Math.min(Math.max(counts.ensembleMembers, 1), SPECS.data_fruit.maxInstances),
      radius: 11,
      nameOf: (i) => `data-${i + 1}`,
    },
  ]

  for (const entry of plan) {
    const step = (Math.PI * 2) / Math.max(1, SPECS[entry.kind].maxInstances)
    // 角度相位来自 (kind, seed)：同 seed 决定性、不同类互不重叠
    const phase = ((base ^ hashSeed(entry.kind)) % 1024) / 1024 * Math.PI * 2
    for (let i = 0; i < entry.count; i += 1) {
      const angle = phase + i * step
      placements.push({
        id: `${entry.kind}:${i}`,
        kind: entry.kind,
        x: Math.cos(angle) * entry.radius,
        z: Math.sin(angle) * entry.radius,
        rotation: Math.PI / 2 - angle,
        name: entry.nameOf(i),
      })
    }
  }

  return placements
}

/** 拾取一个道具 → 给玩家的微奖励 EXP（纯计算，游戏侧使用） */
export function propExpReward(kind: PropKind): number {
  return SPECS[kind].expReward
}
