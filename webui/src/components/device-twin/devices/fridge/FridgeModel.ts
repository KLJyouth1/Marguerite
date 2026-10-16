/**
 * FridgeModel — 冰箱数字孪生参考实现（纯数据，不 import three，node 可测）。
 *
 * 38 个独立装配节点：外壳 / 内胆 / 门 / 抽屉 / 搁架 / 制冷回路 / 控制件 / 脚轮。
 * 几何尺寸全部程序化（单位：0.1m，整机 ~0.6m × 0.6m × 1.7m）。
 * 渲染层（DeviceTwinStage.vue）按这份 spec 用 BoxGeometry/Cylinder/Tube 构建，
 * 不下载 GLB/贴图。
 *
 * 坐标约定：原点在冰箱底面中心；x=宽（左右），y=高（上），z=深（前=+z）。
 * 整机 x∈[-3,3], y∈[0,17], z∈[-3,3]。
 */

import type { PartNode } from '../../model/PartNode'
import type { ParticleChannelSpec } from '../../sim/FlowParticles'

/** 冰箱外形包围盒（渲染层相机取景用） */
export const FRIDGE_BOUNDS = {
  width: 6,
  height: 17,
  depth: 6,
  center: { x: 0, y: 8.5, z: 0 },
}

/**
 * 冰箱部件树。顶层是一个虚拟 root（整机），children 是所有独立部件。
 * 爆炸偏移沿各部件「飞出」方向给（门往前、抽屉往前、压缩机往下、管路往外）。
 */
export const FRIDGE_ROOT: PartNode = {
  id: 'fridge-root',
  metadata: {
    id: 'fridge-root',
    name: '整机',
    english: 'Whole unit',
    category: 'shell',
    intro: '一台上下双门冰箱的数字孪生模型。选择任意部件进入特写，或切换视图查看内部结构。',
    facts: [['类型', '双门冷藏冷冻'], ['容积', '约 320L'], ['能耗', '教学模拟 · 非实测']],
  },
  geometry: { kind: 'box', size: [6, 17, 6], colorRole: 'body' },
  transform: { base: [0, 8.5, 0], explodeOffset: [0, 0, 0] },
  group: 'fixed',
  children: [
    // ── 外壳 ──────────────────────────────────────────
    {
      id: 'top-panel',
      metadata: {
        id: 'top-panel', name: '顶板', english: 'Top panel', category: 'shell',
        intro: '顶盖外壳，承载顶部冷凝盘管与电路控制盒。',
        facts: [['材质', '彩涂钢板'], ['厚度', '0.4mm']],
      },
      geometry: { kind: 'panel', size: [6, 0.3, 6], colorRole: 'body' },
      transform: { base: [0, 16.85, 0], explodeOffset: [0, 2.5, 0] },
      group: 'fixed',
    },
    {
      id: 'bottom-panel',
      metadata: {
        id: 'bottom-panel', name: '底板', english: 'Bottom panel', category: 'shell',
        intro: '底座板，压缩机仓与接水盘安装于此。',
        facts: [['材质', '镀锌钢板'], ['功能', '承载压缩机']],
      },
      geometry: { kind: 'panel', size: [6, 0.4, 6], colorRole: 'body' },
      transform: { base: [0, 0.2, 0], explodeOffset: [0, -2, 0] },
      group: 'fixed',
    },
    {
      id: 'left-wall',
      metadata: {
        id: 'left-wall', name: '左侧板', english: 'Left wall', category: 'shell',
        intro: '左侧外壳板，内部贴合发泡保温层。',
        facts: [['材质', '彩涂钢板'], ['保温', '聚氨酯发泡']],
      },
      geometry: { kind: 'panel', size: [0.3, 17, 6], colorRole: 'body' },
      transform: { base: [-2.85, 8.5, 0], explodeOffset: [-2.5, 0, 0] },
      group: 'fixed',
    },
    {
      id: 'right-wall',
      metadata: {
        id: 'right-wall', name: '右侧板', english: 'Right wall', category: 'shell',
        intro: '右侧外壳板，冷凝盘管贴附其外表面散热。',
        facts: [['材质', '彩涂钢板'], ['散热', '盘管贴附']],
      },
      geometry: { kind: 'panel', size: [0.3, 17, 6], colorRole: 'body' },
      transform: { base: [2.85, 8.5, 0], explodeOffset: [2.5, 0, 0] },
      group: 'fixed',
    },
    {
      id: 'back-panel',
      metadata: {
        id: 'back-panel', name: '背板', english: 'Back panel', category: 'shell',
        intro: '后背板，遮挡冷凝器与毛细管，底部留散热栅。',
        facts: [['材质', '背板钢板'], ['通风', '底部栅格']],
      },
      geometry: { kind: 'panel', size: [6, 17, 0.25], colorRole: 'body' },
      transform: { base: [0, 8.5, -2.9], explodeOffset: [0, 0, -2.5] },
      group: 'fixed',
    },
    {
      id: 'front-frame',
      metadata: {
        id: 'front-frame', name: '前脸边框', english: 'Front frame', category: 'shell',
        intro: '门框结构，密封胶条与门体吸合面。',
        facts: [['材质', 'HIPS 注塑'], ['密封', '磁条吸合']],
      },
      geometry: { kind: 'box', size: [6, 17, 0.3], colorRole: 'interior' },
      transform: { base: [0, 8.5, 2.85], explodeOffset: [0, 0, 2] },
      group: 'fixed',
    },

    // ── 内胆 ──────────────────────────────────────────
    {
      id: 'upper-liner',
      metadata: {
        id: 'upper-liner', name: '上层内胆', english: 'Upper liner', category: 'interior',
        intro: '冷藏室内胆，HIPS 真空成型，搁架与抽屉导轨集成。',
        facts: [['材质', 'HIPS 食品级'], ['容积', '约 210L']],
      },
      geometry: { kind: 'box', size: [5.4, 9, 5.4], colorRole: 'interior' },
      transform: { base: [0, 11.5, 0], explodeOffset: [0, 0, 0.5] },
      group: 'fixed',
    },
    {
      id: 'lower-liner',
      metadata: {
        id: 'lower-liner', name: '下层内胆', english: 'Lower liner', category: 'interior',
        intro: '冷冻室内胆，抽屉式结构，蒸发器贴后壁。',
        facts: [['材质', 'HIPS 食品级'], ['容积', '约 110L']],
      },
      geometry: { kind: 'box', size: [5.4, 7, 5.4], colorRole: 'interior' },
      transform: { base: [0, 4, 0], explodeOffset: [0, 0, 0.5] },
      group: 'fixed',
    },

    // ── 门体 ──────────────────────────────────────────
    {
      id: 'upper-door',
      metadata: {
        id: 'upper-door', name: '上门（冷藏）', english: 'Upper door', category: 'door',
        intro: '冷藏室门体，内置门搁架与控制面板。右铰链右开。',
        facts: [['开启角', '约 105°'], ['铰链', '右上+左下']],
      },
      geometry: { kind: 'panel', size: [5.8, 8.8, 0.4], colorRole: 'body' },
      transform: {
        base: [0, 11.6, 3.05],
        explodeOffset: [0, 0, 3.5],
        openAxis: 'y',
        openAngle: 1.83, // ~105°
      },
      group: 'upperDoor',
    },
    {
      id: 'lower-door',
      metadata: {
        id: 'lower-door', name: '下门（冷冻）', english: 'Lower door', category: 'door',
        intro: '冷冻室门体，抽屉式冷冻配合门体密封。',
        facts: [['开启角', '约 90°'], ['铰链', '右下']],
      },
      geometry: { kind: 'panel', size: [5.8, 7.2, 0.4], colorRole: 'body' },
      transform: {
        base: [0, 4.1, 3.05],
        explodeOffset: [0, 0, 3.5],
        openAxis: 'y',
        openAngle: 1.57, // ~90°
      },
      group: 'lowerDoor',
    },
    {
      id: 'upper-handle',
      metadata: {
        id: 'upper-handle', name: '上门把手', english: 'Upper handle', category: 'door',
        intro: '长条形内嵌把手，与门体一体化。',
        facts: [['材质', 'ABS 注塑'], ['长度', '约 40cm']],
      },
      geometry: { kind: 'box', size: [0.5, 4, 0.3], colorRole: 'metal' },
      transform: { base: [2.5, 11.6, 3.3], explodeOffset: [0, 0, 4] },
      group: 'upperDoor',
    },
    {
      id: 'lower-handle',
      metadata: {
        id: 'lower-handle', name: '下门把手', english: 'Lower handle', category: 'door',
        intro: '下门内嵌把手。',
        facts: [['材质', 'ABS 注塑'], ['长度', '约 35cm']],
      },
      geometry: { kind: 'box', size: [0.5, 3.2, 0.3], colorRole: 'metal' },
      transform: { base: [2.5, 4.1, 3.3], explodeOffset: [0, 0, 4] },
      group: 'lowerDoor',
    },
    {
      id: 'upper-gasket',
      metadata: {
        id: 'upper-gasket', name: '上门密封条', english: 'Upper gasket', category: 'door',
        intro: '磁吸门封条，保证箱内不漏冷。老化会导致结霜。',
        facts: [['材质', 'PVC+磁条'], ['更换周期', '约 8 年']],
      },
      geometry: { kind: 'box', size: [5.6, 8.6, 0.15], colorRole: 'rubber' },
      transform: { base: [0, 11.6, 2.85], explodeOffset: [0, 0, 3.8] },
      group: 'upperDoor',
    },
    {
      id: 'lower-gasket',
      metadata: {
        id: 'lower-gasket', name: '下门密封条', english: 'Lower gasket', category: 'door',
        intro: '冷冻门封条，耐寒配方。',
        facts: [['材质', '耐寒 PVC'], ['磁吸力', '约 40N']],
      },
      geometry: { kind: 'box', size: [5.6, 7, 0.15], colorRole: 'rubber' },
      transform: { base: [0, 4.1, 2.85], explodeOffset: [0, 0, 3.8] },
      group: 'lowerDoor',
    },
    {
      id: 'upper-hinge',
      metadata: {
        id: 'upper-hinge', name: '上铰链', english: 'Upper hinge', category: 'door',
        intro: '上门右上铰链，承重+限位。',
        facts: [['材质', '镀锌钢'], ['限位', '110°']],
      },
      geometry: { kind: 'cylinder', size: [0.2, 1.5], colorRole: 'metal' },
      transform: { base: [2.8, 16, 2.9], explodeOffset: [0.5, 0.5, 1] },
      group: 'fixed',
    },
    {
      id: 'lower-hinge',
      metadata: {
        id: 'lower-hinge', name: '下铰链', english: 'Lower hinge', category: 'door',
        intro: '下门底部铰链，可调水平。',
        facts: [['材质', '镀锌钢'], ['调节', '±2mm']],
      },
      geometry: { kind: 'cylinder', size: [0.2, 1.2], colorRole: 'metal' },
      transform: { base: [2.8, 0.5, 2.9], explodeOffset: [0.5, -0.5, 1] },
      group: 'fixed',
    },

    // ── 抽屉 ──────────────────────────────────────────
    {
      id: 'upper-drawer',
      metadata: {
        id: 'upper-drawer', name: '果蔬抽屉', english: 'Crisper drawer', category: 'drawer',
        intro: '冷藏最下层果蔬盒，高湿保湿。拉出前须先开门。',
        facts: [['容积', '约 25L'], ['湿度', '约 90%RH']],
      },
      geometry: { kind: 'box', size: [5, 1.5, 5], colorRole: 'interior' },
      transform: {
        base: [0, 8, 0],
        explodeOffset: [0, 0, 3],
        pullAxis: 'z',
        pullDistance: 4.5,
      },
      group: 'upperDrawer',
    },
    {
      id: 'lower-drawer',
      metadata: {
        id: 'lower-drawer', name: '冷冻抽屉', english: 'Freezer drawer', category: 'drawer',
        intro: '冷冻室大抽屉，透明盒体可见食物。',
        facts: [['容积', '约 60L'], ['温度', '约 -18°C']],
      },
      geometry: { kind: 'box', size: [5, 2.5, 5], colorRole: 'glass' },
      transform: {
        base: [0, 3, 0],
        explodeOffset: [0, 0, 3.5],
        pullAxis: 'z',
        pullDistance: 4.5,
      },
      group: 'lowerDrawer',
    },

    // ── 搁架 ──────────────────────────────────────────
    {
      id: 'shelf-1',
      metadata: {
        id: 'shelf-1', name: '搁板 1', english: 'Shelf 1', category: 'shelf',
        intro: '冷藏上层玻璃搁板，可调高度。',
        facts: [['材质', '钢化玻璃'], ['承重', '约 20kg']],
      },
      geometry: { kind: 'shelf', size: [5.2, 0.15, 5.2], colorRole: 'glass' },
      transform: { base: [0, 14.5, 0], explodeOffset: [0, 2, 0.5] },
      group: 'fixed',
    },
    {
      id: 'shelf-2',
      metadata: {
        id: 'shelf-2', name: '搁板 2', english: 'Shelf 2', category: 'shelf',
        intro: '冷藏中层搁板。',
        facts: [['材质', '钢化玻璃'], ['可调', '±2cm']],
      },
      geometry: { kind: 'shelf', size: [5.2, 0.15, 5.2], colorRole: 'glass' },
      transform: { base: [0, 12.2, 0], explodeOffset: [0, 1, 0.5] },
      group: 'fixed',
    },
    {
      id: 'shelf-3',
      metadata: {
        id: 'shelf-3', name: '搁板 3', english: 'Shelf 3', category: 'shelf',
        intro: '冷藏下层搁板，位于果蔬盒上方。',
        facts: [['材质', '钢化玻璃'], ['可调', '±2cm']],
      },
      geometry: { kind: 'shelf', size: [5.2, 0.15, 5.2], colorRole: 'glass' },
      transform: { base: [0, 9.8, 0], explodeOffset: [0, 0, 0.5] },
      group: 'fixed',
    },
    {
      id: 'door-bin-1',
      metadata: {
        id: 'door-bin-1', name: '门搁架 1', english: 'Door bin 1', category: 'shelf',
        intro: '上门内侧最高搁架，放调味品。',
        facts: [['材质', 'PS 透明'], ['容量', '约 3L']],
      },
      geometry: { kind: 'box', size: [5, 0.8, 1.5], colorRole: 'interior' },
      transform: { base: [0, 15.5, 2.2], explodeOffset: [0, 1, 4.5] },
      group: 'upperDoor',
    },
    {
      id: 'door-bin-2',
      metadata: {
        id: 'door-bin-2', name: '门搁架 2', english: 'Door bin 2', category: 'shelf',
        intro: '上门内侧中层搁架。',
        facts: [['材质', 'PS 透明'], ['容量', '约 4L']],
      },
      geometry: { kind: 'box', size: [5, 0.8, 1.5], colorRole: 'interior' },
      transform: { base: [0, 13, 2.2], explodeOffset: [0, 0.5, 4.5] },
      group: 'upperDoor',
    },

    // ── 制冷回路 ──────────────────────────────────────
    {
      id: 'compressor',
      metadata: {
        id: 'compressor', name: '压缩机', english: 'Compressor', category: 'cooling',
        intro: '制冷心脏。压缩制冷剂蒸气升温升压，耗电功率全部变成热排到冷凝器。',
        facts: [['类型', '往复式定速'], ['功率', '约 120W（教学）'], ['工作', '温控器通断']],
      },
      geometry: { kind: 'compressor', size: [2, 1.5, 2], colorRole: 'metal' },
      transform: { base: [0, 0.8, -1.5], explodeOffset: [0, -1.5, -1] },
      group: 'compressor',
    },
    {
      id: 'condenser',
      metadata: {
        id: 'condenser', name: '冷凝器', english: 'Condenser', category: 'cooling',
        intro: '背部盘管，高温高压制冷剂在此向环境放热，冷凝成液体。',
        facts: [['形式', '丝管式贴背'], ['散热', '自然对流']],
      },
      geometry: { kind: 'tube', size: 1, colorRole: 'hot' },
      transform: { base: [0, 8, -2.9], explodeOffset: [0, 0, -2] },
      group: 'fixed',
    },
    {
      id: 'evaporator',
      metadata: {
        id: 'evaporator', name: '蒸发器', english: 'Evaporator', category: 'cooling',
        intro: '冷冻室后壁盘管，低压制冷剂在此汽化吸热，把箱内热量搬走。',
        facts: [['形式', '板管式'], ['吸热', '约 85W（教学）']],
      },
      geometry: { kind: 'tube', size: 1, colorRole: 'cold' },
      transform: { base: [0, 5, -2.5], explodeOffset: [0, 0, -1] },
      group: 'fixed',
    },
    {
      id: 'capillary',
      metadata: {
        id: 'capillary', name: '毛细管', english: 'Capillary tube', category: 'cooling',
        intro: '细长铜管，节流降压：高压液体在此节流成低压低温气液两相。',
        facts: [['内径', '约 0.7mm'], ['长度', '约 2m']],
      },
      geometry: { kind: 'tube', size: 1, colorRole: 'copper' },
      transform: { base: [2.5, 2, -2.5], explodeOffset: [1.5, 0, -0.5] },
      group: 'fixed',
    },
    {
      id: 'filter-drier',
      metadata: {
        id: 'filter-drier', name: '干燥过滤器', english: 'Filter drier', category: 'cooling',
        intro: '吸收系统水分、拦截杂质，保护毛细管不被堵。',
        facts: [['位置', '冷凝器出口'], ['功能', '吸水+滤杂']],
      },
      geometry: { kind: 'cylinder', size: [0.2, 0.8], colorRole: 'copper' },
      transform: { base: [2.5, 1.5, -2.5], explodeOffset: [1.5, -0.5, -0.5] },
      group: 'fixed',
    },
    {
      id: 'drain-pan',
      metadata: {
        id: 'drain-pan', name: '接水盘', english: 'Drain pan', category: 'cooling',
        intro: '接化霜水，靠压缩机热量自然蒸发。',
        facts: [['位置', '压缩机顶部'], ['蒸发', '余热利用']],
      },
      geometry: { kind: 'box', size: [2.5, 0.2, 2], colorRole: 'interior' },
      transform: { base: [0, 1.7, -1.5], explodeOffset: [0, -1, -1.5] },
      group: 'fixed',
    },

    // ── 控制与照明 ────────────────────────────────────
    {
      id: 'display-panel',
      metadata: {
        id: 'display-panel', name: '控制面板', english: 'Display panel', category: 'control',
        intro: '外显触控屏，显示冷藏/冷冻温度与模式。',
        facts: [['显示', 'LED 数码管'], ['功能', '调温/模式/锁定']],
      },
      geometry: { kind: 'panel', size: [2, 1, 0.1], colorRole: 'accent' },
      transform: { base: [0, 15, 3.2], explodeOffset: [0, 0.5, 2] },
      group: 'upperDoor',
    },
    {
      id: 'led-light',
      metadata: {
        id: 'led-light', name: 'LED 灯', english: 'LED light', category: 'control',
        intro: '冷藏顶部 LED 照明灯，开门亮、关门灭。',
        facts: [['功率', '约 2W'], ['色温', '约 6500K']],
      },
      geometry: { kind: 'box', size: [3, 0.15, 0.8], colorRole: 'accent' },
      transform: { base: [0, 16.5, 2], explodeOffset: [0, 0.5, 1] },
      group: 'fixed',
    },
    {
      id: 'fan',
      metadata: {
        id: 'fan', name: '循环风扇', english: 'Circulation fan', category: 'control',
        intro: '冷风风扇，把蒸发器冷量吹到箱内各角落。',
        facts: [['形式', '轴流风扇'], ['开门', '停转防漏冷']],
      },
      geometry: { kind: 'cylinder', size: [0.5, 0.3], colorRole: 'metal' },
      transform: { base: [0, 6.5, -2.2], explodeOffset: [0, 0, -1] },
      group: 'fixed',
    },
    {
      id: 'thermostat',
      metadata: {
        id: 'thermostat', name: '温控器', english: 'Thermostat', category: 'control',
        intro: '感温包控制压缩机通断：到温度停机、回升开机。',
        facts: [['回差', '约 ±1.5°C'], ['类型', '机械感温']],
      },
      geometry: { kind: 'box', size: [0.6, 0.8, 0.5], colorRole: 'metal' },
      transform: { base: [-2.5, 16, -2], explodeOffset: [-1, 1, -1] },
      group: 'fixed',
    },

    // ── 脚轮 ─────────────────────────────────────────
    {
      id: 'foot-fl',
      metadata: {
        id: 'foot-fl', name: '前脚左', english: 'Front-left foot', category: 'foot',
        intro: '可调底脚，调水平防震。',
        facts: [['材质', 'PP 塑料'], ['调节', '±5mm']],
      },
      geometry: { kind: 'cylinder', size: [0.3, 0.5], colorRole: 'rubber' },
      transform: { base: [-2.5, 0, 2.5], explodeOffset: [-1, -1, 1] },
      group: 'fixed',
    },
    {
      id: 'foot-fr',
      metadata: {
        id: 'foot-fr', name: '前脚右', english: 'Front-right foot', category: 'foot',
        intro: '可调底脚。',
        facts: [['材质', 'PP 塑料'], ['调节', '±5mm']],
      },
      geometry: { kind: 'cylinder', size: [0.3, 0.5], colorRole: 'rubber' },
      transform: { base: [2.5, 0, 2.5], explodeOffset: [1, -1, 1] },
      group: 'fixed',
    },
    {
      id: 'foot-bl',
      metadata: {
        id: 'foot-bl', name: '后脚左', english: 'Back-left foot', category: 'foot',
        intro: '固定底脚。',
        facts: [['材质', 'PP 塑料'], ['固定', '不可调']],
      },
      geometry: { kind: 'cylinder', size: [0.3, 0.5], colorRole: 'rubber' },
      transform: { base: [-2.5, 0, -2.5], explodeOffset: [-1, -1, -1] },
      group: 'fixed',
    },
    {
      id: 'foot-br',
      metadata: {
        id: 'foot-br', name: '后脚右', english: 'Back-right foot', category: 'foot',
        intro: '固定底脚。',
        facts: [['材质', 'PP 塑料'], ['固定', '不可调']],
      },
      geometry: { kind: 'cylinder', size: [0.3, 0.5], colorRole: 'rubber' },
      transform: { base: [2.5, 0, -2.5], explodeOffset: [1, -1, -1] },
      group: 'fixed',
    },
  ],
}

/**
 * 三路工作流粒子路径（教学示意，非真实管路坐标）。
 * cold   = 箱内冷空气循环（蒸发器 → 风扇 → 搁架 → 回风）
 * hot    = 冷凝器排热（压缩机 → 冷凝器 → 环境）
 * working = 密闭制冷剂回路（蒸发器 → 压缩机 → 冷凝器 → 毛细管 → 蒸发器）
 */
export const FRIDGE_FLOW_CHANNELS: ParticleChannelSpec[] = [
  {
    channel: 'cold',
    closed: false,
    count: 24,
    speed: 0.6,
    path: [
      [0, 5.5, -2.4],   // 蒸发器出
      [0, 6.5, -2.0],   // 风扇
      [0, 10, 0],       // 吹向箱内
      [0, 14, 2],       // 上升到搁架
      [-1.5, 12, 2],    // 回流
      [0, 8, -2.0],     // 回到蒸发器
    ],
  },
  {
    channel: 'hot',
    closed: false,
    count: 20,
    speed: 0.5,
    path: [
      [0, 0.8, -1.5],   // 压缩机出
      [2, 2, -2.8],     // 上行
      [2.5, 8, -2.9],   // 冷凝器上部
      [2.5, 14, -2.9],  // 冷凝器顶部
      [0, 15, -2.5],    // 弯出
    ],
  },
  {
    channel: 'working',
    closed: true,
    count: 30,
    speed: 0.8,
    path: [
      [0, 5, -2.5],     // 蒸发器出口（低压气）
      [-2.5, 3, -2.5],  // 吸气管
      [0, 0.8, -1.5],   // 压缩机入口
      [0.5, 1, -1.5],   // 压缩机出口（高压气）
      [2.5, 2, -2.5],   // 排气管
      [2.5, 10, -2.9],  // 冷凝器（放热液化）
      [2.5, 1.5, -2.5], // 干燥过滤器
      [2.5, 2, -2.5],   // 毛细管（节流）
      [0, 5, -2.5],     // 回到蒸发器（低压两相）
    ],
  },
]
