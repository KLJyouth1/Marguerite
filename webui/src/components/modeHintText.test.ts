import { describe, expect, it } from 'vitest'

import { stripDuplicateLabel } from './modeHintText'

describe('stripDuplicateLabel', () => {
  it('剥掉以别名开头的 hint 前缀（真机「全知超验 · 全知超验」的回归守卫）', () => {
    expect(stripDuplicateLabel('全知超验 · 跨模态综合推理', '全知超验')).toBe('跨模态综合推理')
    expect(stripDuplicateLabel('Omniscience · synthèse multimodale', 'Omniscience')).toBe(
      'synthèse multimodale',
    )
    expect(stripDuplicateLabel('全知超越：クロスモーダル', '全知超越')).toBe('クロスモーダル')
  })

  it('不含重复别名时原样返回', () => {
    expect(stripDuplicateLabel('默认模式 · 只读自由', '日常协作')).toBe('默认模式 · 只读自由')
    expect(stripDuplicateLabel('声明式思考深度与验证遍数', '深度打磨')).toBe(
      '声明式思考深度与验证遍数',
    )
  })

  it('hint 与别名完全相同时返回空串（不留一个孤立分隔符）', () => {
    expect(stripDuplicateLabel('全知超验', '全知超验')).toBe('')
  })

  it('空输入不抛错', () => {
    expect(stripDuplicateLabel('', 'x')).toBe('')
    expect(stripDuplicateLabel('abc', '')).toBe('abc')
    expect(stripDuplicateLabel(undefined as unknown as string, 'x')).toBe('')
  })
})
