#!/usr/bin/env node
// 生成 Marguerite 官方能力索引（market index）。
//
// 为什么需要它：`market.remote.search`（core/src/gateway/market_remote.rs）已经能读
// **任意**索引 URL——`{plugins:[…]}` / `{items:[…]}` / 裸数组三种形状都认。也就是说
// 「我们自己的生态入口」缺的不是协议，而是**那份索引文件本身**，以及从仓库真源
// （`plugins/manifests/*.plugin.json` + `skills/*.yaml`）自动产出它的流水线。
//
// 设计约束（有意为之）：
// 1. **零依赖**：只用 node 内置模块。索引生成要能在 CI 与用户本机跑，不该为它引包。
// 2. **只读真源**：仓库里的清单/技能是唯一事实源，本脚本不缓存、不写仓库内文件
//    （默认输出到 dist/，可 `--out` 覆盖）。
// 3. **不编造字段**：包里没有的字段就不写。`trust` 一律先给 `unverified`，
//    真实分级由 `market.audit` 的本地复算给出（见策略文档 §2-L3），不在这里假装。
// 4. 解析失败**不静默跳过**：逐条 `warn` 到 stderr 并计入 `skipped[]`，退出码非 0。
//    （项目纪律：目录遍历的静默减计数是本仓反复踩过的坑。）
//
// 用法：
//   node scripts/gen-market-index.mjs                       # 输出 dist/market/index.json
//   node scripts/gen-market-index.mjs --out x.json --base-url https://raw.githubusercontent.com/o/r/main
//   node scripts/gen-market-index.mjs --check                # 只校验不写盘（CI 用）
//
// # 与 dsh / 第三方生态「打通」（2026-10-29 增补）
//
// 事实先行（读的是磁盘上的真实 dsh 安装，不是猜）：
// - dsh 插件是 **npm 包 + `package.json` 里的 `dsh` 字段**，`dsh.bundle.patch` 指向一份
//   `cordis.patch.yml`（`- insert: [{id, name, config}]` 行），运行在 **Cordis（TS）**里；
// - 我们的插件是**声明式 `*.plugin.json`**（没有 `entry`，不可动态加载），能力以原生 RPC/UI 落地；
// - 因此「打通」只能是**能力层**的：我们能做到 发现 / 索引 / 审计 / 适配映射，
//   **做不到** 直接执行 dsh 的 TS 插件。索引里对每条 dsh 条目如实写
//   `installable:false` + `runtime:"cordis-ts"`，绝不假装能装。
//
// 三种来源合并进同一份索引（每条带 `source`）：
//   marguerite（我们的清单/技能）· dsh（本地检出或 node_modules 里的 dsh 包）· external（第三方索引）
//
//   node scripts/gen-market-index.mjs --dsh-root dsh-data/profiles/node_modules
//   node scripts/gen-market-index.mjs --external-index ./their-index.json

import { createHash } from 'node:crypto'
import { readdirSync, readFileSync, mkdirSync, writeFileSync, existsSync } from 'node:fs'
import { dirname, join, resolve, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(HERE, '..')

const args = process.argv.slice(2)
function argValue(flag, fallback) {
  const i = args.indexOf(flag)
  return i >= 0 && args[i + 1] ? args[i + 1] : fallback
}
/** 可重复参数（`--dsh-root a --dsh-root b`）。 */
function argValues(flag) {
  const out = []
  for (let i = 0; i < args.length; i += 1) {
    if (args[i] === flag && args[i + 1]) out.push(args[i + 1])
  }
  return out
}
const OUT = resolve(ROOT, argValue('--out', 'dist/market/index.json'))
const BASE_URL = argValue('--base-url', '').replace(/\/+$/, '')
const CHECK_ONLY = args.includes('--check')
// dsh 生态来源：显式 `--dsh-root`，再叠加仓库内的既有约定目录
// （`third_party/external` 已被 `market.sources` 用作「已检出仓库」根）。
const DSH_ROOTS = [
  ...argValues('--dsh-root').map(p => resolve(ROOT, p)),
  join(ROOT, 'third_party', 'external'),
]
const EXTERNAL_INDEXES = argValues('--external-index').map(p => resolve(ROOT, p))

const SKILL_DIR = join(ROOT, 'skills')
const MANIFEST_DIR = join(ROOT, 'plugins', 'manifests')

const skipped = []
function warn(msg) {
  skipped.push(msg)
  process.stderr.write(`[market-index] WARN ${msg}\n`)
}

/**
 * 极简 YAML 取值：只认技能文件实际用到的顶层标量与一层字符串列表
 * （`name` / `description` / `version` / `author` / `intent: [a, b]` 或 `- a`）。
 * 刻意不引 yaml 包——技能文件是**我们自己的**格式，字段集稳定且有护栏
 * （`webui` 侧 `check-i18n` 与技能加载器都会先失败），这里不需要通用解析器。
 */
function parseSkillYaml(text) {
  const out = {}
  const lines = text.split(/\r?\n/)
  let currentList = null
  let currentMap = null
  for (const raw of lines) {
    const line = raw.replace(/\s+$/, '')
    if (!line || line.startsWith('#')) continue
    const top = /^([A-Za-z_][\w-]*):\s*(.*)$/.exec(line)
    if (top) {
      const [, key, rest] = top
      const value = rest.trim()
      if (value === '') {
        // 空值：可能是列表（后面跟 `- x`），也可能是嵌套映射（后面跟 `k: v`）。
        // 两种都先记成"待定容器"，遇到第一行再定型——`requires:` 正是后者。
        currentList = key
        currentMap = key
        out[key] = []
      } else if (value.startsWith('[') && value.endsWith(']')) {
        out[key] = value
          .slice(1, -1)
          .split(',')
          .map(s => s.trim().replace(/^["']|["']$/g, ''))
          .filter(Boolean)
        currentList = null
        currentMap = null
      } else {
        out[key] = value.replace(/^["']|["']$/g, '')
        currentList = null
        currentMap = null
      }
      continue
    }
    const item = /^\s*-\s+(.*)$/.exec(line)
    if (item && currentList) {
      if (currentMap && !Array.isArray(out[currentMap])) out[currentMap] = []
      if (!Array.isArray(out[currentList])) out[currentList] = []
      out[currentList].push(item[1].trim().replace(/^["']|["']$/g, ''))
      currentMap = null
      continue
    }
    const pair = /^\s+([A-Za-z_][\w-]*):\s*(.+)$/.exec(line)
    if (pair && currentMap) {
      // 缩进的 `k: v` → 把该键从"列表占位"升级成映射。
      // ⚠️ 只在它**还是数组**时升级：写成 `!Array.isArray(x) || x.length === 0`
      // 会让对象每行都被重置成 `{}`，最后只剩最后一个键（真踩过：`requires` 只剩
      // `{shell:true}`，而 network/fs_write/credentials 全丢了 —— 而且 check 依然"通过"）。
      if (Array.isArray(out[currentMap])) out[currentMap] = {}
      const v = pair[2].trim().replace(/^["']|["']$/g, '')
      out[currentMap][pair[1]] = v === 'true' ? true : v === 'false' ? false : v
      currentList = null
    }
  }
  return out
}

function sha256(buf) {
  return createHash('sha256').update(buf).digest('hex')
}

function urlFor(relPath) {
  if (!BASE_URL) return null
  return `${BASE_URL}/${relPath.split('\\').join('/')}`
}

function collectSkills() {
  if (!existsSync(SKILL_DIR)) {
    warn(`技能目录不存在：${relative(ROOT, SKILL_DIR)}`)
    return []
  }
  const files = readdirSync(SKILL_DIR, { withFileTypes: true })
    .filter(d => d.isFile() && d.name.endsWith('.yaml'))
    .map(d => d.name)
    .sort()

  const items = []
  for (const file of files) {
    const abs = join(SKILL_DIR, file)
    let text
    try {
      text = readFileSync(abs, 'utf8')
    } catch (e) {
      warn(`读技能失败 ${file}: ${e.message}`)
      continue
    }
    const meta = parseSkillYaml(text)
    const name = String(meta.name || '').trim()
    if (!name) {
      warn(`技能缺少 name，跳过：${file}`)
      continue
    }
    // 文件名与 name 必须一致：`dsh-router` 曾因文件名≠name 让 install/upgrade 真出错
    // （见 AGENTS.md 插件底座事实）。这里是同一类坑的前置拦截。
    if (`${name}.yaml` !== file) {
      warn(`技能 name(${name}) 与文件名(${file}) 不一致，跳过`)
      continue
    }
    const rel = relative(ROOT, abs)
    items.push({
      kind: 'skill',
      source: 'marguerite',
      name,
      version: String(meta.version || '0.0.0'),
      description: String(meta.description || ''),
      author: String(meta.author || ''),
      intent: Array.isArray(meta.intent) ? meta.intent : [],
      tools: Array.isArray(meta.tools) ? meta.tools : [],
      // 供给面声明：生态策略 §3-③ 要求「包必须声明它要什么」，缺声明不许进官方索引。
      // `requires` 由 `scripts/stamp-skill-requires.mjs` 从 `tools:` 推导后写进 YAML；
      // 这里只如实读，**缺失就写 null 并置 missing_requires**，绝不含糊成"安全"。
      requires: (meta.requires && typeof meta.requires === 'object' && !Array.isArray(meta.requires))
        ? meta.requires
        : null,
      missing_requires: !(meta.requires && typeof meta.requires === 'object' && !Array.isArray(meta.requires)),
      trust: 'unverified',
      // 技能是可落盘的纯声明（YAML），能被本工作台直接加载。
      installable: true,
      runtime: 'marguerite-skill-yaml',
      sha256: sha256(readFileSync(abs)),
      url: urlFor(rel),
      path: rel,
    })
  }
  return items
}

function collectPlugins() {
  if (!existsSync(MANIFEST_DIR)) {
    warn(`插件清单目录不存在：${relative(ROOT, MANIFEST_DIR)}`)
    return []
  }
  const files = readdirSync(MANIFEST_DIR, { withFileTypes: true })
    .filter(d => d.isFile() && d.name.endsWith('.plugin.json'))
    .map(d => d.name)
    .sort()

  const items = []
  for (const file of files) {
    const abs = join(MANIFEST_DIR, file)
    let raw
    try {
      raw = readFileSync(abs)
    } catch (e) {
      warn(`读清单失败 ${file}: ${e.message}`)
      continue
    }
    let manifest
    try {
      manifest = JSON.parse(raw.toString('utf8'))
    } catch (e) {
      warn(`清单 JSON 解析失败 ${file}: ${e.message}`)
      continue
    }
    const name = String(manifest.name || '').trim()
    if (!name) {
      warn(`清单缺少 name，跳过：${file}`)
      continue
    }
    if (`${name}.plugin.json` !== file) {
      warn(`清单 name(${name}) 与文件名(${file}) 不一致，跳过`)
      continue
    }
    const rel = relative(ROOT, abs)
    items.push({
      kind: 'plugin',
      source: 'marguerite',
      name,
      version: String(manifest.version || '0.0.0'),
      description: String(manifest.description || ''),
      author: String(manifest.author || ''),
      capabilities: Array.isArray(manifest.capabilities) ? manifest.capabilities : [],
      slash_commands: Array.isArray(manifest.slash_commands)
        ? manifest.slash_commands.map(c => String(c?.name || '')).filter(Boolean)
        : [],
      dependencies: Array.isArray(manifest.dependencies) ? manifest.dependencies : [],
      // 清单出现 system_prompt / full_context 即视为「绕过上下文治理」的嫌疑对象
      // （plugins/di.rs::context_leak_suspects 同口径）。索引里先标出来，
      // 不等到安装才炸。
      context_leak_suspect: Boolean(manifest.system_prompt || manifest.full_context),
      trust: 'unverified',
      // 清单是**声明式**的（没有 entry，不可动态加载），能力由原生 RPC/UI 承担，
      // 所以「装」= 落一份清单 + 由 gateway 装配。可装，但不是「加载可执行插件」。
      installable: true,
      runtime: 'declarative-manifest',
      sha256: sha256(raw),
      manifest_url: urlFor(rel),
      path: rel,
    })
  }
  return items
}

const skills = collectSkills()
const plugins = collectPlugins()

/**
 * 采集 dsh 生态条目：扫目录找带 `dsh` 字段的 `package.json`。
 *
 * dsh 插件的真实形态（实测 `dsh-data/profiles/node_modules`：510 个包中 42 个带 `dsh`）：
 * `{ "dsh": { "bundle": { "patch": "./cordis.patch.yml" } } }` 或
 * `{ "dsh": { "client": { "inject": [...], "platform": "web" } } }`。
 * 它们运行在 Cordis（TS）里，**本工作台无法执行**，所以 `installable:false`、`runtime:"cordis-ts"`。
 * 索引它的价值是「可发现 + 可审计 + 可对照我们的适配表（`core/src/plugins/adaptation.rs`）」，
 * 不是「假装能装」。
 */
function collectDshPackages(roots) {
  const out = []
  const seenNames = new Set()
  const MAX_DEPTH = 4
  const SKIP_DIRS = new Set(['.git', 'dist', 'lib', 'src', 'test', 'tests', '__pycache__'])

  function readPkg(dir) {
    const pj = join(dir, 'package.json')
    if (!existsSync(pj)) return null
    let raw
    try {
      raw = readFileSync(pj)
    } catch (e) {
      warn(`读 dsh package.json 失败 ${pj}: ${e.message}`)
      return null
    }
    try {
      return { json: JSON.parse(raw.toString('utf8')), raw, path: pj }
    } catch (e) {
      // node_modules 里存在坏 JSON 属常态（半装/裁剪包），只记录不中断。
      warn(`dsh package.json 解析失败 ${relative(ROOT, pj)}: ${e.message}`)
      return null
    }
  }

  function walk(dir, depth) {
    if (depth > MAX_DEPTH) return
    let entries
    try {
      entries = readdirSync(dir, { withFileTypes: true })
    } catch (e) {
      if (e.code !== 'ENOENT') warn(`读目录失败 ${relative(ROOT, dir)}: ${e.message}`)
      return
    }
    for (const entry of entries) {
      if (!entry.isDirectory()) continue
      if (SKIP_DIRS.has(entry.name)) continue
      const child = join(dir, entry.name)
      const pkg = readPkg(child)
      if (pkg && pkg.json && pkg.json.dsh && pkg.json.name) {
        const name = String(pkg.json.name)
        if (!seenNames.has(name)) {
          seenNames.add(name)
          const dsh = pkg.json.dsh || {}
          const patch = dsh.bundle && typeof dsh.bundle.patch === 'string' ? dsh.bundle.patch : ''
          out.push({
            kind: 'dsh-plugin',
            source: 'dsh',
            name,
            version: String(pkg.json.version || '0.0.0'),
            description: String(pkg.json.description || ''),
            author: String(pkg.json.author || (pkg.json.repository?.url ? 'dsh-ecosystem' : '')),
            license: String(pkg.json.license || ''),
            // bundle patch = 这个包往 profile 里插入的插件行；client 段 = 它给 Web UI 注入了什么。
            bundle_patch: patch || null,
            client_platform: (dsh.client && dsh.client.platform) || null,
            client_injects: Array.isArray(dsh.client?.inject) ? dsh.client.inject.length : 0,
            // 诚实边界：Cordis/TS 运行时，本工作台不可执行。
            runtime: 'cordis-ts',
            installable: false,
            compat: 'index-only',
            // dsh 包不设 system_prompt 红线概念；标记 null 表示「不适用」，不是「安全」。
            context_leak_suspect: null,
            trust: 'unverified',
            sha256: sha256(pkg.raw),
            path: relative(ROOT, pkg.path),
          })
        }
      }
      // 只在 node_modules / 作用域目录下继续下钻，避免把 src 也扫一遍。
      if (entry.name === 'node_modules' || entry.name.startsWith('@')) walk(child, depth + 1)
    }
  }

  for (const root of roots) {
    if (!existsSync(root)) continue
    walk(root, 0)
  }
  return out
}

/**
 * 合并第三方索引：接受 `{plugins|items|entries:[]}` 或裸数组（与 `market_remote.rs` 同口径）。
 * 没有 `manifest_url` 的条目**不可安装**（我们只会下载清单直链，不做协议猜解）。
 */
function mergeExternalIndexes(paths) {
  const out = []
  for (const p of paths) {
    let parsed
    try {
      parsed = JSON.parse(readFileSync(p, 'utf8'))
    } catch (e) {
      warn(`外部索引读取/解析失败 ${relative(ROOT, p)}: ${e.message}`)
      continue
    }
    let list = []
    if (Array.isArray(parsed)) list = parsed
    else if (Array.isArray(parsed.plugins)) list = parsed.plugins
    else if (Array.isArray(parsed.items)) list = parsed.items
    else if (Array.isArray(parsed.entries)) list = parsed.entries
    else warn(`外部索引形状不认识（无 plugins/items/entries 数组）：${relative(ROOT, p)}`)

    for (const raw of list) {
      const name = String(raw?.name || '').trim()
      if (!name) {
        warn(`外部索引有条目缺 name：${relative(ROOT, p)}`)
        continue
      }
      const manifestUrl = typeof raw.manifest_url === 'string' ? raw.manifest_url.trim() : ''
      out.push({
        kind: 'external',
        source: 'external',
        name,
        version: String(raw.version || '0.0.0'),
        description: String(raw.description || ''),
        author: String(raw.author || ''),
        runtime: 'unknown',
        installable: Boolean(manifestUrl),
        compat: manifestUrl ? 'manifest-installable' : 'index-only',
        reason: manifestUrl ? '有 manifest_url，可走 market.install_url' : '无 manifest_url，仅可发现',
        trust: 'unverified',
        manifest_url: manifestUrl || null,
        origin_index: relative(ROOT, p),
      })
    }
  }
  return out
}

const dshPlugins = collectDshPackages(DSH_ROOTS)
const externalItems = mergeExternalIndexes(EXTERNAL_INDEXES)
const items = [...skills, ...plugins, ...dshPlugins, ...externalItems]

// 同名冲突会让 install 行为不可预测（谁覆盖谁取决于顺序），必须硬失败。
const seen = new Map()
for (const item of items) {
  const prev = seen.get(item.name)
  if (prev) {
    warn(`名字冲突 ${item.name}：${prev.path} 与 ${item.path}`)
  } else {
    seen.set(item.name, item)
  }
}

const index = {
  schema: 'marguerite-market-index/1',
  generated_at: new Date().toISOString(),
  generator: 'scripts/gen-market-index.mjs',
  // 同时给 `items` 与 `plugins` 两个键：`market.remote.search` 四种形状都认，
  // 但第三方 registry 习惯 `plugins`，我们自己的前端习惯 `items`。同一份数组，不复制。
  items,
  plugins: items,
  total: items.length,
  counts: {
    plugin: plugins.length,
    skill: skills.length,
    dsh: dshPlugins.length,
    external: externalItems.length,
  },
  // 打通口径：能发现多少 / 能装多少 / 只是能看多少。三者分开报，不混成一句"已打通"。
  interop: {
    installable: items.filter(i => i.installable).length,
    discover_only: items.filter(i => !i.installable).length,
    by_source: {
      marguerite: items.filter(i => i.source === 'marguerite').length,
      dsh: dshPlugins.length,
      external: externalItems.length,
    },
    dsh_runtime_note: 'dsh 插件运行在 Cordis(TS)，本工作台不可执行；仅发现/审计/适配映射。',
  },
  skipped,
}

if (CHECK_ONLY) {
  if (skipped.length) {
    process.stderr.write(`[market-index] FAILED — ${skipped.length} 条被跳过\n`)
    process.exit(1)
  }
  // 供给面声明是**上线门槛**：缺 `requires` 的技能不许进官方索引。
  // 只卡 marguerite 自己的技能——dsh/第三方条目本来就不可安装，不适用这条。
  const missing = skills.filter(i => i.missing_requires).map(i => i.name)
  if (missing.length) {
    process.stderr.write(
      `[market-index] FAILED — ${missing.length} 个技能缺 requires 声明：${missing.slice(0, 10).join(', ')}`
      + `\n  修法：node scripts/stamp-skill-requires.mjs --apply（或手工写明它要什么）\n`,
    )
    process.exit(1)
  }
  process.stdout.write(
    `[market-index] OK — ${index.total} 条（plugin ${plugins.length} / skill ${skills.length}`
    + ` / dsh ${dshPlugins.length} / external ${externalItems.length}）；`
    + `可装 ${index.interop.installable} · 仅发现 ${index.interop.discover_only}`
    + ` · 缺 requires ${missing.length}\n`,
  )
  process.exit(0)
}

mkdirSync(dirname(OUT), { recursive: true })
writeFileSync(OUT, `${JSON.stringify(index, null, 2)}\n`, 'utf8')
process.stdout.write(
  `[market-index] 写入 ${relative(ROOT, OUT)} — ${index.total} 条`
  + `（plugin ${plugins.length} / skill ${skills.length} / dsh ${dshPlugins.length}`
  + ` / external ${externalItems.length}）；可装 ${index.interop.installable}`
  + ` · 仅发现 ${index.interop.discover_only}；base_url=${BASE_URL || '(未设置，url 为空)'}\n`,
)
if (skipped.length) {
  process.stderr.write(`[market-index] 有 ${skipped.length} 条被跳过，见上方 WARN\n`)
  process.exit(1)
}
