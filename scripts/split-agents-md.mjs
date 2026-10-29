#!/usr/bin/env node
// 把超长的 AGENTS.md 拆成「瘦入口 + 分册」，并**证明没有丢内容**。
//
// # 为什么必须拆（不是洁癖）
// AGENTS.md 实测 **159 KB**，而智能体指令预算约 32–64 KiB：超出部分在注入时被**截断**，
// 而最要紧的「完成前验证铁律」恰好在 §8（尾部）。文件自己在 §0 里也写着这件事——
// 也就是说：**我们明知会被截断，还把铁律放在会被切掉的位置**。
// 160 KB 里 §4「已知坑」独占 ~68 KB，是它把文件顶爆的。
//
// # 拆法（保守、可逆、可核）
// - 保留在入口：标题、§0 三条铁律、§1 仓库结构、§8 验证铁律、导航表、§映射表。
// - 分册：§2 / §3 / §4 / §5 / §6 / §7 → `docs/agents/NN-*.md`，内容**逐字节搬运**，
//   只在文件头加一行「原 AGENTS.md §N」。
// - 校验：把旧文件所有非空行建集合，逐行检查是否仍存在于「新入口 + 全部分册」，
//   **缺失必须为 0**；同时断言新入口 ≤ 32 KiB。
//
// # 用法
//   node scripts/split-agents-md.mjs            # 只演算 + 校验，不写盘（默认）
//   node scripts/split-agents-md.mjs --apply    # 真写盘（会备份旧文件到 artifacts/）
//
// ⚠️ 本仓禁用 `git checkout/restore` 回滚（工作区长期 1800+ 未提交改动）。
// 撤销只能用备份文件手工还原 —— 所以 `--apply` 先把旧文件整份复制到
// `artifacts/backup/AGENTS.md.pre-split-<时间戳>`，并在 stdout 打印路径。

import { createHash } from 'node:crypto'
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(HERE, '..')
const AGENTS = join(ROOT, 'AGENTS.md')
const PARTS_DIR = join(ROOT, 'docs', 'agents')
const APPLY = process.argv.includes('--apply')
const BUDGET = 32 * 1024

/** 分册计划：`section` 用于切分（按 `## N.` 标题匹配），`file` 是输出文件名。 */
const PARTS = [
  { section: '2', file: '02-build-and-test.md', title: '构建与测试（权威命令）' },
  { section: '3', file: '03-invariants.md', title: '架构不变量（违反即缺陷）' },
  { section: '4', file: '04-gotchas.md', title: '已知坑（gotchas）' },
  { section: '5', file: '05-contributing-workflow.md', title: '贡献 workflow' },
  { section: '6', file: '06-doc-cli-drift.md', title: '文档/代码 CLI 漂移校验（CI 门禁）' },
  { section: '7', file: '07-third-party.md', title: '第三方集成层（third_party/）' },
]
const KEEP_INLINE = new Set(['0', '1', '8'])

const original = readFileSync(AGENTS, 'utf8')
const originalBytes = statSync(AGENTS).size
const lines = original.split(/\r?\n/)

// ---- 1. 按 `## <n>.` 切段（保留标题行本身） -------------------------------------
const starts = []
lines.forEach((line, i) => {
  const m = /^## (\d+)\./.exec(line)
  if (m) starts.push({ section: m[1], index: i })
})
if (!starts.length) {
  process.stderr.write('[split-agents] 找不到 `## N.` 章节标题，放弃（不猜结构）\n')
  process.exit(2)
}
const preamble = lines.slice(0, starts[0].index)
const sections = new Map()
for (let k = 0; k < starts.length; k += 1) {
  const from = starts[k].index
  const to = k + 1 < starts.length ? starts[k + 1].index : lines.length
  sections.set(starts[k].section, lines.slice(from, to))
}

const missingSections = PARTS.filter(p => !sections.has(p.section)).map(p => p.section)
if (missingSections.length) {
  process.stderr.write(`[split-agents] 缺章节：§${missingSections.join(', ')} —— 停手，请先核对文件结构\n`)
  process.exit(2)
}

// ---- 2. 组装新入口 ---------------------------------------------------------------
const kept = [...KEEP_INLINE].filter(s => sections.has(s))
// 入口在仓库根，链接必须从根算起（`docs/agents/…`）。写成 `agents/…` 就是一条死链——
// 这正是本仓 §4 记过的「相对链接悄悄指到仓库外」同类坑。
const navRows = PARTS.map(p => `| §${p.section} | ${p.title} | [\`docs/agents/${p.file}\`](docs/agents/${p.file}) |`)
// 保留在入口的章节也要在表里可见，否则读者不知道"哪些还在本文件里"。
const keptRows = kept.map(s => {
  const first = (sections.get(s)[0] || '').replace(/^## /, '')
  return `| §${s} | ${first} | 本文件内（下方） |`
})
const keptBytes = kept.reduce((sum, s) => sum + Buffer.byteLength(sections.get(s).join('\n'), 'utf8'), 0)

const entry = [
  '# AGENTS.md — Marguerite 仓库指南（供 AI 智能体与贡献者）',
  '',
  '> Marguerite 是终端优先的智能体工作台：单 Rust 引擎 + 四入口（CLI / ratatui TUI / **Tauri 桌面端** / **Vue3 Web 控制台**）。',
  '> CLI 与 TUI 各自是独立 bin；桌面端是加载网关地址的 Tauri 壳；Web 端仅由回环网关（127.0.0.1）托管，**不开放公网**。',
  '> Python 引擎（OpenSquilla 系）以侧车方式共存，能力经统一网关触达。',
  '',
  '## 0. 先读这一段',
  '',
  '> **【2026-10-29 · 文档瘦身】** 本文件原为 **159 KB** 单文件，超过智能体指令预算后在注入时会被**截断**，',
  '> 而「完成前验证铁律」恰在 §8 —— 等于把最要紧的规矩放在会被切掉的位置。现已拆为「瘦入口 + `docs/agents/` 分册」：',
  '> 入口只保留铁律、仓库结构与验证章节；其余按 § 分册，**内容逐字节搬运、无删改**（拆分脚本自带行覆盖校验）。',
  '> 按需 `Read` 对应分册即可，不要再往本文件里堆细节。',
  '',
  '### 三条铁律（先记这三条，其余按需检索）',
  '',
  '1. **无新鲜验证证据，不得声称完成。** `bash scripts/verify.sh --clean --all` 全绿（每条 `[PASS]` + exit 0）才算数；',
  '   `[SKIP]` **永远不算通过**。详见 §8（已保留在本文件内）。',
  '2. **动手前先看 §2（构建/测试）与 §4（已知坑）。** §4 里每一条都是踩过的坑：GBK 乱码伪装成合法 UTF-8、',
  '   作者本机路径随代码发布、`sk-` 既是密钥前缀也是本仓 CSS 类名前缀……',
  '3. **工作区长期有未提交改动（用户明确要求不 commit）** → `git checkout/restore/stash` 一律禁止；',
  '   撤销只能用编辑法或 `git show HEAD:<path>` 手工比对。',
  '',
  '### 分册导航（原 §N ↔ 文件）',
  '',
  '| 原章节 | 内容 | 位置 |',
  '|---|---|---|',
  ...keptRows,
  ...navRows,
  '',
  '> 维护约定：**新增「坑」写进 §4 分册**，新增门禁/命令写进 §2 分册，架构不变量写进 §3 分册；',
  '> 入口只改导航与铁律。任何一条分册不应超过 32 KiB（超了继续按 `###` 再拆）。',
  '',
  ...kept.flatMap(s => [...sections.get(s), '']),
].join('\n')

// ---- 3. 组装分册（内容原样 + 一行来源说明） ----------------------------------------
const partFiles = []
for (const p of PARTS) {
  const body = sections.get(p.section).join('\n')
  const head = [
    `<!-- 本文件原为 AGENTS.md §${p.section}，2026-10-29 由 scripts/split-agents-md.mjs 逐字节搬运；`,
    '     内容未删改，请勿在此加"摘要"，细节就该留在分册里。 -->',
    '',
  ].join('\n')
  partFiles.push({ ...p, content: `${head}${body}\n`, bytes: Buffer.byteLength(`${head}${body}\n`, 'utf8') })
}

// ---- 4. 校验：行覆盖 + 体积 -------------------------------------------------------
const norm = s => s.trim()
const corpus = new Set()
for (const line of entry.split('\n')) corpus.add(norm(line))
for (const pf of partFiles) for (const line of pf.content.split('\n')) corpus.add(norm(line))
const lost = lines
  .map((l, i) => ({ l: norm(l), i: i + 1 }))
  .filter(x => x.l.length > 0 && !corpus.has(x.l))

const entryBytes = Buffer.byteLength(entry, 'utf8')
const partsBytes = partFiles.reduce((s, p) => s + p.bytes, 0)

const report = {
  original_bytes: originalBytes,
  entry_bytes: entryBytes,
  parts: partFiles.map(p => ({ file: p.file, bytes: p.bytes, section: p.section })),
  parts_bytes: partsBytes,
  total_bytes: entryBytes + partsBytes,
  kept_inline_sections: kept,
  lost_lines: lost.length,
  entry_within_budget: entryBytes <= BUDGET,
}
process.stdout.write(`${JSON.stringify(report, null, 2)}\n`)

if (lost.length) {
  process.stderr.write(`[split-agents] 内容覆盖校验失败：${lost.length} 行未落在任何输出文件中\n`)
  for (const x of lost.slice(0, 10)) process.stderr.write(`  L${x.i}: ${x.l.slice(0, 100)}\n`)
  process.exit(1)
}
if (entryBytes > BUDGET) {
  process.stderr.write(`[split-agents] 新入口 ${entryBytes}B > ${BUDGET}B 预算，拒绝写盘\n`)
  process.exit(1)
}

if (!APPLY) {
  process.stdout.write(
    `[split-agents] 演算通过：入口 ${(entryBytes / 1024).toFixed(1)}KB（预算 ${BUDGET / 1024}KB）`
    + ` · 分册 ${partFiles.length} 份 ${(partsBytes / 1024).toFixed(0)}KB · 行覆盖 100%\n`
    + '[split-agents] 未写盘。加 --apply 执行。\n',
  )
  process.exit(0)
}

// ---- 5. 写盘（先备份，绝不依赖 git 回滚） ------------------------------------------
const stamp = new Date().toISOString().replace(/[:.]/g, '-')
const backupDir = join(ROOT, 'artifacts', 'backup')
mkdirSync(backupDir, { recursive: true })
const backup = join(backupDir, `AGENTS.md.pre-split-${stamp}`)
copyFileSync(AGENTS, backup)
mkdirSync(PARTS_DIR, { recursive: true })
for (const pf of partFiles) writeFileSync(join(PARTS_DIR, pf.file), pf.content, 'utf8')
writeFileSync(AGENTS, entry, 'utf8')

const written = readdirSync(PARTS_DIR).filter(f => f.endsWith('.md'))
process.stdout.write(
  `[split-agents] 已写盘：AGENTS.md ${(entryBytes / 1024).toFixed(1)}KB`
  + ` · docs/agents/ ${written.length} 份 ${(partsBytes / 1024).toFixed(0)}KB\n`
  + `[split-agents] 备份：${relative(ROOT, backup)}（回滚请手工 copy 回来，勿用 git checkout）\n`,
)
