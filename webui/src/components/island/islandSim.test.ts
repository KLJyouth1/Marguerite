import { describe, expect, it } from 'vitest'
import { DEFAULT_ISLAND_SPEC, ISLAND_SPECS } from './islandSpec'
import { buildIslandWorld, emptyRuntimeInput } from './islandWorld'
import {
  cameraPosition,
  clamp,
  computeIslandFrame,
  createIslandCamera,
  dampCamera,
  entityVisual,
  fillDirection,
  focusCamera,
  fract,
  hashString,
  hashUnit,
  ISLAND_KEYBOARD,
  islandCameraLimits,
  keyboardCameraCommand,
  mixColor,
  orbitCamera,
  panCamera,
  shade,
  skyAt,
  luminance,
  waterColorAt,
  waterTintAt,
  waterWaveAt,
  zoomCamera,
} from './islandSim'
const SPEC = DEFAULT_ISLAND_SPEC
const DAY = SPEC.lighting.dayLength

const channels = (color: number) => [(color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff]
const channelSum = (color: number) => channels(color).reduce((a, b) => a + b, 0)

describe('islandSim — 昼夜与光照', () => {
  it('正午：白昼满格、无夜灯、太阳最亮', () => {
    const sky = skyAt(SPEC, DAY * 0.25)
    expect(sky.phase).toBe('day')
    expect(sky.dayFactor).toBeCloseTo(1, 6)
    expect(sky.nightFactor).toBeCloseTo(0, 6)
    expect(sky.sunIntensity).toBeGreaterThan(2)
    expect(sky.sunDirection.y).toBeGreaterThan(0)
  })

  it('午夜：月光滑落、灯塔光束最强', () => {
    const noon = skyAt(SPEC, DAY * 0.25)
    const midnight = skyAt(SPEC, DAY * 0.75)
    expect(midnight.phase).toBe('night')
    expect(midnight.nightFactor).toBeCloseTo(1, 6)
    expect(midnight.dayFactor).toBeCloseTo(0, 6)
    expect(midnight.sunColor).toBe(SPEC.lighting.moonColor)
    expect(midnight.sunIntensity).toBeLessThan(noon.sunIntensity)
    expect(midnight.beamIntensity).toBeGreaterThan(noon.beamIntensity)
    expect(midnight.ambientIntensity).toBeCloseTo(SPEC.lighting.ambientNight, 6)
  })

  it('日出/日落：dayProgress 决定 dawn 与 dusk', () => {
    expect(skyAt(SPEC, 0).phase).toBe('dawn')
    expect(skyAt(SPEC, DAY * 0.5).phase).toBe('dusk')
    expect(skyAt(SPEC, DAY * 0.55).phase).toBe('night')
  })

  it('夜空比白昼天空暗、黄昏有暖色（色带连续性）', () => {
    const day = skyAt(SPEC, DAY * 0.25)
    const night = skyAt(SPEC, DAY * 0.75)
    expect(channelSum(night.skyTop)).toBeLessThan(channelSum(day.skyTop))
    expect(channelSum(night.skyBottom)).toBeLessThan(channelSum(day.skyBottom))

    // 平滑：相邻 0.5s 采样之间，每个颜色通道变化不超过 40
    for (let i = 0; i < 40; i++) {
      const t = (i / 40) * DAY
      const a = skyAt(SPEC, t)
      const b = skyAt(SPEC, t + 0.5)
      const ca = channels(a.sunColor)
      const cb = channels(b.sunColor)
      for (let c = 0; c < 3; c++) {
        expect(Math.abs(ca[c] - cb[c]), `t=${t} 太阳色跳变`).toBeLessThanOrEqual(40)
      }
      expect(Math.abs(a.dayFactor - b.dayFactor)).toBeLessThanOrEqual(0.05)
    }
  })

  it('所有颜色都在合法 24 位范围内', () => {
    for (let i = 0; i < 24; i++) {
      const sky = skyAt(SPEC, (i / 24) * DAY)
      for (const color of [sky.sunColor, sky.skyTop, sky.skyBottom, sky.ambientColor, sky.fogColor]) {
        expect(Number.isInteger(color)).toBe(true)
        expect(color).toBeGreaterThanOrEqual(0)
        expect(color).toBeLessThanOrEqual(0xffffff)
      }
    }
  })

  it('确定性：同一 t 两次取样完全一致', () => {
    expect(skyAt(SPEC, 12.34)).toEqual(skyAt(SPEC, 12.34))
  })

  it('雾色在夜里贴近夜空（不掺白云），避免远处水面被染出一条亮带', () => {
    const day = skyAt(SPEC, DAY * 0.25)
    const night = skyAt(SPEC, DAY * 0.75)
    // 夜里雾色应几乎等于地平线色（与背景同色 → 水面远端自然融入夜空）
    expect(night.fogColor).toBe(night.skyBottom)
    // 白天则掺入云色，比地平线更亮（远处泛白的水汽感）
    expect(channelSum(day.fogColor)).toBeGreaterThan(channelSum(day.skyBottom))
  })
})

describe('islandSim — 水面与实体动画', () => {
  it('波高被限制在 waveHeight 内且随位置变化', () => {
    for (let i = 0; i < 50; i++) {
      const h = waterWaveAt(SPEC, i * 0.7, i * 1.3, i * 0.4)
      expect(Math.abs(h)).toBeLessThanOrEqual(SPEC.water.waveHeight + 1e-9)
    }
    expect(waterWaveAt(SPEC, 1, 1, 1)).not.toBe(waterWaveAt(SPEC, 5, 1, 1))
  })

  it('活动建筑最亮，idle 最暗；错误状态会脉动', () => {
    const sky = skyAt(SPEC, DAY * 0.75)
    const active = entityVisual(SPEC, { id: 'workshop:t1', kind: 'workshop', status: 'active' }, sky, 3)
    const idle = entityVisual(SPEC, { id: 'workshop:t2', kind: 'workshop', status: 'idle' }, sky, 3)
    expect(active.glow).toBeGreaterThan(idle.glow)
    expect(active.lantern).toBe(true)

    const err1 = entityVisual(SPEC, { id: 'e', kind: 'workshop', status: 'error' }, sky, 0)
    const err2 = entityVisual(SPEC, { id: 'e', kind: 'workshop', status: 'error' }, sky, 0.25)
    expect(err1.glow).not.toBeCloseTo(err2.glow, 3)
  })

  it('哨所/仓库没有灯笼（契约能力位生效）', () => {
    const sky = skyAt(SPEC, DAY * 0.75)
    const outpost = entityVisual(SPEC, { id: 'outpost:x', kind: 'outpost', status: 'active' }, sky, 1)
    expect(outpost.lantern).toBe(false)
  })

  it('风摆幅度受 motion.windSway 约束', () => {
    const sky = skyAt(SPEC, DAY * 0.25)
    for (let i = 0; i < 20; i++) {
      const v = entityVisual(SPEC, { id: `w${i}`, kind: 'workshop', status: 'success' }, sky, i * 0.7)
      expect(Math.abs(v.sway)).toBeLessThanOrEqual(SPEC.motion.windSway + 1e-9)
    }
  })

  it('一帧模拟覆盖全部实体与道路：活动道路才跑货物流', () => {
    const world = buildIslandWorld(
      {
        ...emptyRuntimeInput(),
        sessionKey: 's',
        tools: [
          { id: 't1', name: 'read_file', isRunning: true, isError: false },
          { id: 't2', name: 'read_file', isRunning: false, isError: false },
        ],
      },
      SPEC,
    )
    const frame = computeIslandFrame(SPEC, world, 5)
    expect(frame.entities).toHaveLength(world.entities.length)
    expect(frame.traffic).toHaveLength(world.links.length)
    for (const traffic of frame.traffic) {
      const link = world.links.find((l) => l.id === traffic.linkId)
      expect(traffic.active).toBe(link?.active)
      if (!traffic.active) expect(traffic.phase).toBe(-1)
      else expect(traffic.phase).toBeGreaterThanOrEqual(0)
    }
  })
})

describe('islandSim — 轨道相机交互', () => {
  it('默认相机在限制范围内且位于地平线上方', () => {
    const cam = createIslandCamera(SPEC)
    const limits = islandCameraLimits(SPEC)
    expect(cam.distance).toBeGreaterThanOrEqual(limits.minDistance)
    expect(cam.distance).toBeLessThanOrEqual(limits.maxDistance)
    expect(cameraPosition(cam).y).toBeGreaterThan(0)
  })

  it('俯仰角与距离都被夹紧（用户无法把镜头压进地面或拉出外太空）', () => {
    const limits = islandCameraLimits(SPEC)
    const cam = createIslandCamera(SPEC)
    const tooLow = orbitCamera(cam, 0, -99, SPEC)
    const tooHigh = orbitCamera(cam, 0, 99, SPEC)
    expect(tooLow.pitch).toBe(limits.minPitch)
    expect(tooHigh.pitch).toBe(limits.maxPitch)

    expect(zoomCamera(cam, 99, SPEC).distance).toBe(limits.maxDistance)
    expect(zoomCamera(cam, -99, SPEC).distance).toBe(limits.minDistance)
  })

  it('旋转不改变距离，平移被限制在岛域内', () => {
    const cam = createIslandCamera(SPEC)
    expect(orbitCamera(cam, 0.5, 0.1, SPEC).distance).toBe(cam.distance)
    const panned = panCamera(cam, 500, 500, SPEC)
    expect(Math.hypot(panned.targetX, panned.targetZ)).toBeLessThanOrEqual(islandCameraLimits(SPEC).maxTargetRadius + 1e-9)
  })

  it('阻尼收敛到目标（帧率无关）', () => {
    const from = createIslandCamera(SPEC)
    const to = { ...from, yaw: from.yaw + 1, distance: from.distance * 0.6 }
    let cam = from
    for (let i = 0; i < 120; i++) cam = dampCamera(cam, to, 1 / 60)
    expect(cam.yaw).toBeCloseTo(to.yaw, 2)
    expect(cam.distance).toBeCloseTo(to.distance, 2)
  })

  it('聚焦建筑：视点落到该建筑并拉近，且仍在限制内', () => {
    const focus = focusCamera(createIslandCamera(SPEC), { x: 4.2, z: -3.1 }, SPEC)
    expect(focus.targetX).toBeCloseTo(4.2, 6)
    expect(focus.targetZ).toBeCloseTo(-3.1, 6)
    const limits = islandCameraLimits(SPEC)
    expect(focus.distance).toBeGreaterThanOrEqual(limits.minDistance)
    expect(focus.distance).toBeLessThanOrEqual(limits.maxDistance)
  })
})

describe('islandSim — 基础数学', () => {
  it('夹紧/取余/插值/明暗', () => {
    expect(clamp(5, 0, 1)).toBe(1)
    expect(clamp(-5, 0, 1)).toBe(0)
    expect(fract(-0.25)).toBeCloseTo(0.75, 6)
    expect(mixColor(0x000000, 0xffffff, 0.5)).toBe(0x808080)
    expect(shade(0xffffff, 0)).toBe(0x000000)
    expect(shade(0xffffff, 1)).toBe(0xffffff)
  })

  it('哈希稳定且分布均匀（不依赖 Math.random）', () => {
    expect(hashString('workshop:t1')).toBe(hashString('workshop:t1'))
    expect(hashString('a')).not.toBe(hashString('b'))
    const buckets = new Array(10).fill(0)
    for (let i = 0; i < 1000; i++) buckets[Math.floor(hashUnit(i) * 10)] += 1
    for (const count of buckets) expect(count).toBeGreaterThan(50)
  })
})

describe('islandSim — 水体径向色阶（唯美海面）', () => {
  it('三个锚点精确落在调色板上（岸线=浅滩、30%=主体海色、外缘=远海）', () => {
    expect(waterColorAt(SPEC, 0)).toBe(SPEC.palette.waterShallow)
    expect(waterColorAt(SPEC, 0.3)).toBe(SPEC.palette.water)
    expect(waterColorAt(SPEC, 1)).toBe(SPEC.palette.waterDeep)
  })

  it('亮度单调递减：浅滩最亮、远海最暗（层次不得反转或塌成单色）', () => {
    const samples = [0, 0.1, 0.2, 0.3, 0.45, 0.6, 0.75, 0.9, 1].map((t) => luminance(waterColorAt(SPEC, t)))
    for (let i = 1; i < samples.length; i++) {
      expect(samples[i], `t 序号 ${i} 处亮度应低于前一点`).toBeLessThanOrEqual(samples[i - 1] + 1e-9)
    }
    // 层次必须「看得出来」：首尾亮度差不得小于 30（否则等于单色 + 雾）
    expect(samples[0] - samples[samples.length - 1]).toBeGreaterThan(30)
  })

  it('浅滩带刻意做窄：前 30% 完成 浅滩→海色，之后才缓慢加深', () => {
    // 0.3 处已完全到达主体海色，不再是浅滩色
    expect(waterColorAt(SPEC, 0.3)).not.toBe(SPEC.palette.waterShallow)
    // 0.6 处仍明显亮于远海（加深发生在后半段）
    expect(luminance(waterColorAt(SPEC, 0.6))).toBeGreaterThan(luminance(SPEC.palette.waterDeep) + 10)
  })

  it('越界 t 被夹紧、确定性（同输入同输出）', () => {
    expect(waterColorAt(SPEC, -1)).toBe(waterColorAt(SPEC, 0))
    expect(waterColorAt(SPEC, 5)).toBe(waterColorAt(SPEC, 1))
    expect(waterColorAt(SPEC, 0.42)).toBe(waterColorAt(SPEC, 0.42))
  })

  it('全部岛屿皮肤都满足「浅滩 > 海色 > 远海」不变量（含热插拔的暮岛）', () => {
    for (const spec of Object.values(ISLAND_SPECS)) {
      const P = spec.palette
      expect(luminance(P.waterShallow), `${spec.id} 浅滩应亮于海色`).toBeGreaterThan(luminance(P.water))
      expect(luminance(P.water), `${spec.id} 海色应亮于远海`).toBeGreaterThan(luminance(P.waterDeep))
      expect(waterColorAt(spec, 0)).toBe(P.waterShallow)
      expect(waterColorAt(spec, 1)).toBe(P.waterDeep)
    }
  })

  it('luminance 与通道加权一致（纯函数）', () => {
    expect(luminance(0x000000)).toBe(0)
    expect(luminance(0xffffff)).toBeCloseTo(255, 6)
    expect(luminance(0xff0000)).toBeCloseTo(255 * 0.2126, 6)
  })

  it('昼夜染色：白天恰好中性白（不污染色阶），入夜压暗但不发黑', () => {
    // 白天必须是纯白——否则会与顶点色相乘、把浅滩色阶整体带偏
    expect(waterTintAt(SPEC, 0)).toBe(0xffffff)
    const noon = luminance(waterTintAt(SPEC, 0))
    const night = luminance(waterTintAt(SPEC, 1))
    expect(night).toBeLessThan(noon)
    // 夜里仍要看得见海的层次：不得压到发黑
    expect(night).toBeGreaterThan(60)
    // 越界 nightFactor 被夹紧
    expect(waterTintAt(SPEC, -1)).toBe(0xffffff)
    expect(waterTintAt(SPEC, 9)).toBe(waterTintAt(SPEC, 1))
  })
})

describe('islandSim — 逆光补光（岛缘侧壁不得压成黑描边）', () => {
  it('方向是太阳的水平对侧，且固定中等仰角（不随日高沉入地平线）', () => {
    for (const t of [0, DAY * 0.25, DAY * 0.5, DAY * 0.75]) {
      const sky = skyAt(SPEC, t)
      const f = fillDirection(sky)
      const s = sky.sunDirection
      const sunH = Math.hypot(s.x, s.z) || 1
      const fillH = Math.hypot(f.x, f.z)
      // 水平方向与太阳相反（点积 < 0）
      expect((f.x * s.x + f.z * s.z) / (fillH * sunH), `t=${t}`).toBeLessThan(0)
      // 水平分量已归一（补光方向只由方位决定）
      expect(fillH).toBeCloseTo(1, 6)
      // 仰角固定
      expect(f.y).toBe(0.5)
    }
  })

  it('正午（太阳最高）时仍有足够水平分量照亮侧壁', () => {
    const f = fillDirection(skyAt(SPEC, DAY * 0.25))
    expect(Math.hypot(f.x, f.z)).toBeGreaterThan(0.9)
  })

  it('确定性：同一天空状态给出同方向', () => {
    const sky = skyAt(SPEC, DAY * 0.31)
    expect(fillDirection(sky)).toEqual(fillDirection(sky))
  })

  it('补光强度规格：正数且为有限数（0 会让岛缘重新压黑）', () => {
    for (const spec of Object.values(ISLAND_SPECS)) {
      expect(Number.isFinite(spec.lighting.fillIntensity), spec.id).toBe(true)
      expect(spec.lighting.fillIntensity, spec.id).toBeGreaterThan(0)
    }
  })
})

describe('islandSim — 键盘导航映射（纯函数）', () => {
  it('方向键 → 绕行；左右改偏航、上下改俯仰，且互不串扰', () => {
    const left = keyboardCameraCommand('ArrowLeft')
    const right = keyboardCameraCommand('ArrowRight')
    const up = keyboardCameraCommand('ArrowUp')
    const down = keyboardCameraCommand('ArrowDown')
    expect(left).toEqual({ kind: 'orbit', yaw: -ISLAND_KEYBOARD.orbitYaw, pitch: 0 })
    expect(right).toEqual({ kind: 'orbit', yaw: ISLAND_KEYBOARD.orbitYaw, pitch: 0 })
    expect(up).toEqual({ kind: 'orbit', yaw: 0, pitch: ISLAND_KEYBOARD.orbitPitch })
    expect(down).toEqual({ kind: 'orbit', yaw: 0, pitch: -ISLAND_KEYBOARD.orbitPitch })
    // 左右方向相反、上下方向相反（按反了会让操作感觉「打架」）
    expect((left as { yaw: number }).yaw).toBeLessThan(0)
    expect((right as { yaw: number }).yaw).toBeGreaterThan(0)
    expect((up as { pitch: number }).pitch).toBeGreaterThan(0)
    expect((down as { pitch: number }).pitch).toBeLessThan(0)
  })

  it('Shift + 方向键 → 平移（不再绕行），左右与前后方向自洽', () => {
    expect(keyboardCameraCommand('ArrowLeft', true)).toEqual({ kind: 'pan', dx: -ISLAND_KEYBOARD.panStep, dy: 0 })
    expect(keyboardCameraCommand('ArrowRight', true)).toEqual({ kind: 'pan', dx: ISLAND_KEYBOARD.panStep, dy: 0 })
    expect(keyboardCameraCommand('ArrowUp', true)).toEqual({ kind: 'pan', dx: 0, dy: ISLAND_KEYBOARD.panStep })
    expect(keyboardCameraCommand('ArrowDown', true)).toEqual({ kind: 'pan', dx: 0, dy: -ISLAND_KEYBOARD.panStep })
  })

  it('+/- 缩放方向正确（+ 拉近 = 负增量），并兼容主键盘的 = 与 _', () => {
    const zoomIn = keyboardCameraCommand('+')
    const zoomOut = keyboardCameraCommand('-')
    expect(zoomIn).toEqual({ kind: 'zoom', delta: -ISLAND_KEYBOARD.zoomStep })
    expect(zoomOut).toEqual({ kind: 'zoom', delta: ISLAND_KEYBOARD.zoomStep })
    // 「+」在不按 Shift 的键位上是「=」，两者必须等价，否则用户按不出来
    expect(keyboardCameraCommand('=')).toEqual(zoomIn)
    expect(keyboardCameraCommand('_')).toEqual(zoomOut)
  })

  it('0 → 复位；其余键返回 null（不抢 Esc/字母键，交给上层）', () => {
    expect(keyboardCameraCommand('0')).toEqual({ kind: 'reset' })
    for (const key of ['Escape', 'a', 'Enter', ' ', 'Tab', 'PageUp']) {
      expect(keyboardCameraCommand(key), `${key} 不应被相机接管`).toBeNull()
    }
  })

  it('命令应用后仍在相机限制内（缩放/俯仰/平移都不会越界）', () => {
    const limits = islandCameraLimits(SPEC)
    let cam = createIslandCamera(SPEC)
    // 疯狂按键：任一维都不得越界
    for (let i = 0; i < 200; i++) {
      for (const key of ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', '+', '-']) {
        const cmd = keyboardCameraCommand(key)!
        if (cmd.kind === 'orbit') cam = orbitCamera(cam, cmd.yaw, cmd.pitch, SPEC)
        else if (cmd.kind === 'zoom') cam = zoomCamera(cam, cmd.delta, SPEC)
      }
    }
    expect(cam.distance).toBeGreaterThanOrEqual(limits.minDistance)
    expect(cam.distance).toBeLessThanOrEqual(limits.maxDistance)
    expect(cam.pitch).toBeGreaterThanOrEqual(limits.minPitch)
    expect(cam.pitch).toBeLessThanOrEqual(limits.maxPitch)
    // 平移也要被夹在岛域内
    let panned = cam
    for (let i = 0; i < 200; i++) {
      const cmd = keyboardCameraCommand('ArrowRight', true)!
      if (cmd.kind === 'pan') panned = panCamera(panned, cmd.dx, cmd.dy, SPEC)
    }
    expect(Math.hypot(panned.targetX, panned.targetZ)).toBeLessThanOrEqual(limits.maxTargetRadius + 1e-9)
  })

  it('步长为正有限数（0 会让按键看起来「没反应」）', () => {
    for (const v of Object.values(ISLAND_KEYBOARD)) {
      expect(Number.isFinite(v)).toBe(true)
      expect(v).toBeGreaterThan(0)
    }
  })
})