import { describe, expect, it } from 'vitest'

import en from '@/locales/en.json'

import {
  emptyMaterialSource,
  materialSourceErrorKey,
  normalizeMaterialSource,
  MATERIAL_SOURCE_KINDS,
  type MaterialSource,
} from './materialSource'
import {
  GENERATE_PARAM_FIELDS,
  GENERATE_TOOL_NAMES,
  MATERIAL_GEN_NAMESPACE,
  MATERIAL_SOURCE_NAMESPACE,
  argsPreview,
  buildToolArgs,
  fieldsForTool,
  invalidNumberField,
  launchBlocking,
  looksLikeImageRef,
  numberFieldIsValid,
  type GenerateTool,
  type ParamFieldDef,
} from './generateParams'

const KEYS = (tool: GenerateTool) => fieldsForTool(tool).map(f => f.key)
const field = (tool: GenerateTool, key: string): ParamFieldDef => {
  const found = fieldsForTool(tool).find(f => f.key === key)
  if (!found) throw new Error(`字段 ${key} 不在 ${tool} 的参数面里`)
  return found
}
const webSource = (value: string): MaterialSource => normalizeMaterialSource('web', value)

describe('generateParams — 参数面 = 后端 schema 的真实子集', () => {
  it('真实工具名对齐 Rust 注册表（不用 UI 别名）', () => {
    expect(GENERATE_TOOL_NAMES).toEqual({ image: 'image_generate', video: 'video_generate' })
  })

  it('图像参数面逐条等于 image_gen.rs:208-218 的 input_schema（7 项，不多不少）', () => {
    expect(KEYS('image').sort()).toEqual(
      ['negative_prompt', 'n', 'out', 'prompt', 'quality', 'size', 'style'].sort(),
    )
  })

  it('视频参数面 = video_gen.rs:578-595 的 input_schema 去掉刻意不暴露的轮询四件套（10 项）', () => {
    expect(KEYS('video').sort()).toEqual(
      ['aspect_ratio', 'duration', 'fps', 'image', 'negative_prompt', 'out', 'prompt', 'resolution', 'seed', 'size']
        .sort(),
    )
  })

  it('闭合世界：不暴露 response_format（在白名单但不在 schema）与异步轮询四件套', () => {
    const withheld = ['response_format', 'status_url', 'status_method', 'poll_interval_ms', 'poll_timeout_s']
    for (const tool of ['image', 'video'] as const) {
      for (const key of withheld) expect(KEYS(tool)).not.toContain(key)
    }
    // 反向自证：整个字段表里也不存在这些键，防止「只在某个工具漏掉」。
    for (const key of withheld) expect(GENERATE_PARAM_FIELDS.map(f => f.key)).not.toContain(key)
  })

  it('只有 prompt 是 required，且两个工具都有它', () => {
    const required = GENERATE_PARAM_FIELDS.filter(f => f.required).map(f => f.key)
    expect(required).toEqual(['prompt'])
    for (const tool of ['image', 'video'] as const) {
      expect(field(tool, 'prompt').required).toBe(true)
    }
  })

  it('每个字段都带文件:行取证、至少属于一个工具、且无重复键', () => {
    for (const def of GENERATE_PARAM_FIELDS) {
      expect(def.evidence).toMatch(/\.rs:\d/)
      expect(def.tools.length).toBeGreaterThan(0)
      expect(def.labelKey).toBeTruthy()
    }
    const keys = GENERATE_PARAM_FIELDS.map(f => f.key)
    expect(new Set(keys).size).toBe(keys.length)
  })

  it('fieldsForTool 返回副本且将 tools 收窄为单工具，不污染字段表', () => {
    const imagePrompt = fieldsForTool('image')[0]
    expect(imagePrompt).not.toBe(GENERATE_PARAM_FIELDS[0])
    expect(imagePrompt.tools).toEqual(['image'])
    expect(GENERATE_PARAM_FIELDS[0].tools).toEqual(['image', 'video'])
    expect(GENERATE_PARAM_FIELDS[0].unsigned).toBeUndefined()
  })

  it('n 声明为无符号（image_gen.rs:165 parse::<u32>），seed 不是（video_gen.rs:443 parse::<i64>）', () => {
    expect(field('image', 'n').unsigned).toBe(true)
    expect(field('video', 'seed').unsigned).toBeUndefined()
    expect(field('video', 'seed').type).toBe('integer')
  })
})

describe('generateParams — numberFieldIsValid 真值表', () => {
  const text = field('image', 'size')
  const int = field('image', 'n')
  const signedInt = field('video', 'seed')
  const decimal = field('video', 'duration')

  it('空串一律合法（= 不提交该参数，与 Rust filter(!is_empty) 同口径）', () => {
    for (const def of [text, int, signedInt, decimal]) {
      expect(numberFieldIsValid(def, '')).toBe(true)
      expect(numberFieldIsValid(def, '   ')).toBe(true)
      expect(numberFieldIsValid(def, null as unknown as string)).toBe(true)
    }
  })

  it('text 字段不受数值校验约束', () => {
    expect(numberFieldIsValid(text, '1024x1024')).toBe(true)
    expect(numberFieldIsValid(text, '完全不搭')).toBe(true)
  })

  it('非有限数一律非法（NaN / Infinity / 垃圾串）', () => {
    for (const def of [int, signedInt, decimal]) {
      expect(numberFieldIsValid(def, 'abc')).toBe(false)
      expect(numberFieldIsValid(def, 'NaN')).toBe(false)
      expect(numberFieldIsValid(def, 'Infinity')).toBe(false)
    }
  })

  it('integer：必须整数；无符号字段再拒绝负数，有符号字段放行负数', () => {
    expect(numberFieldIsValid(int, '2')).toBe(true)
    expect(numberFieldIsValid(int, '0')).toBe(true)
    expect(numberFieldIsValid(int, '1.5')).toBe(false)
    expect(numberFieldIsValid(int, '-1')).toBe(false) // u32：后端会静默回落 1 → 闸门拦下
    expect(numberFieldIsValid(signedInt, '-1')).toBe(true) // i64：负种子合法，不得误伤
    expect(numberFieldIsValid(signedInt, '42')).toBe(true)
    expect(numberFieldIsValid(signedInt, '4.2')).toBe(false)
  })

  it('number（duration/fps）只要求有限数，小数合法', () => {
    expect(numberFieldIsValid(decimal, '5')).toBe(true)
    expect(numberFieldIsValid(decimal, '4.5')).toBe(true)
    expect(numberFieldIsValid(decimal, '-1')).toBe(true)
    expect(numberFieldIsValid(decimal, 'x')).toBe(false)
  })
})

describe('generateParams — invalidNumberField 取第一个非法键', () => {
  it('全部合法 / 全空 → null', () => {
    expect(invalidNumberField('video', { prompt: 'p', duration: '5', fps: '24', seed: '-1' })).toBeNull()
    expect(invalidNumberField('video', {})).toBeNull()
  })

  it('按参数面声明顺序报告（duration 先于 fps 先于 seed）', () => {
    expect(invalidNumberField('video', { duration: 'abc', fps: 'xyz', seed: '1.5' })).toBe('duration')
    expect(invalidNumberField('video', { fps: 'xyz', seed: '1.5' })).toBe('fps')
    expect(invalidNumberField('video', { seed: '1.5' })).toBe('seed')
  })

  it('只查本工具的字段：图像的非法 n 不影响视频，反之亦然', () => {
    expect(invalidNumberField('video', { n: 'abc' })).toBeNull()
    expect(invalidNumberField('image', { n: 'abc' })).toBe('n')
    expect(invalidNumberField('image', { seed: 'abc' })).toBeNull()
  })

  it('负数：n 报错、seed 放行（修前两者同口径，负种子会被闸门误伤）', () => {
    expect(invalidNumberField('image', { prompt: 'p', n: '-1' })).toBe('n')
    expect(invalidNumberField('video', { prompt: 'p', seed: '-1' })).toBeNull()
  })
})

describe('generateParams — looksLikeImageRef（视频 image 只认 URL / data URI）', () => {
  it('接受 http/https/data:，大小写不敏感，允许首尾空白', () => {
    for (const ok of ['https://x/a.png', 'http://x/a.png', 'HTTPS://X', 'data:image/png;base64,AAA', '  https://x  ']) {
      expect(looksLikeImageRef(ok)).toBe(true)
    }
  })

  it('拒绝本机路径/相对路径/其它协议/空串/未锚定的前后缀', () => {
    for (const bad of ['', '   ', 'D:\\assets\\a.png', './a.png', 'ftp://x/a.png', 'https', 'xhttps://y', 'a https://y']) {
      expect(looksLikeImageRef(bad)).toBe(false)
    }
  })
})

describe('generateParams — buildToolArgs（只发后端真吃的键）', () => {
  it('trim 一切、丢弃空值、丢弃 schema 之外的键', () => {
    const args = buildToolArgs('image', { prompt: '  猫  ', size: '  ', n: ' 2 ', response_format: 'b64_json' }, emptyMaterialSource())
    expect(args).toEqual({ prompt: '猫', n: '2' })
    for (const v of Object.values(args)) expect(typeof v).toBe('string')
  })

  it('空面板 + 默认素材源 → 空参数表', () => {
    expect(buildToolArgs('image', {}, emptyMaterialSource())).toEqual({})
    expect(buildToolArgs('video', {}, emptyMaterialSource())).toEqual({})
  })

  it('A 网络 + 视频 + 值像 URL + 未手填首帧 → 写入 image', () => {
    expect(buildToolArgs('video', { prompt: 'p' }, webSource('  https://cdn/x.png  '))).toEqual({
      prompt: 'p',
      image: 'https://cdn/x.png',
    })
  })

  it('A 网络但手填了首帧 → 手填优先（显式输入赢过来源推断）', () => {
    const args = buildToolArgs('video', { prompt: 'p', image: 'data:image/png;base64,AAA' }, webSource('https://cdn/x.png'))
    expect(args.image).toBe('data:image/png;base64,AAA')
  })

  it('A 网络但值不像 URL → 不写 image（不把本机路径冒充首帧）', () => {
    expect(buildToolArgs('video', { prompt: 'p' }, webSource('D:/assets/a.png'))).toEqual({ prompt: 'p' })
  })

  it('B/C/D → 不写入任何后端参数（造键即造假控件）', () => {
    const cases: MaterialSource[] = [
      emptyMaterialSource(),
      normalizeMaterialSource('directory', 'D:/assets/poster'),
      normalizeMaterialSource('custom', '用户上传的草图'),
    ]
    for (const source of cases) {
      for (const tool of ['image', 'video'] as const) {
        expect(buildToolArgs(tool, { prompt: 'p' }, source)).toEqual({ prompt: 'p' })
      }
    }
  })

  it('图像工具永不因素材源多出 image 键（image 不是 image_generate 的参数）', () => {
    expect(buildToolArgs('image', { prompt: 'p' }, webSource('https://cdn/x.png'))).toEqual({ prompt: 'p' })
  })
})

describe('generateParams — launchBlocking 闸门真值表', () => {
  it('prompt 缺失最优先（即使素材源也非法）', () => {
    expect(launchBlocking('image', {}, normalizeMaterialSource('directory', ''))).toBe('needPrompt')
    expect(launchBlocking('image', { prompt: '   ' }, emptyMaterialSource())).toBe('needPrompt')
  })

  it('其次非法数值，最后才是素材源 C/D 未填', () => {
    expect(launchBlocking('video', { prompt: 'p', fps: 'x' }, normalizeMaterialSource('directory', ''))).toBe('invalidNumber')
    expect(launchBlocking('image', { prompt: 'p' }, normalizeMaterialSource('directory', ''))).toBe('directoryRequired')
    expect(launchBlocking('image', { prompt: 'p' }, normalizeMaterialSource('custom', '  '))).toBe('customRequired')
  })

  it('A/B 无附加值也可提交；C/D 填了即可提交', () => {
    expect(launchBlocking('image', { prompt: 'p' }, normalizeMaterialSource('web', ''))).toBeNull()
    expect(launchBlocking('video', { prompt: 'p' }, emptyMaterialSource())).toBeNull()
    expect(launchBlocking('image', { prompt: 'p' }, normalizeMaterialSource('directory', 'D:/a'))).toBeNull()
    expect(launchBlocking('image', { prompt: 'p' }, normalizeMaterialSource('custom', '草图'))).toBeNull()
  })

  it('合法负种子不拦（修前这里返回 invalidNumber）', () => {
    expect(launchBlocking('video', { prompt: 'p', seed: '-1' }, emptyMaterialSource())).toBeNull()
    expect(launchBlocking('image', { prompt: 'p', n: '2' }, emptyMaterialSource())).toBeNull()
  })

  it('闸门与 buildToolArgs 自洽：放行的表单必然带上 prompt', () => {
    const cases: Array<[GenerateTool, Record<string, string>, MaterialSource]> = [
      ['image', { prompt: 'p', n: '2' }, emptyMaterialSource()],
      ['video', { prompt: 'p', seed: '-1' }, webSource('https://cdn/x.png')],
      ['video', { prompt: 'p' }, normalizeMaterialSource('custom', '草图')],
    ]
    for (const [tool, values, source] of cases) {
      expect(launchBlocking(tool, values, source)).toBeNull()
      expect(buildToolArgs(tool, values, source).prompt).toBe('p')
    }
  })
})

describe('generateParams — argsPreview', () => {
  it('排除 prompt（由消息模板单独引用），其余按 k="v" 以分号拼接', () => {
    expect(argsPreview({ prompt: '一只猫', size: '1024x1024', n: '2' })).toBe('size="1024x1024"; n="2"')
  })

  it('空参数表 → 空串（不渲染「预览：」空壳）', () => {
    expect(argsPreview({})).toBe('')
    expect(argsPreview({ prompt: '只有提示词' })).toBe('')
  })

  it('保持插入顺序，值里的引号原样呈现（预览不是解析器）', () => {
    expect(argsPreview({ a: '1', b: '2', c: '3' })).toBe('a="1"; b="2"; c="3"')
  })
})

describe('generateParams — 文案键与字典对账（check-i18n 只看静态字面量，这里补动态键）', () => {
  const ns = (name: string): Record<string, unknown> => {
    const settings = (en as { settings?: Record<string, unknown> }).settings ?? {}
    return (settings[name] ?? {}) as Record<string, unknown>
  }
  const gen = ns(MATERIAL_GEN_NAMESPACE)
  const material = ns(MATERIAL_SOURCE_NAMESPACE)

  it('每个字段的 labelKey / placeholderKey 都在 en 字典里存在且非空', () => {
    for (const def of GENERATE_PARAM_FIELDS) {
      expect(typeof gen[def.labelKey], `settings.${MATERIAL_GEN_NAMESPACE}.${def.labelKey}`).toBe('string')
      expect(String(gen[def.labelKey]).trim()).not.toBe('')
      if (def.placeholderKey) {
        expect(typeof gen[def.placeholderKey], `settings.${MATERIAL_GEN_NAMESPACE}.${def.placeholderKey}`).toBe('string')
        expect(String(gen[def.placeholderKey]).trim()).not.toBe('')
      }
    }
  })

  it('materialSourceErrorKey 的两个后缀都有文案', () => {
    // 由真实 API 推导（不是硬编码清单）：四个选项在「未填值」时实际报出的键。
    const suffixes = MATERIAL_SOURCE_KINDS
      .map(kind => materialSourceErrorKey({ kind, value: '' }))
      .filter((key): key is NonNullable<typeof key> => key !== null)
      .sort()
    expect(suffixes).toEqual(['customRequired', 'directoryRequired'])
    for (const suffix of suffixes) {
      expect(typeof material[suffix], `settings.${MATERIAL_SOURCE_NAMESPACE}.${suffix}`).toBe('string')
      expect(String(material[suffix]).trim()).not.toBe('')
    }
  })
})
