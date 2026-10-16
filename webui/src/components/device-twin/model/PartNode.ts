/**
 * PartNode — 装配节点（纯数据，不依赖 three.js，可在 node 环境单测）。
 *
 * 每个部件是一个独立节点：独立 id、metadata 档案、运动变换提示。
 * 渲染层（DeviceTwinStage.vue）按这份 spec 程序化生成几何 —— 几何尺寸/位置/爆炸偏移
 * 全部收敛在数据里，与 islandSpec.ts 的「形状即数据」范式一致。
 *
 * 颜色通过 colorRole 语义引用（body/interior/metal/rubber/glass/copper/hot/cold），
 * 渲染层再从 design token 解析为具体色值 —— 这里**不写死 hex**。
 */

/** 部件语义色角色（渲染层映射到 design token / three material color） */
export type PartColorRole =
  | 'body'        // 外壳（面板/侧板）
  | 'interior'    // 内胆（白色塑料/内壁）
  | 'metal'       // 金属结构（铰链/滑轨/螺丝）
  | 'rubber'      // 密封条/防震垫
  | 'glass'       // 透明件（搁板/视窗）
  | 'copper'      // 管路/工质通道
  | 'hot'         // 散热侧（冷凝器/排气管）
  | 'cold'        // 制冷侧（蒸发器/出风口）
  | 'accent'      // 强调色（显示屏/指示灯）

/** 程序化几何类型（渲染层据此选 BoxGeometry / CylinderGeometry / TubeGeometry …） */
export type PartGeometryKind =
  | 'box'
  | 'panel'       // 薄板（门板/侧板）—— box 的语义别名，渲染层可薄化
  | 'cylinder'
  | 'tube'        // 弯管（配合 FlowParticles 的路径）
  | 'shelf'       // 搁板（透明薄盒）
  | 'compressor'  // 压缩机（不规则块，用圆柱+盒近似）

export interface PartGeometrySpec {
  kind: PartGeometryKind
  /** 尺寸：box/panel/shelf = [w,h,d]；cylinder = [radius, height]；tube/compressor 忽略 */
  size: [number, number, number] | [number, number] | number
  colorRole: PartColorRole
}

/**
 * 运动变换提示。所有部件共享同一套语义，渲染层用 `1 - exp(-dt*speed)` 缓动。
 * - base：整机（product 态）中的位置
 * - explodeOffset：爆炸态在此基础上叠加的位移
 * - openAxis/openAngle：开合绕哪个轴转多少弧度（门/翻盖）
 * - pullAxis/pullDistance：抽屉沿哪个轴拉出多少（世界单位）
 * - cutAxis/cutDistance：剖面态沿哪个轴切除到什么距离
 */
export interface PartTransform {
  base: [number, number, number]
  explodeOffset: [number, number, number]
  openAxis?: 'x' | 'y' | 'z'
  openAngle?: number
  pullAxis?: 'x' | 'y' | 'z'
  pullDistance?: number
  cutAxis?: 'x' | 'y' | 'z'
  cutDistance?: number
}

/** 部件档案（metadata 驱动说明卡，不手写 HTML） */
export interface PartMetadata {
  id: string
  name: string
  english: string
  category: string
  intro: string
  facts: [string, string][]
}

/** 运动组（门/抽屉/压缩机…），联锁控制器按组做约束 */
export type KinematicGroup =
  | 'upperDoor'
  | 'lowerDoor'
  | 'upperDrawer'
  | 'lowerDrawer'
  | 'compressor'
  | 'fixed'

export interface PartNode {
  id: string
  metadata: PartMetadata
  geometry: PartGeometrySpec
  transform: PartTransform
  /** 所属运动组（fixed = 不可动） */
  group: KinematicGroup
  /** 子节点（如门把手上的指示灯） */
  children?: PartNode[]
}

/** 递归收集所有叶子+节点 id（爆炸/特写需要全集） */
export function collectPartIds(root: PartNode): string[] {
  const ids: string[] = [root.id]
  for (const child of root.children ?? []) ids.push(...collectPartIds(child))
  return ids
}

/** 按 id 深度搜索一个节点（特写模式定位用） */
export function findPart(root: PartNode, id: string): PartNode | null {
  if (root.id === id) return root
  for (const child of root.children ?? []) {
    const hit = findPart(child, id)
    if (hit) return hit
  }
  return null
}
