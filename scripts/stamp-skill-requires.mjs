#!/usr/bin/env node
// 给 `skills/*.yaml` 补上**供给面声明** `requires`，并把推导规则写死在这里（可复核）。
//
// # 为什么需要
// 生态策略（`artifacts/research/ecosystem-strategy-2026-10-29.md` §3-③）把
// 「包必须声明它要什么」定为上线门槛：`network` / `fs_write` / `credentials` / `shell`。
// 缺声明的技能不许进官方索引——否则用户装一个技能，无法预知它会不会联网、会不会写盘。
//
// # 诚实边界（重要）
// 声明是**从 `tools:` 推导**的，不是运行时动态观测得到的：
// - 只要有 `shell` 类工具就同时置 `shell` 与 `fs_write`（shell 能写盘，隐瞒就是骗人）；
// - `credentials` 一律 `false` —— 现有 23 个技能都用全局 provider 配置，不各自持有密钥；
//   将来若有技能真要独立密钥，必须**手工**改成 true，脚本不会替你猜。
// 推导规则与结果都写进文件注释，任何人可逐条核对。
//
// # 用法
//   node scripts/stamp-skill-requires.mjs           # 演算（打印将写入的声明）
//   node scripts/stamp-skill-requires.mjs --apply   # 写入（幂等：已有 requires 的文件跳过）

import { readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(HERE, '..')
const SKILL_DIR = join(ROOT, 'skills')
const APPLY = process.argv.includes('--apply')

const SHELL_TOOLS = new Set(['shell', 'shell_exec', 'terminal', 'run_command', 'bash', 'system_control'])
const WRITE_TOOLS = new Set(['write_file', 'edit_file', 'fs', 'str_replace_editor', 'apply_patch'])
const NET_TOOLS = new Set(['browser', 'web_search', 'http', 'fetch', 'download', 'media'])

function extractTools(text) {
  // 只认顶层 `tools:`（行内数组 或 紧随的 `- x` 列表）；技能文件的字段集稳定，
  // 不需要通用 YAML 解析器（同 `gen-market-index.mjs` 的取舍与理由）。
  const lines = text.split(/\r?\n/)
  const i = lines.findIndex(l => /^tools:/.test(l))
  if (i < 0) return []
  const inline = /^tools:\s*\[(.*)\]\s*$/.exec(lines[i])
  if (inline) {
    return inline[1].split(',').map(s => s.trim().replace(/^["']|["']$/g, '')).filter(Boolean)
  }
  const out = []
  for (let k = i + 1; k < lines.length; k += 1) {
    const line = lines[k]
    if (/^[A-Za-z_]/.test(line)) break          // 下一个顶层键 → 列表结束
    const m = /^\s*-\s+(.+)$/.exec(line)
    if (m) out.push(m[1].trim().replace(/^["']|["']$/g, ''))
  }
  return out
}

function derive(tools) {
  const shell = tools.some(t => SHELL_TOOLS.has(t))
  const fsWrite = shell || tools.some(t => WRITE_TOOLS.has(t))
  const network = tools.some(t => NET_TOOLS.has(t))
  return { network, fs_write: fsWrite, credentials: false, shell }
}

const files = readdirSync(SKILL_DIR).filter(f => f.endsWith('.yaml')).sort()
const plan = []
for (const file of files) {
  const abs = join(SKILL_DIR, file)
  const text = readFileSync(abs, 'utf8')
  if (/^requires:/m.test(text)) {
    plan.push({ file, skipped: '已有 requires' })
    continue
  }
  const tools = extractTools(text)
  const req = derive(tools)
  const block = [
    '',
    '# 供给面声明（2026-10-29 补）：安装前要展示「它要什么」。',
    '# 由 scripts/stamp-skill-requires.mjs 从上面的 tools 推导，规则见该脚本头部注释。',
    'requires:',
    `  network: ${req.network}`,
    `  fs_write: ${req.fs_write}`,
    `  credentials: ${req.credentials}`,
    `  shell: ${req.shell}`,
    '',
  ].join('\n')
  plan.push({ file, tools, requires: req, block })
}

const writing = plan.filter(p => !p.skipped)
for (const p of plan) {
  if (p.skipped) {
    process.stdout.write(`- ${p.file}: 跳过（${p.skipped}）\n`)
    continue
  }
  const flag = ['shell', 'fs_write', 'network'].filter(k => p.requires[k]).join(',') || '仅本地提示词'
  process.stdout.write(`- ${p.file}: requires={${flag}} ← tools=[${p.tools.join(',')}]\n`)
  if (APPLY) {
    const abs = join(SKILL_DIR, p.file)
    const text = readFileSync(abs, 'utf8')
    // 结尾统一补一个换行后再追加，避免把 `prompt_template: |` 的字面量块吃进来。
    const base = text.endsWith('\n') ? text : `${text}\n`
    writeFileSync(abs, `${base}${p.block}`, 'utf8')
  }
}

process.stdout.write(
  `[stamp-requires] ${writing.length} 份待写入 / ${plan.length} 份扫描`
  + `${APPLY ? '（已写入）' : '（演算，加 --apply 写入）'}\n`,
)
