import { describe, expect, it } from 'vitest'
import { ansiToHtml } from './ansi'

describe('ansiToHtml', () => {
  it('renders SGR colors as theme classes instead of raw escape codes', () => {
    const html = ansiToHtml('\u001b[32mok\u001b[0m done')
    expect(html).toContain('class="t-ansi-green"')
    expect(html).toContain('ok')
    expect(html).not.toContain('\u001b[')
    expect(html).toContain('done')
  })

  it('resets style on code 0', () => {
    const html = ansiToHtml('\u001b[31mred\u001b[0mplain')
    expect(html).toContain('t-ansi-red')
    // 重置后的文本不再包在颜色 span 里
    expect(html.endsWith('plain')).toBe(true)
  })

  it('escapes HTML so terminal output cannot inject markup', () => {
    const html = ansiToHtml('<img src=x onerror=alert(1)>')
    expect(html).not.toContain('<img')
    expect(html).toContain('&lt;img')
  })

  it('drops cursor-control sequences that make no sense in replay', () => {
    expect(ansiToHtml('a\u001b[2Kb')).toBe('ab')
  })
})
