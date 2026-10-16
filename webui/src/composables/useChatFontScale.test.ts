import { describe, expect, it } from 'vitest'
import {
  CHAT_FONT_SCALES,
  CHAT_FONT_SCALE_PX,
  CHAT_FONT_SCALE_STORAGE_KEY,
  parseChatFontScale,
} from './useChatFontScale'

describe('parseChatFontScale', () => {
  it('keeps every declared scale', () => {
    for (const scale of CHAT_FONT_SCALES) {
      expect(parseChatFontScale(scale)).toBe(scale)
    }
  })

  it('falls back to the default for unknown or missing values', () => {
    // 设置项引入前没有这个 key：必须回落到 default，保持既有观感而不是报错或乱缩放。
    for (const bad of [undefined, null, '', 'huge', 16, {}, []]) {
      expect(parseChatFontScale(bad)).toBe('default')
    }
  })

  it('orders scales monotonically so the picker reads left-to-right', () => {
    const px = CHAT_FONT_SCALES.map(scale => CHAT_FONT_SCALE_PX[scale])
    for (let i = 1; i < px.length; i++) {
      expect(px[i]).toBeGreaterThan(px[i - 1])
    }
  })

  it('keeps default at the design root size (16px)', () => {
    // foundation.css 以 1rem = 16px 为基准；default 必须等于它，否则"标准"一档会整体偏移。
    expect(CHAT_FONT_SCALE_PX.default).toBe(16)
  })

  it('uses a versioned storage key so a future shape change can fall back safely', () => {
    expect(CHAT_FONT_SCALE_STORAGE_KEY).toMatch(/\.v\d+$/)
  })
})
