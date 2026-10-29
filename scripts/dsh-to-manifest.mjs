#!/usr/bin/env node
// 把 **dsh（DeepSeek Harness）生态的 npm 插件包** 转成 Marguerite 的声明式清单
// （`*.plugin.json`），让两个生态真正"对得上号"，而不是各说各话。
//
// # 为什么需要这一层（先读，别误解能力边界）
//
// 实测事实（`dsh-data/profiles/node_modules`，510 个包 / 42 个带 `dsh` 字段）：
// - dsh 插件 = npm 包 + `package.json.dsh`，运行在 **Cordis（TypeScript）** 里；
//   它的"接线"是一份 `cordis.patch.yml`（`- insert: [{id, name, config}]` 行）。
// - Marguerite 插件 = **声明式清单**（无 `entry`、不可动态加载），能力由原生 RPC/UI 承担
//   （见 `core/src/plugins/adaptation.rs` 的映射表与诚实边界）。
//
// 所以本转换器产出的是**可审计的对照清单**，不是"能让 dsh 插件跑起来"的魔法：
// 它把「这个包往 profile 里插了哪些行、注入了什么 client」记录下来，供：
//   ① 插件页展示「上游有这个能力 / 我们已适配 / 我们只有部分」；
//   ② `market.audit` 与红线扫描（`plugins/di.rs::context_leak_suspects`）统一入口；
//   ③ 将来写适配（native RPC）时有明确的"待办清单"。
//
// # 三条硬规矩
// 1. **绝不产出 `system_prompt` / `full_context`**：那是绕过上下文治理的红线字段。
// 2. **绝不写进 `plugins/manifests/`**：那里是运行时装配源，现有基线是 16 份清单
//    （`plugins.status` 的 E2E 口径）；批量写入会静默改掉基线。默认输出到
//    `dist/market/dsh-manifests/`，人工审过再决定是否迁入。
// 3. `installable: false` 一律写在文件里：dsh 包在本工作台**不可执行**，
//    清单只能描述"上游声明了什么"，不能假装能加载。
//
// 用法：
//   node scripts/dsh-to-manifest.mjs <path/to/package.json> [--out dir]
//   node scripts/dsh-to-manifest.mjs dsh-data/profiles/node_modules --all --out dist/market/dsh-manifests
//   node scripts/dsh-to-manifest.mjs <pkg> --print     # 打到 stdout，不落盘

import { readdirSync, readFileSync, mkdirSync, writeFileSync, existsSync, statSync } from 'node:fs'
import { basename, dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(HERE, '..')
const args = process.argv.slice(2)
const target = args.find(a => !a.startsWith('--'))
if (!target) {
  process.stderr.write('用法: node scripts/dsh-to-manifest.mjs <package.json|目录> [--all] [--out dir] [--print]\n')
  process.exit(2)
}
function argValue(flag, fallback) {
  const i = args.indexOf(flag)
  return i >= 0 && args[i + 1] ? args[i + 1] : fallback
}
const OUT_DIR = resolve(ROOT, argValue('--out', 'dist/market/dsh-manifests'))
const PRINT_ONLY = args.includes('--print')
const ALL = args.includes('--all')

const problems = []
function warn(msg) {
  problems.push(msg)
  process.stderr.write(`[dsh2manifest] WARN ${msg}\n`)
}

// `--all` 会扫过整棵 node_modules：绝大多数字包不是 dsh 包，这属于**预期**，
// 不能算 problem（否则退出码永远非 0，等于把"扫到 468 个普通包"报成失败）。
let nonDshCount = 0

/** `@scope/name` → `name`（我们的清单文件名与 `name` 必须一致，见 dsh-router 踩过的坑）。 */
function bareName(pkgName) {
  const s = String(pkgName || '').trim()
  if (!s) return ''
  const i = s.lastIndexOf('/')
  return (i >= 0 ? s.slice(i + 1) : s).replace(/[^\w.@-]/g, '-')
}

/**
 * 从 `cordis.patch.yml` 抽「插入行」。
 * 只做**行级扫描**，不实现 YAML 解析：patch 文件里我们关心的是 `- id:` 与 `name:`
 * 两种键（配置文件本体是上游格式，硬解析反而会在格式演进时假失败）。
 */
function scanPatchRows(text) {
  const ids = []
  const packages = []
  for (const line of text.split(/\r?\n/)) {
    const id = /^\s*-\s*id:\s*['"]?([^'"\s]+)['"]?/.exec(line)
    if (id) ids.push(id[1])
    const nm = /^\s+name:\s*['"]?([^'"\s]+)['"]?/.exec(line)
    if (nm) packages.push(nm[1])
  }
  return { ids, packages }
}

function convert(dirOrFile) {
  const pkgPath = statSync(dirOrFile).isDirectory() ? join(dirOrFile, 'package.json') : dirOrFile
  if (!existsSync(pkgPath)) {
    warn(`找不到 package.json: ${relative(ROOT, pkgPath)}`)
    return null
  }
  let raw
  let json
  try {
    raw = readFileSync(pkgPath, 'utf8')
    json = JSON.parse(raw)
  } catch (e) {
    warn(`解析失败 ${relative(ROOT, pkgPath)}: ${e.message}`)
    return null
  }
  if (!json.dsh) {
    if (ALL) {
      nonDshCount += 1
    } else {
      warn(`不是 dsh 包（无 dsh 字段）：${relative(ROOT, pkgPath)}`)
    }
    return null
  }
  const name = bareName(json.name)
  if (!name) {
    warn(`包名不可用：${relative(ROOT, pkgPath)}`)
    return null
  }

  const dsh = json.dsh || {}
  const patchRel = dsh.bundle && typeof dsh.bundle.patch === 'string' ? dsh.bundle.patch : ''
  let rows = { ids: [], packages: [] }
  if (patchRel) {
    const patchPath = join(dirname(pkgPath), patchRel)
    if (existsSync(patchPath)) {
      try {
        // 上限保护：patch 是配置，不该是数据文件。
        const text = readFileSync(patchPath, 'utf8')
        if (text.length > 2 * 1024 * 1024) warn(`patch 超 2MB，只取前 2MB：${relative(ROOT, patchPath)}`)
        rows = scanPatchRows(text.slice(0, 2 * 1024 * 1024))
      } catch (e) {
        warn(`读 patch 失败 ${relative(ROOT, patchPath)}: ${e.message}`)
      }
    } else {
      warn(`patch 文件不存在（声明了但没随包发布）：${patchRel}`)
    }
  }

  // 红线：绝不产出这两个字段（plugins/di.rs::context_leak_suspects 的判据）。
  const manifest = {
    name,
    version: String(json.version || '0.0.0'),
    description: String(json.description || ''),
    author: String(json.author || 'dsh-ecosystem'),
    license: String(json.license || ''),
    // dsh 包不声明 capabilities；这里留空而不是编造。
    capabilities: [],
    hooks: [],
    slash_commands: [],
    subagents: [],
    skills: [],
    dependencies: [],
    events: [],
    // 以下为「上游事实」区，全部来自 package.json / cordis.patch.yml，可逐项核对。
    upstream: {
      package: String(json.name || ''),
      version: String(json.version || ''),
      repository: json.repository?.url || json.repository || '',
      patch: patchRel || null,
      patch_rows: rows.ids.slice(0, 200),
      patch_row_count: rows.ids.length,
      inserted_packages: rows.packages.slice(0, 200),
      client_platform: (dsh.client && dsh.client.platform) || null,
      client_injects: Array.isArray(dsh.client?.inject) ? dsh.client.inject.length : 0,
    },
    // 诚实边界，写进文件本身，任何人打开清单就知道它不能被执行。
    runtime: 'cordis-ts',
    installable: false,
    compat: 'index-only',
    note: '由 scripts/dsh-to-manifest.mjs 从 dsh 包生成：声明上游能力，供审计与适配对照；'
      + '本工作台不执行 Cordis(TS) 插件，故 entry 恒为 null。',
  }
  return { name, manifest, sourcePath: pkgPath }
}

function collectDirs(root) {
  const found = []
  const MAX_DEPTH = 4
  const SKIP = new Set(['.git', 'dist', 'lib', 'src', 'test', 'tests'])
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
      if (SKIP.has(entry.name)) continue
      const child = join(dir, entry.name)
      if (existsSync(join(child, 'package.json'))) found.push(child)
      if (entry.name === 'node_modules' || entry.name.startsWith('@')) walk(child, depth + 1)
    }
  }
  walk(root, 0)
  return found
}

const targets = ALL ? collectDirs(resolve(ROOT, target)) : [resolve(ROOT, target)]
const converted = []
const seen = new Map()
for (const t of targets) {
  const r = convert(t)
  if (!r) continue
  const prev = seen.get(r.name)
  if (prev) {
    // 同名多版本会让 install 行为不可预测：只保留版本号字典序更大的一个，并明确告警。
    if (String(prev.manifest.version) >= String(r.manifest.version)) {
      warn(`同名包跳过（保留 ${prev.manifest.version}）：${r.name} ← ${relative(ROOT, r.sourcePath)}`)
      continue
    }
    warn(`同名包替换（${prev.manifest.version} → ${r.manifest.version}）：${r.name}`)
  }
  seen.set(r.name, r)
  converted.push(r)
}

if (PRINT_ONLY) {
  const one = converted[0]
  if (!one) process.exit(1)
  process.stdout.write(`${JSON.stringify(one.manifest, null, 2)}\n`)
  process.exit(0)
}

if (!converted.length) {
  process.stderr.write('[dsh2manifest] 没有可转换的 dsh 包。\n')
  process.exit(problems.length ? 1 : 0)
}

mkdirSync(OUT_DIR, { recursive: true })
for (const c of converted) {
  const file = join(OUT_DIR, `${c.name}.plugin.json`)
  writeFileSync(file, `${JSON.stringify(c.manifest, null, 2)}\n`, 'utf8')
}
process.stdout.write(
  `[dsh2manifest] 转换 ${converted.length} 个 dsh 包 → ${relative(ROOT, OUT_DIR)}`
  + `（扫描过的非 dsh 包 ${nonDshCount}，problems ${problems.length}）\n`,
)
if (problems.length) process.exit(1)
