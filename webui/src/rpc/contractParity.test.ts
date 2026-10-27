/**
 * 前后端 RPC 契约颗粒度守卫。
 *
 * 为什么需要：`supportsMethod()` 只读握手帧里的 `features.methods`（= Rust 侧
 * `core/src/gateway/protocol.rs::SUPPORTED_METHODS`）。一旦前端探测了一个后端没声明的方法，
 * UI 功能入口会**永远**不显示；反过来，后端漏登记但前端已经在 `rpc.call` 的方法，
 * 同样说明颗粒度已经漂移。该守卫在单测层面把两侧的**字面量**放在一起比对。
 *
 * 动态拼接（如 `'models.local.' + name`）与模板字符串无法静态判定，天然不匹配字面量正则；
 * 以 `.` 结尾的前缀字面量（`'models.local.'`）也显式排除。
 *
 * 注意：本文件自身也在 `src/**` 扫描范围内，所以不能在里面写 supportsMethod(…) /
 * rpc.call(…) 这类带引号的示例调用字面量，否则会被自己的扫描器当成真实调用。
 */
import { readFileSync, readdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'
import { describe, expect, it } from 'vitest'

const testDir = dirname(fileURLToPath(import.meta.url))
const SRC_DIR = join(testDir, '..')
const PROTOCOL_PATH = fileURLToPath(
  new URL('../../../core/src/gateway/protocol.rs', import.meta.url),
)
const HARNESS_VIEW_PATH = join(SRC_DIR, 'views', 'HarnessView.vue')

/** 本轮新增的读型方法：后端 must 登记，前端才有探测/调用依据。 */
const NEW_METHODS = [
  'harness.ledger.status',
  'harness.ledger.query',
  'harness.ledger.replay',
  'skills.coverage',
  'sessions.insights',
] as const

/**
 * 逐行剥离 Rust 注释后再取字符串字面量。
 *
 * 不能先用块注释正则做整块剥离：protocol.rs 的行注释里存在 `skills/trae/**` 与
 * `settings.*` / `auto_enabled.*` 这类文本，其中的星号加斜杠会被误判成块注释结尾，
 * 把大半个数组吞掉（既有 check-rpc-contract.mjs 正是因此只解析出 120 条）。
 */
function declaredMethods(): Set<string> {
  const source = readFileSync(PROTOCOL_PATH, 'utf8')
  const start = source.indexOf('pub const SUPPORTED_METHODS')
  if (start < 0) throw new Error(`在 ${PROTOCOL_PATH} 里找不到 SUPPORTED_METHODS 声明`)
  const open = source.indexOf('&[', start)
  const close = source.indexOf('];', open)
  if (open < 0 || close < 0) throw new Error('SUPPORTED_METHODS 数组边界解析失败')
  const body = source
    .slice(open, close)
    .split('\n')
    .map((line) => {
      if (line.trim().startsWith('//')) return ''
      const inline = line.indexOf('//')
      return inline >= 0 ? line.slice(0, inline) : line
    })
    .join('\n')
  const methods = new Set<string>()
  for (const match of body.matchAll(/"([^"]+)"/g)) methods.add(match[1])
  return methods
}

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name)
    if (entry.isDirectory()) {
      if (entry.name === 'node_modules') continue
      sourceFiles(full, out)
    } else if (entry.name.endsWith('.ts') || entry.name.endsWith('.vue')) {
      out.push(full)
    }
  }
  return out
}

/** supportsMethod 门控点：方法名为单/双引号字面量。 */
const GATE_RE = /supportsMethod\s*\(\s*(['"])([A-Za-z0-9_.-]+)\1\s*\)/g
/** rpc.call 调用点：可选泛型实参，方法名为单/双引号字面量。 */
const CALL_RE = /\brpc\.call(?:<[^>]*>)?\s*\(\s*(['"])([A-Za-z0-9_.-]+)\1/g

function collectLiterals(regex: RegExp, text: string, into: Set<string>): void {
  for (const match of text.matchAll(regex)) {
    const method = match[2]
    // 动态拼接的前缀（`'models.local.'`）不是完整方法名，跳过。
    if (method.endsWith('.')) continue
    into.add(method)
  }
}

function scanFrontend(): { gates: Set<string>; calls: Set<string> } {
  const gates = new Set<string>()
  const calls = new Set<string>()
  for (const file of sourceFiles(SRC_DIR)) {
    const text = readFileSync(file, 'utf8')
    collectLiterals(GATE_RE, text, gates)
    collectLiterals(CALL_RE, text, calls)
  }
  return { gates, calls }
}

const declared = declaredMethods()
const { gates, calls } = scanFrontend()

function missingFrom(actual: Set<string>): string[] {
  return [...actual].filter((method) => !declared.has(method)).sort()
}

describe('RPC 契约颗粒度', () => {
  it('SUPPORTED_METHODS 可被完整解析（防止解析规则退化后静默通过）', () => {
    // 真值约 400 条；这里只做下界哨兵，解析失效会显著小于它。
    expect(declared.size, `只解析出 ${declared.size} 条，解析规则可能已失效`).toBeGreaterThan(300)
  })

  it('① 每个 supportsMethod 字面量都在后端 SUPPORTED_METHODS 内', () => {
    const missing = missingFrom(gates)
    expect(
      missing,
      `以下 supportsMethod 门控的方法未在 SUPPORTED_METHODS 声明（对应功能入口永远不可见）：\n  ${missing.join('\n  ')}`,
    ).toEqual([])
  })

  it('② 每个 rpc.call 字面量都在后端 SUPPORTED_METHODS 内', () => {
    const missing = missingFrom(calls)
    expect(
      missing,
      `以下 rpc.call 调用的方法未在 SUPPORTED_METHODS 声明（前后端颗粒度已漂移）：\n  ${missing.join('\n  ')}`,
    ).toEqual([])
  })

  it('③ 本轮新增的读型方法都已在 SUPPORTED_METHODS 登记', () => {
    const missing = NEW_METHODS.filter((method) => !declared.has(method))
    expect(
      missing,
      `SUPPORTED_METHODS 缺少本轮冻结契约方法（后端漏登记会导致 UI 静默失效）：\n  ${missing.join('\n  ')}`,
    ).toEqual([])
  })

  it('HarnessView 里调用的 RPC 名称都在契约内且只用了这些读型方法', () => {
    const viewCalls = new Set<string>()
    collectLiterals(CALL_RE, readFileSync(HARNESS_VIEW_PATH, 'utf8'), viewCalls)
    const missing = [...viewCalls].filter((method) => !declared.has(method)).sort()
    expect(
      missing,
      `HarnessView 调用了未声明的 RPC：\n  ${missing.join('\n  ')}`,
    ).toEqual([])
    expect([...viewCalls].sort()).toEqual([...NEW_METHODS].sort())
  })
})
