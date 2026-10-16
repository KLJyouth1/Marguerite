import { describe, expect, it } from 'vitest'
import { FRIDGE_ROOT, FRIDGE_FLOW_CHANNELS } from './FridgeModel'
import { collectPartIds, findPart } from '../../model/PartNode'
import { MetadataCatalog } from '../../model/MetadataCatalog'

describe('FridgeModel — 参考实现装配树', () => {
  const ids = collectPartIds(FRIDGE_ROOT)

  it('至少 35 个独立装配节点', () => {
    // 减去 root 自身
    expect(ids.length - 1).toBeGreaterThanOrEqual(35)
  })

  it('所有 id 唯一', () => {
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('每个节点都有完整 metadata（name/english/category/intro/facts）', () => {
    function check(node: typeof FRIDGE_ROOT): void {
      expect(node.metadata.name.length).toBeGreaterThan(0)
      expect(node.metadata.english.length).toBeGreaterThan(0)
      expect(node.metadata.category.length).toBeGreaterThan(0)
      expect(node.metadata.intro.length).toBeGreaterThan(0)
      expect(Array.isArray(node.metadata.facts)).toBe(true)
      for (const child of node.children ?? []) check(child)
    }
    check(FRIDGE_ROOT)
  })

  it('包含门/抽屉/制冷件关键部件', () => {
    for (const need of [
      'upper-door', 'lower-door', 'compressor', 'condenser',
      'evaporator', 'upper-drawer', 'lower-drawer', 'shelf-1',
    ]) {
      expect(findPart(FRIDGE_ROOT, need)).not.toBeNull()
    }
  })

  it('MetadataCatalog 索引覆盖全部节点', () => {
    const cat = new MetadataCatalog(FRIDGE_ROOT)
    expect(cat.size).toBe(ids.length)
  })

  it('三路流体通道语义色齐全（cold/hot/working）', () => {
    const channels = FRIDGE_FLOW_CHANNELS.map((c) => c.channel)
    expect(channels).toContain('cold')
    expect(channels).toContain('hot')
    expect(channels).toContain('working')
  })

  it('流体路径点数合法（>=2）且坐标有限', () => {
    for (const ch of FRIDGE_FLOW_CHANNELS) {
      expect(ch.path.length).toBeGreaterThanOrEqual(2)
      for (const pt of ch.path) {
        expect(pt).toHaveLength(3)
        for (const v of pt) expect(Number.isFinite(v)).toBe(true)
      }
    }
  })

  it('working 通道是闭合回路，cold/hot 是开放路径', () => {
    const w = FRIDGE_FLOW_CHANNELS.find((c) => c.channel === 'working')!
    expect(w.closed).toBe(true)
    const c = FRIDGE_FLOW_CHANNELS.find((ch) => ch.channel === 'cold')!
    expect(c.closed).toBe(false)
  })
})
