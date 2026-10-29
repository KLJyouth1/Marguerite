import { describe, expect, it } from 'vitest'

import {
  DEFAULT_MATERIAL_SOURCE_KIND,
  MATERIAL_SOURCE_KINDS,
  emptyMaterialSource,
  materialSourceAcceptsValue,
  materialSourceErrorKey,
  materialSourceIsValid,
  materialSourceNeedsValue,
  normalizeMaterialSource,
  type MaterialSource,
  type MaterialSourceKind,
} from './materialSource'

const source = (kind: MaterialSourceKind, value = ''): MaterialSource => ({ kind, value })

describe('materialSource — 四选一的口径', () => {
  it('展示顺序固定 A 网络 → B 自动生成 → C 指定目录 → D 其他', () => {
    expect(MATERIAL_SOURCE_KINDS).toEqual(['web', 'generate', 'directory', 'custom'])
    expect(DEFAULT_MATERIAL_SOURCE_KIND).toBe('generate')
  })

  it('默认值 = B（自动生成）且无需任何附加值即可提交', () => {
    const empty = emptyMaterialSource()
    expect(empty).toEqual({ kind: 'generate', value: '' })
    expect(materialSourceIsValid(empty)).toBe(true)
  })

  it('needsValue：只有 C/D 必须填值，A/B 无附加值也可提交', () => {
    expect({
      web: materialSourceNeedsValue('web'),
      generate: materialSourceNeedsValue('generate'),
      directory: materialSourceNeedsValue('directory'),
      custom: materialSourceNeedsValue('custom'),
    }).toEqual({ web: false, generate: false, directory: true, custom: true })
  })

  it('acceptsValue ≠ needsValue：A 携带选填 URL，只有 B 恒不携带', () => {
    expect({
      web: materialSourceAcceptsValue('web'),
      generate: materialSourceAcceptsValue('generate'),
      directory: materialSourceAcceptsValue('directory'),
      custom: materialSourceAcceptsValue('custom'),
    }).toEqual({ web: true, generate: false, directory: true, custom: true })
    // 「必须填」是「可携带」的子集：C/D 两者皆真，A 只满足后者。
    for (const kind of MATERIAL_SOURCE_KINDS) {
      if (materialSourceNeedsValue(kind)) expect(materialSourceAcceptsValue(kind)).toBe(true)
    }
  })
})

describe('materialSource — 提交真值表（kind × value）', () => {
  const table: Array<{ kind: MaterialSourceKind, value: string, valid: boolean, key: string | null }> = [
    // A 网络：URL 选填 → 空与非空都可提交
    { kind: 'web', value: '', valid: true, key: null },
    { kind: 'web', value: 'https://example.com/a.png', valid: true, key: null },
    // B 自动生成：永远可提交（带脏值也只算「没用到」，不拦提交）
    { kind: 'generate', value: '', valid: true, key: null },
    { kind: 'generate', value: 'C:/leftover', valid: true, key: null },
    // C 指定目录：空 / 纯空白 → 不可提交
    { kind: 'directory', value: '', valid: false, key: 'directoryRequired' },
    { kind: 'directory', value: '   ', valid: false, key: 'directoryRequired' },
    { kind: 'directory', value: '\t \n', valid: false, key: 'directoryRequired' },
    { kind: 'directory', value: 'D:/assets/poster', valid: true, key: null },
    // D 其他：同上，但报错键不同
    { kind: 'custom', value: '', valid: false, key: 'customRequired' },
    { kind: 'custom', value: '  ', valid: false, key: 'customRequired' },
    { kind: 'custom', value: '用户自己上传的一张草图', valid: true, key: null },
  ]

  for (const row of table) {
    it(`${row.kind} + value=${JSON.stringify(row.value)} → 可提交=${row.valid}`, () => {
      expect(materialSourceIsValid(source(row.kind, row.value))).toBe(row.valid)
      expect(materialSourceErrorKey(source(row.kind, row.value))).toBe(row.key)
    })
  }

  it('isValid 与 errorKey 严格互斥（不存在「无错键但不可提交」或反之）', () => {
    for (const kind of MATERIAL_SOURCE_KINDS) {
      for (const value of ['', '  ', 'x']) {
        const s = source(kind, value)
        expect(materialSourceIsValid(s)).toBe(materialSourceErrorKey(s) === null)
      }
    }
  })

  it('两个必填项各自报自己的键，不串用', () => {
    expect(materialSourceErrorKey(source('directory', ''))).toBe('directoryRequired')
    expect(materialSourceErrorKey(source('custom', ''))).toBe('customRequired')
  })
})

describe('materialSource — normalizeMaterialSource', () => {
  it('B：丢弃残留输入（切回自动生成不会把上一次的目录带走）', () => {
    expect(normalizeMaterialSource('generate', 'D:/assets/leftover')).toEqual({ kind: 'generate', value: '' })
  })

  it('A：保留**选填** URL（修前会被抹成空串，令 buildToolArgs 的 web→image 联动恒不生效）', () => {
    expect(normalizeMaterialSource('web', '  https://cdn/x.png  ')).toEqual({
      kind: 'web',
      value: 'https://cdn/x.png',
    })
    expect(normalizeMaterialSource('web', '')).toEqual({ kind: 'web', value: '' })
    expect(normalizeMaterialSource('web', '   ')).toEqual({ kind: 'web', value: '' })
  })

  it('C/D：trim 后保留；纯空白归一成空串（于是必然落到「不可提交」）', () => {
    expect(normalizeMaterialSource('directory', '  D:/assets  ')).toEqual({ kind: 'directory', value: 'D:/assets' })
    expect(normalizeMaterialSource('custom', '  草图  ')).toEqual({ kind: 'custom', value: '草图' })
    expect(normalizeMaterialSource('directory', ' \t ')).toEqual({ kind: 'directory', value: '' })
    expect(materialSourceIsValid(normalizeMaterialSource('directory', ' \t '))).toBe(false)
  })

  it('非法 kind 回落默认选项且不抛错（脏持久化数据），并因 B 语义清空 value', () => {
    const dirty = 'not-a-kind' as MaterialSourceKind
    expect(normalizeMaterialSource(dirty, 'D:/assets')).toEqual({ kind: DEFAULT_MATERIAL_SOURCE_KIND, value: '' })
  })

  it('rawValue 非字符串（null/undefined）不抛错，归一为空串', () => {
    expect(normalizeMaterialSource('directory', null as unknown as string)).toEqual({ kind: 'directory', value: '' })
    expect(normalizeMaterialSource('custom', undefined as unknown as string)).toEqual({ kind: 'custom', value: '' })
  })

  it('归一化是可重入的：normalize(normalize(x)) === normalize(x)', () => {
    for (const kind of MATERIAL_SOURCE_KINDS) {
      const once = normalizeMaterialSource(kind, '  a b  ')
      expect(normalizeMaterialSource(once.kind, once.value)).toEqual(once)
    }
  })

  it('归一化不改变可提交性（C/D 的空值仍然不可提交）', () => {
    for (const kind of MATERIAL_SOURCE_KINDS) {
      for (const value of ['', '  ', 'v']) {
        const raw = source(kind, value)
        expect(materialSourceIsValid(normalizeMaterialSource(kind, value))).toBe(materialSourceIsValid(raw))
      }
    }
  })
})
