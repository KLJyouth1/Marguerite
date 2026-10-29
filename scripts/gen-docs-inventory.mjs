#!/usr/bin/env node
// Markdown 文档清单与健康度审计（全仓）。
//
// 为什么不用 `find | wc -l`：本仓的 md 有 6 类来源，治理策略完全不同——
//   ① 发布权威文档（会随 clone 走，写错=对外错误）；② 技能文档（供给面，1600+ 份）；
//   ③ 第三方检出（**不是我们的文档**，只有检出不治理）；④ 内部工作记录（`artifacts/`、
//   `deliverables/`，被 .gitignore，不随发布走）；⑤ 构建产物（几万个，必须排除）；
//   ⑥ 归档（`_archive/`、`docs/legacy/`）。
// 混在一起统计只会得到"3 万份文档"这种无用结论。本脚本按 tier 分层，并给出四个
// 可行动的信号：**超预算文件 / 重名文件 / 断链 / 陈旧文件**。
//
// 输出：
//   artifacts/reports/docs-inventory.json   （机器可读，供后续批次消费）
//   artifacts/reports/docs-inventory.md     （人读报告）
//
// 用法：
//   node scripts/gen-docs-inventory.mjs            # 生成报告
//   node scripts/gen-docs-inventory.mjs --check    # 只做门禁判定（断链/超预算 → exit 1）
//   node scripts/gen-docs-inventory.mjs --quiet    # 只打印汇总一行

import { execFileSync } from 'node:child_process'
import { readdirSync, readFileSync, writeFileSync, mkdirSync, statSync, existsSync } from 'node:fs'
import { dirname, extname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(HERE, '..')
const args = process.argv.slice(2)
const CHECK = args.includes('--check')
const QUIET = args.includes('--quiet')
const REPORT_DIR = join(ROOT, 'artifacts', 'reports')

/** 不走目录遍历的目录（体量太大且不属于治理对象）。 */
const PRUNE = [
  'node_modules', 'target', 'dist', '.git',
  'marguerite_build_target.keep', '_archive', 'dsh-data',
  'third_party',                 // 单独按「检出条目数」统计，不逐个建索引
  // `.tmp/` 是**一次 MSI 解包残留**（`.tmp/msi_extract_0217/…` 是一整份 App 资源的拷贝）。
  // 首轮审计把它算进 `authority`，凭空造出 3200+ 份"发布文档"与 1000+ 条断链，
  // 把真实信号全淹了。它是本地残留、已被 .gitignore 忽略 → 排除，但在报告里单独点名。
  '.tmp', '.venv', 'venv', '.pytest_cache', '.mypy_cache', 'coverage',
]

/**
 * 按**相对路径**排除的目录：这些不是"文档"，而是构建镜像或工具私有目录。
 * 首轮审计把它们算进 `authority`，于是"发布文档 1981 份"里 1920 份是假的。
 */
const PRUNE_PATHS = [
  'gui/src-tauri/skills',      // 打包期的 skills 镜像（只在目标缺失时才拷，见 AGENTS §4）
  'gui/src-tauri/resources',   // 打包期的 resources 镜像（pyengine 等）
  'gui/src-tauri/target',
  'webui/dist.v1', 'webui/dist',
  '.workbuddy', '.workbuddy-ai', '.trae', '.dumate', 'temp',
]

/**
 * agent 指令类文件的预算（32 KiB）。**只对"会被自动注入上下文"的文件生效**：
 * `AGENTS.md` / `CLAUDE.md` 是每次会话都被塞进提示词的那种，超了就被截断；
 * `docs/agents/*.md` 是**按需 Read** 的分册，不受这个预算约束（但会作为信息项列出）。
 * 把两者混为一谈会得出"拆完还超标"的错误结论。
 */
const AGENT_DOC_BUDGET = 32 * 1024
const AGENT_DOCS = ['AGENTS.md', 'CLAUDE.md']

/** 分层：顺序即优先级，命中第一个即归类。 */
const TIERS = [
  {
    id: 'third_party',
    label: '第三方检出（不治理）',
    test: p => p.startsWith('third_party/') || p.startsWith('pyengine/'),
  },
  {
    id: 'build',
    label: '构建/归档产物',
    test: p => p.startsWith('build_') || p.includes('/build/') || p.startsWith('_archive/'),
  },
  {
    id: 'skills',
    label: '技能文档（供给面）',
    test: p => p.startsWith('skills/') || p.startsWith('.skills/'),
  },
  {
    id: 'internal',
    label: '内部工作记录（不随发布）',
    test: p => p.startsWith('artifacts/') || p.startsWith('deliverables/')
      || p.startsWith('next-gen-plan') || p.startsWith('docs/internal/')
      || p.startsWith('docs/plans/') || p.startsWith('docs/prd/') || p.startsWith('docs/decisions/'),
  },
  {
    id: 'legacy',
    label: '归档文档',
    test: p => p.startsWith('docs/legacy/') || p.startsWith('docs/archive/'),
  },
  {
    id: 'authority',
    label: '发布权威文档（会随 clone 走）',
    test: () => true,
  },
]

function gitList(argsArr) {
  try {
    return execFileSync('git', argsArr, { cwd: ROOT, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
      .split('\n')
      .map(s => s.trim())
      .filter(Boolean)
  } catch (e) {
    process.stderr.write(`[docs-inventory] git ${argsArr.join(' ')} 失败: ${e.message}\n`)
    return []
  }
}

function walk(dir, out) {
  let entries
  try {
    entries = readdirSync(dir, { withFileTypes: true })
  } catch (e) {
    if (e.code !== 'ENOENT') process.stderr.write(`[docs-inventory] 读目录失败 ${dir}: ${e.message}\n`)
    return
  }
  for (const entry of entries) {
    const abs = join(dir, entry.name)
    if (entry.isDirectory()) {
      if (PRUNE.includes(entry.name)) continue
      const rel = relative(ROOT, abs).split('\\').join('/')
      if (PRUNE_PATHS.some(p => rel === p || rel.startsWith(`${p}/`))) continue
      walk(abs, out)
    } else if (extname(entry.name).toLowerCase() === '.md') {
      out.push(relative(ROOT, abs).split('\\').join('/'))
    }
  }
}

function tierOf(p) {
  for (const t of TIERS) if (t.test(p)) return t
  return TIERS[TIERS.length - 1]
}

/** 统计 third_party / 被排除目录的规模（只报数，不建索引）。 */
function countPruned() {
  const stats = {}
  for (const name of PRUNE) {
    const abs = join(ROOT, name)
    if (!existsSync(abs)) continue
    let n = 0
    const stack = [abs]
    while (stack.length) {
      const d = stack.pop()
      let es
      try {
        es = readdirSync(d, { withFileTypes: true })
      } catch {
        continue
      }
      for (const e of es) {
        if (e.isDirectory()) stack.push(join(d, e.name))
        else if (extname(e.name).toLowerCase() === '.md') n += 1
      }
      if (n > 200000) break
    }
    stats[name] = n
  }
  return stats
}

/** 抽取 markdown 站内链接（相对路径），用于断链检查。 */
function extractLinks(text) {
  // 先剥掉**代码**：围栏块与行内 code span 里的 `[x](../y.md)` 是**示例**，不是链接。
  // 首轮审计没做这一步，把 AGENTS.md 里那段"相对链接会指到仓库外"的示例文字
  // 报成了 2 条断链 —— 门禁的假阳性比漏报更伤信任（会训练人忽略红灯）。
  const stripped = text
    .replace(/^```[\s\S]*?^```/gm, '')
    .replace(/`[^`\n]*`/g, '')
  const out = []
  const re = /\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g
  let m
  while ((m = re.exec(stripped))) {
    const target = m[1].trim()
    if (!target) continue
    // `file:///d:/…/foo.rs#L10-L20` 是作者本机绝对路径：它不是仓库内相对链接，
    // 但**可以校验**（文件是否存在）。首轮把它当相对路径解析，报了 4 条假断链。
    if (/^file:/i.test(target)) {
      out.push(target)
      continue
    }
    if (/^(https?:|mailto:|tel:|data:)/i.test(target)) continue
    if (target.startsWith('#')) continue
    out.push(target)
  }
  return out
}

const files = []
walk(ROOT, files)

const tracked = new Set(gitList(['ls-files', '*.md']).map(p => p.split('\\').join('/')))
// 未跟踪且未被忽略的 md（会被下一次 `git add -A` 带进提交 → 必须看见）
const untracked = new Set(gitList(['ls-files', '-o', '--exclude-standard', '*.md']).map(p => p.split('\\').join('/')))

const now = Date.now()
const DAY = 24 * 60 * 60 * 1000
const items = []
const brokenLinks = []
const oversized = []

for (const p of files.sort()) {
  const abs = join(ROOT, p)
  let st
  try {
    st = statSync(abs)
  } catch (e) {
    process.stderr.write(`[docs-inventory] stat 失败 ${p}: ${e.message}\n`)
    continue
  }
  const tier = tierOf(p)
  const ageDays = Math.floor((now - st.mtimeMs) / DAY)
  items.push({
    path: p,
    tier: tier.id,
    bytes: st.size,
    mtime: new Date(st.mtimeMs).toISOString().slice(0, 10),
    age_days: ageDays,
    tracked: tracked.has(p),
    would_commit: untracked.has(p),
  })

  // 只对「发布权威 + 内部记录」做门禁级检查：第三方/产出里有大量历史断链，管不了也不该管。
  if (tier.id === 'authority' || tier.id === 'internal') {
    if (st.size > AGENT_DOC_BUDGET) oversized.push({ path: p, bytes: st.size, tier: tier.id })
    let text = ''
    try {
      text = readFileSync(abs, 'utf8')
    } catch (e) {
      process.stderr.write(`[docs-inventory] 读取失败 ${p}: ${e.message}\n`)
      continue
    }
    for (const link of extractLinks(text)) {
      const target = link.split('#')[0]
      if (!target) continue
      const isFileUrl = /^file:/i.test(target)
      // `file:///d:/x` → `d:/x`（去掉协议与开头的斜杠）；其余保持原样。
      const bare = isFileUrl ? target.replace(/^file:\/\/\//i, '').replace(/^\/([a-zA-Z]:)/, '$1') : target
      const resolved = isFileUrl
        ? resolve(decodeURIComponent(bare))
        : resolve(dirname(abs), decodeURIComponent(target))
      if (!existsSync(resolved)) {
        // 断链分两类：**发布文档**的断链是对外错误，必须修（门禁失败）；
        // **内部历史记录**（artifacts/deliverables 里的旧报告）的断链照实保留——
        // 它们是当时的事实，改写历史比留一条死链更有害。只统计，不阻断。
        brokenLinks.push({ from: p, link, tier: tier.id })
      }
    }
  }
}

// 重名文件：同名不同路径会让 `rg "xxx.md"`、文档互相引用、以及新人找文档全部退化。
const byBase = new Map()
for (const it of items) {
  const base = it.path.split('/').pop()
  if (!byBase.has(base)) byBase.set(base, [])
  byBase.get(base).push(it)
}
const duplicates = [...byBase.entries()]
  .filter(([, list]) => list.length > 1)
  .map(([base, list]) => ({ base, count: list.length, paths: list.slice(0, 6).map(i => i.path) }))
  .sort((a, b) => b.count - a.count)

// agent 指令文件预算（本仓最大的那个问题的量化口径）
const agentDocs = AGENT_DOCS.map(p => {
  const abs = join(ROOT, p)
  if (!existsSync(abs)) return { path: p, exists: false, bytes: 0 }
  const st = statSync(abs)
  return { path: p, exists: true, bytes: st.size, over_budget: st.size > AGENT_DOC_BUDGET }
})
const agentDir = join(ROOT, 'docs', 'agents')
const agentParts = existsSync(agentDir)
  ? readdirSync(agentDir).filter(f => f.endsWith('.md')).map(f => {
    const st = statSync(join(agentDir, f))
    return { path: `docs/agents/${f}`, bytes: st.size }
  })
  : []

const byTier = {}
for (const it of items) {
  byTier[it.tier] ||= { id: it.tier, files: 0, bytes: 0, tracked: 0, would_commit: 0, stale_120d: 0 }
  const t = byTier[it.tier]
  t.files += 1
  t.bytes += it.bytes
  if (it.tracked) t.tracked += 1
  if (it.would_commit) t.would_commit += 1
  if (it.age_days > 120) t.stale_120d += 1
}

const pruned = countPruned()
const totalBytes = items.reduce((s, i) => s + i.bytes, 0)

const summary = {
  generated_at: new Date().toISOString(),
  generator: 'scripts/gen-docs-inventory.mjs',
  totals: {
    indexed_md: items.length,
    indexed_bytes: totalBytes,
    tracked: items.filter(i => i.tracked).length,
    would_commit: items.filter(i => i.would_commit).length,
    excluded: pruned,
  },
  tiers: Object.values(byTier).sort((a, b) => b.files - a.files),
  agent_docs: { budget_bytes: AGENT_DOC_BUDGET, entries: agentDocs, parts: agentParts },
  gates: {
    oversized: oversized.length,
    broken_links: brokenLinks.length,
    broken_links_published: brokenLinks.filter(b => b.tier === 'authority').length,
    broken_links_internal: brokenLinks.filter(b => b.tier !== 'authority').length,
    duplicate_basenames: duplicates.length,
  },
  oversized: oversized.slice(0, 50),
  broken_links: brokenLinks.slice(0, 80),
  broken_links_published: brokenLinks.filter(b => b.tier === 'authority'),
  duplicates: duplicates.slice(0, 30),
  items,
}

if (!QUIET) {
  mkdirSync(REPORT_DIR, { recursive: true })
  writeFileSync(join(REPORT_DIR, 'docs-inventory.json'), `${JSON.stringify(summary, null, 2)}\n`, 'utf8')

  const kb = n => `${(n / 1024).toFixed(0)}KB`
  const lines = []
  lines.push('# 文档清单与健康度审计（自动生成）', '')
  lines.push(`> 生成：${summary.generated_at} · 生成器：\`scripts/gen-docs-inventory.mjs\``)
  lines.push('> 本文件在 `artifacts/` 下（被 .gitignore 忽略，不随发布走）。', '')
  lines.push('## 总量', '')
  lines.push(`- 建索引的 md：**${items.length}** 份 / ${kb(totalBytes)}`)
  lines.push(`- 已被 git 跟踪：${summary.totals.tracked} · **未被跟踪但会被提交**：${summary.totals.would_commit}`)
  lines.push(`- 未建索引（体量大且非治理对象）：`
    + Object.entries(pruned).map(([k, v]) => `${k}=${v}`).join(' · '), '')
  lines.push('## 按分层', '')
  lines.push('| tier | 文件数 | 体积 | 已跟踪 | 会随提交走 | >120 天未动 |')
  lines.push('|---|---|---|---|---|---|')
  for (const t of summary.tiers) {
    lines.push(`| \`${t.id}\` | ${t.files} | ${kb(t.bytes)} | ${t.tracked} | ${t.would_commit} | ${t.stale_120d} |`)
  }
  lines.push('', '## agent 指令文件预算（32 KiB 硬上限）', '')
  for (const d of agentDocs) {
    lines.push(`- \`${d.path}\`：${d.exists ? `${kb(d.bytes)}${d.over_budget ? ' — **超预算**' : ''}` : '不存在'}`)
  }
  if (agentParts.length) {
    lines.push(`- \`docs/agents/\` 分片：${agentParts.length} 份 / ${kb(agentParts.reduce((s, p) => s + p.bytes, 0))}`)
  }
  lines.push('', '## 门禁信号', '')
  lines.push(`- 超 32KB 的文档：**${oversized.length}** 份`)
  lines.push(`- 断链：发布文档 **${summary.gates.broken_links_published}** 处（门禁项）`
    + ` · 内部历史记录 ${summary.gates.broken_links_internal} 处（只统计，不改写历史）`)
  lines.push(`- 重名文件（同名不同路径）：**${duplicates.length}** 组`)
  if (oversized.length) {
    lines.push('', '### 超预算 Top', '')
    for (const o of oversized.slice(0, 15)) lines.push(`- ${kb(o.bytes)} · \`${o.path}\` (${o.tier})`)
  }
  if (brokenLinks.length) {
    lines.push('', '### 断链样例（前 30）', '')
    for (const b of brokenLinks.slice(0, 30)) lines.push(`- \`${b.from}\` → \`${b.link}\``)
  }
  if (duplicates.length) {
    lines.push('', '### 重名样例（前 12）', '')
    for (const d of duplicates.slice(0, 12)) {
      lines.push(`- \`${d.base}\` ×${d.count}：${d.paths.map(p => `\`${p}\``).join('、')}`)
    }
  }
  writeFileSync(join(REPORT_DIR, 'docs-inventory.md'), `${lines.join('\n')}\n`, 'utf8')
}

process.stdout.write(
  `[docs-inventory] ${items.length} 份 / ${(totalBytes / 1024 / 1024).toFixed(1)}MB`
  + ` · 超预算 ${oversized.length}`
  + ` · 发布断链 ${brokenLinks.filter(b => b.tier === 'authority').length}`
  + ` · 内部断链 ${brokenLinks.filter(b => b.tier !== 'authority').length}`
  + ` · 重名 ${duplicates.length}`
  + ` · AGENTS.md ${(agentDocs[0].bytes / 1024).toFixed(0)}KB`
  + `${QUIET ? '' : ` → artifacts/reports/docs-inventory.{json,md}`}\n`,
)

if (CHECK) {
  const fails = []
  const publishedBroken = brokenLinks.filter(b => b.tier === 'authority')
  if (publishedBroken.length) fails.push(`发布文档断链 ${publishedBroken.length} 处`)
  const overAgent = agentDocs.filter(d => d.over_budget)
  if (overAgent.length) fails.push(`agent 指令文件超 32KiB：${overAgent.map(d => d.path).join(', ')}`)
  if (fails.length) {
    process.stderr.write(`[docs-inventory] CHECK FAILED — ${fails.join('；')}\n`)
    process.exit(1)
  }
  process.stdout.write('[docs-inventory] CHECK OK\n')
}
