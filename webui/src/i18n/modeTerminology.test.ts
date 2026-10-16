// @vitest-environment happy-dom
// D1 术语一致性守卫（R65）：发送区的 `modeShort.*` 短标必须与全局规范模式名
// (`mode.alias.*`) **同源一致**——例如 bloom 的短标必须是其规范别名「深度打磨」的子串。
//
// 背景：真机曾把 `modeShort.bloom` 写成音译「布隆」，与规范别名「深度打磨」脱钩；
// 而当时的 i18n 测试只校验**键集**（locale 间键互比）、不校验**值**，所以没有任何
// 守卫能发现。本用例锁定「短标 ⊂ 规范别名」这一不变量：以后改词只需让 alias/short
// 保持同根，不必再同步一个字面量断言（变异自检：把短标改回「布隆」→ 必红）。
import { describe, it, expect } from 'vitest'
import en from '@/locales/en.json'
import zhHans from '@/locales/zh-Hans.json'

function at(obj: unknown, path: string): unknown {
  return path
    .split('.')
    .reduce<unknown>(
      (acc, k) => (acc && typeof acc === 'object' ? (acc as Record<string, unknown>)[k] : undefined),
      obj,
    )
}

const LOCALES: Array<[string, unknown]> = [
  ['en', en],
  ['zh-Hans', zhHans],
]

// 采用同一词根的「别名 → 短标」对：short 必须是 alias 的子串。
// plan/agent 的短标（计划/默认）刻意走另一条命名轴（不是「安全探索/日常协作」的子串），
// 故不纳入此不变量；bloom/ensemble 同根，属本守卫范围。
const SUBSTRING_PAIRS = ['bloom', 'ensemble'] as const

describe('mode short labels stay consistent with canonical mode names (D1)', () => {
  for (const [locale, messages] of LOCALES) {
    for (const mode of SUBSTRING_PAIRS) {
      it(`${locale}: modeShort.${mode} 必须与规范别名同源`, () => {
        const alias = at(messages, `mode.alias.${mode}`)
        const short = at(messages, `chat.composer.modeShort.${mode}`)
        expect(typeof alias, `${locale} mode.alias.${mode} 缺失`).toBe('string')
        expect(typeof short, `${locale} chat.composer.modeShort.${mode} 缺失`).toBe('string')
        // 「深度打磨」必须包含「打磨」；一旦退化为与别名脱钩的音译（如「布隆」）即失败。
        expect(alias as string, `${locale} ${mode} 短标「${short}」与规范别名「${alias}」脱钩`).toContain(
          short as string,
        )
      })
    }
  }

  it('zh-Hans bloom 短标不是与规范别名脱钩的音译', () => {
    const alias = at(zhHans, 'mode.alias.bloom') as string
    const short = at(zhHans, 'chat.composer.modeShort.bloom') as string
    expect(short.length).toBeGreaterThan(0)
    expect(alias).toContain(short)
  })
})
