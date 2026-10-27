/**
 * 编辑器语法高亮（highlight.js 按需注册语言）。
 *
 * 只注册用得到的那一小撮语言：全量 `highlight.js`（~190 门语言）会把工作台首屏
 * 拖进几百 KB，而这里的目标只是「看清结构」。未注册的扩展名一律走**转义纯文本**，
 * 不假装高亮。
 */
import hljs from 'highlight.js/lib/core'
import type { LanguageFn } from 'highlight.js'
import bash from 'highlight.js/lib/languages/bash'
import css from 'highlight.js/lib/languages/css'
import go from 'highlight.js/lib/languages/go'
import java from 'highlight.js/lib/languages/java'
import javascript from 'highlight.js/lib/languages/javascript'
import json from 'highlight.js/lib/languages/json'
import markdown from 'highlight.js/lib/languages/markdown'
import python from 'highlight.js/lib/languages/python'
import rust from 'highlight.js/lib/languages/rust'
import sql from 'highlight.js/lib/languages/sql'
import typescript from 'highlight.js/lib/languages/typescript'
import xml from 'highlight.js/lib/languages/xml'
import yaml from 'highlight.js/lib/languages/yaml'

const REGISTERED: Record<string, LanguageFn> = {
  bash,
  css,
  go,
  java,
  javascript,
  json,
  markdown,
  python,
  rust,
  sql,
  typescript,
  xml,
  yaml,
}

let registered = false
function ensureRegistered(): void {
  if (registered) return
  for (const [name, language] of Object.entries(REGISTERED)) {
    hljs.registerLanguage(name, language)
  }
  registered = true
}

/** 扩展名 → highlight.js 语言名。`vue` 走 xml（模板 + script 结构足够辨认）。 */
const LANG_BY_EXT: Record<string, string> = {
  ts: 'typescript',
  tsx: 'typescript',
  mts: 'typescript',
  cts: 'typescript',
  js: 'javascript',
  jsx: 'javascript',
  mjs: 'javascript',
  cjs: 'javascript',
  vue: 'xml',
  html: 'xml',
  htm: 'xml',
  svg: 'xml',
  xml: 'xml',
  json: 'json',
  jsonc: 'json',
  css: 'css',
  scss: 'css',
  less: 'css',
  py: 'python',
  rs: 'rust',
  go: 'go',
  java: 'java',
  kt: 'java',
  sh: 'bash',
  bash: 'bash',
  zsh: 'bash',
  ps1: 'bash',
  yml: 'yaml',
  yaml: 'yaml',
  md: 'markdown',
  markdown: 'markdown',
  sql: 'sql',
  toml: '', // 未注册：转义输出，不做假高亮
}

export function languageForPath(path: string): string {
  const name = path.split('/').pop() || ''
  const ext = name.includes('.') ? name.split('.').pop()?.toLowerCase() || '' : ''
  return LANG_BY_EXT[ext] || ''
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
}

/**
 * 生成用于 `v-html` 的高亮 HTML。
 * 无语言映射、或高亮抛错时返回**转义后的原文**（安全且不丢内容）。
 */
export function highlightCodeForPath(code: string, path: string): string {
  const language = languageForPath(path)
  if (!language) return escapeHtml(code)
  ensureRegistered()
  try {
    return hljs.highlight(code, { language, ignoreIllegals: true }).value
  } catch {
    return escapeHtml(code)
  }
}
