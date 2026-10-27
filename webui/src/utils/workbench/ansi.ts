/**
 * 极简 ANSI SGR 解析（只处理颜色 / 加粗 / 重置）。
 *
 * 为什么需要：沙箱终端把子进程的原始输出直接给我们，真实命令（git、cargo、
 * esbuild…）都会输出 `\x1b[31m` 这类颜色码。直接塞进 `<pre>` 用户看到的是
 * 一堆 `[32m` 噪声，等于「终端做得差」。
 *
 * 安全：先把文本按 HTML 实体转义，再拼接我们自己的 `<span>`，不做任何
 * 反向信任——输出内容永远只能是文本。
 */

const SGR = /\u001b\[([0-9;]*)m/g

/** 主题色板：只用 CSS 变量，跟随皮肤切换。 */
const FG_CLASSES: Record<number, string> = {
  30: 't-ansi-black',
  31: 't-ansi-red',
  32: 't-ansi-green',
  33: 't-ansi-yellow',
  34: 't-ansi-blue',
  35: 't-ansi-magenta',
  36: 't-ansi-cyan',
  37: 't-ansi-white',
  90: 't-ansi-bright-black',
  91: 't-ansi-bright-red',
  92: 't-ansi-bright-green',
  93: 't-ansi-bright-yellow',
  94: 't-ansi-bright-blue',
  95: 't-ansi-bright-magenta',
  96: 't-ansi-bright-cyan',
  97: 't-ansi-bright-white',
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
}

interface AnsiState {
  fg: string | null
  bold: boolean
  dim: boolean
}

function applyCodes(state: AnsiState, codes: number[]): void {
  for (const code of codes) {
    if (code === 0) {
      state.fg = null
      state.bold = false
      state.dim = false
    } else if (code === 1) {
      state.bold = true
    } else if (code === 2) {
      state.dim = true
    } else if (code === 22) {
      state.bold = false
      state.dim = false
    } else if (code === 39) {
      state.fg = null
    } else if (FG_CLASSES[code]) {
      state.fg = FG_CLASSES[code]
    }
    // 其它 SGR（背景色 / 下划线 / 光标控制）忽略：解析错色不如不解析。
  }
}

function wrap(text: string, state: AnsiState): string {
  if (!text) return ''
  const classes: string[] = []
  if (state.fg) classes.push(state.fg)
  if (state.bold) classes.push('t-ansi-bold')
  if (state.dim) classes.push('t-ansi-dim')
  const escaped = escapeHtml(text)
  return classes.length ? `<span class="${classes.join(' ')}">${escaped}</span>` : escaped
}

/** 把带 ANSI 的终端输出转成安全的 HTML。 */
export function ansiToHtml(raw: string): string {
  if (!raw) return ''
  const state: AnsiState = { fg: null, bold: false, dim: false }
  let out = ''
  let last = 0
  for (const match of raw.matchAll(SGR)) {
    const index = match.index ?? 0
    out += wrap(raw.slice(last, index), state)
    const codes = match[1]
      .split(';')
      .map(part => Number.parseInt(part, 10))
      .filter(code => Number.isFinite(code))
    applyCodes(state, codes.length ? codes : [0])
    last = index + match[0].length
  }
  out += wrap(raw.slice(last), state)
  // 去掉其余控制序列（光标移动、清行等），它们对纯文本回放没有意义。
  return out.replace(/\u001b\[[0-9;?]*[A-Za-z]/g, '')
}
