// RPC 契约守卫：前端 `supportsMethod('X')` 门控的方法，必须在网关 SUPPORTED_METHODS 声明。
//
// 为什么需要它：`supportsMethod()` 是前端做「功能入口可见性」判断的唯一依据，而它只读
// 握手帧里的 `features.methods`（= Rust 的 `SUPPORTED_METHODS` 常量）。一旦某个方法
// 实现了却漏声明，前端就会**永远**判定不支持——功能不是"偶尔失败"，而是**根本点不到**：
//   - sessions.promptCacheKeepalive.*（提示缓存保活对话框永不渲染）
//   - migration.sources.preview（数据迁移预览被跳过）
//   - onboarding.imageGeneration.models.discover（图片生成模型列表永远为空）
// 这三个都是真实发生过的漂移，且没有任何守卫拦得住，故补此守卫。
//
// 范围说明：这里只校验 `supportsMethod('X')` 的**静态字面量**（门控点，危害最大且可
// 静态判定）；`rpc.call('X')` 的动态拼接（如 `'models.local.' + x`）无法静态校验，
// 且 `rpc.call` 不查 supportsMethod（未声明仍可调用），故不纳入。

import { readFileSync, readdirSync } from 'node:fs'
import { dirname, resolve, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const SRC_DIR = resolve(here, '..', 'src')
const PROTOCOL = resolve(here, '..', '..', 'core', 'src', 'gateway', 'protocol.rs')

const SRC_EXT = ['.vue', '.ts', '.tsx', '.js', '.mjs']
const TEST_FILE_RE = /\.(test|spec)\./
// `supportsMethod('a.b')` / `supportsMethod("a.b")`
const GATE_RE = /supportsMethod\s*\(\s*(['"])([A-Za-z0-9_.\-]+)\1\s*\)/g

function sourceFiles(dir, out = []) {
  for (const ent of readdirSync(dir, { withFileTypes: true })) {
    const full = resolve(dir, ent.name)
    if (ent.isDirectory()) sourceFiles(full, out)
    else if (SRC_EXT.some((e) => ent.name.endsWith(e)) && !TEST_FILE_RE.test(ent.name)) out.push(full)
  }
  return out
}

/** 去掉行注释与块注释，避免注释里的方法名被当成声明。 */
function stripComments(text) {
  return text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '')
}

function supportedMethods() {
  const src = readFileSync(PROTOCOL, 'utf8')
  const start = src.indexOf('SUPPORTED_METHODS')
  if (start < 0) throw new Error('protocol.rs 里找不到 SUPPORTED_METHODS')
  const open = src.indexOf('[', start)
  const close = src.indexOf('];', open)
  const block = stripComments(src.slice(open, close))
  const out = new Set()
  for (const m of block.matchAll(/"([^"]+)"/g)) out.add(m[1])
  return out
}

/** 声明表里是否有重复条目（重复无害但会让清单腐化，且掩盖"改了这里忘了那里"）。 */
function duplicateDeclarations() {
  const src = readFileSync(PROTOCOL, 'utf8')
  const start = src.indexOf('SUPPORTED_METHODS')
  const open = src.indexOf('[', start)
  const close = src.indexOf('];', open)
  const block = stripComments(src.slice(open, close))
  const seen = new Map()
  for (const m of block.matchAll(/"([^"]+)"/g)) seen.set(m[1], (seen.get(m[1]) ?? 0) + 1)
  return [...seen.entries()].filter(([, n]) => n > 1).map(([m, n]) => `${m} ×${n}`)
}

const declared = supportedMethods()
if (declared.size < 200) {
  console.error(`[check-rpc-contract] SUPPORTED_METHODS 只解析出 ${declared.size} 条，解析规则可能已失效`)
  process.exit(1)
}

const gated = new Map()
for (const file of sourceFiles(SRC_DIR)) {
  const text = readFileSync(file, 'utf8')
  GATE_RE.lastIndex = 0
  let m
  while ((m = GATE_RE.exec(text))) {
    const method = m[2]
    if (!gated.has(method)) gated.set(method, new Set())
    gated.get(method).add(relative(resolve(here, '..'), file))
  }
}

if (gated.size === 0) {
  console.error('[check-rpc-contract] 没有扫到任何 supportsMethod 门控点，扫描规则可能已失效')
  process.exit(1)
}

const missing = [...gated.entries()]
  .filter(([method]) => !declared.has(method))
  .sort(([a], [b]) => a.localeCompare(b))

if (missing.length) {
  console.error(
    `[check-rpc-contract] ${missing.length} 个 supportsMethod 门控的方法未在 SUPPORTED_METHODS 声明` +
      '（对应功能入口永远不会显示）：',
  )
  for (const [method, files] of missing) {
    console.error(`  - ${method}   ← ${[...files].join(', ')}`)
  }
  console.error('\n修法：在 core/src/gateway/protocol.rs 的 SUPPORTED_METHODS 里补上该方法。')
  process.exit(1)
}

console.log(
  `[check-rpc-contract] OK — ${gated.size} 个 supportsMethod 门控方法全部已在 SUPPORTED_METHODS 声明`,
)

// ─────────────────────────────────────────────────────────────────────────────
// 反方向：SUPPORTED_METHODS 里声明的方法，必须真的有人实现
//
// 为什么需要它：声明是握手帧里唯一的「能力清单」，UI 据此显示功能入口。若声明了一个
// 没人实现的方法，用户看到入口、点了必然失败。真实案例：`exec.proposals.create/update/
// remove/approve/apply` 这 5 个名字**侧车里根本不存在**（侧车只有 pending_count/list/
// show/accept/reject/settings.*/auto_enabled.*），是早期 API 的残留名；`migration.preview`
// 与 `onboarding.channel.configure` 同理。
//
// 实现来源（四处任一即可）：
//   1. Rust dispatch 臂（methods.rs / goal.rs / ops_dispatch.rs）
//   2. ASYNC_ONLY_METHODS / RUST_FIRST_METHODS
//   3. Python 侧车 METHOD_SCOPES（权威清单，侧车实现了就一定在里面）
// ─────────────────────────────────────────────────────────────────────────────

const CORE_GATEWAY = resolve(here, '..', '..', 'core', 'src', 'gateway')
const SCOPES_PY = resolve(here, '..', '..', 'pyengine', 'src', 'opensquilla', 'gateway', 'scopes.py')

/** 方法名可含点也可不含（`health` / `status` 都是真实方法），故点号段用 `*`。 */
const ARM_RE = /^\s*"([A-Za-z0-9_]+(?:\.[A-Za-z0-9_\-]+)*)"\s*=>/gm
/** `agent_rpc_err` 里对「错误码字符串」的 match 不是 RPC 臂。 */
const ERR_CODE_LOOKALIKES = new Set(['agent.exists', 'agent.not_found', 'agent.builtin_immutable'])

function namesInList(file, constName) {
  const src = stripComments(readFileSync(file, 'utf8'))
  const i = src.indexOf(constName)
  if (i < 0) return new Set()
  const open = src.indexOf('[', i)
  const close = src.indexOf('];', open)
  const out = new Set()
  for (const m of src.slice(open, close).matchAll(/"([^"]+)"/g)) out.add(m[1])
  return out
}

function implementedMethods() {
  const out = new Set()
  for (const f of ['methods.rs', 'goal.rs', 'ops_dispatch.rs']) {
    const src = stripComments(readFileSync(resolve(CORE_GATEWAY, f), 'utf8'))
    for (const m of src.matchAll(ARM_RE)) out.add(m[1])
  }
  for (const name of ERR_CODE_LOOKALIKES) out.delete(name)
  for (const constName of ['ASYNC_ONLY_METHODS', 'RUST_FIRST_METHODS']) {
    for (const m of namesInList(resolve(CORE_GATEWAY, 'methods.rs'), constName)) out.add(m)
  }
  // Python 侧车权威清单：`"method.name": SCOPE,`
  const py = stripComments(readFileSync(SCOPES_PY, 'utf8'))
  for (const m of py.matchAll(/"([A-Za-z0-9_]+(?:\.[A-Za-z0-9_\-]+)*)"\s*:/g)) out.add(m[1])
  return out
}

const implemented = implementedMethods()
if (implemented.size < 200) {
  console.error(
    `[check-rpc-contract] 只解析出 ${implemented.size} 条实现，解析规则可能已失效（拒绝静默通过）`,
  )
  process.exit(1)
}

const dupes = duplicateDeclarations()
if (dupes.length) {
  console.error(`[check-rpc-contract] SUPPORTED_METHODS 有 ${dupes.length} 条重复声明：`)
  for (const d of dupes) console.error(`  - ${d}`)
  process.exit(1)
}

const phantom = [...declared].filter((m) => !implemented.has(m)).sort()
if (phantom.length) {
  console.error(
    `[check-rpc-contract] ${phantom.length} 条声明找不到任何实现（幽灵声明：UI 会显示入口但调用必失败）：`,
  )
  for (const m of phantom) console.error(`  - ${m}`)
  console.error(
    '\n修法：在 core/src/gateway/protocol.rs 的 SUPPORTED_METHODS 里删掉它，' +
      '或按 Python 侧车 scopes.py 的真实方法名改正。',
  )
  process.exit(1)
}

console.log(
  `[check-rpc-contract] OK — SUPPORTED_METHODS ${declared.size} 条声明全部有实现（幽灵声明 0 条）`,
)

// ─────────────────────────────────────────────────────────────────────────────
// 双实现守卫：命中 Python 代理前缀 **且** 有 Rust dispatch 臂的方法，必须在白名单内
//
// 为什么需要它（2026-10-09 实测）：
//   `PY_ENGINE_PREFIXES` 命中即「优先代理到 Python 侧车」，侧车离线才回落 Rust。
//   于是同一方法的行为**取决于侧车是否在线** —— 两侧字段形状/状态源/读写同源都可能不同。
//   已成病例：`artifacts.list`（Rust 侧登记产物、Python 侧应答空库 ⇒ 产物面板永远空）。
//   实测全仓 47 个方法处于此状态，其中 `channels.*` 的 Rust 臂是**空桩**
//   （`json!({"channels": []})`）⇒ 侧车离线时渠道列表**静默变空**。
//
//   本守卫**不要求立刻修完 47 个**（那是独立工作流），只保证：
//   **不再有第 48 个悄悄加入**。新增一条必须显式二选一 ——
//     ① 同源：加入 `methods.rs` 的 `RUST_OWNED_METHODS`（禁止代理）；
//     ② 委托：把方法名加进下面的白名单，并在 PR 里说明为何两侧可共存。
//
//   补充说明（RUST_FIRST 误报）：命中 `RUST_FIRST_METHODS` 的方法行为**不随侧车在线与否变化**——
//   `dispatch` 在 ② `RUST_FIRST_METHODS` 段直接调用 Rust 实现且非 null 即 return，Rust 侧恒抢答，
//   Python 侧车永不接管。故其「双实现」是**静态共存**而非运行时切换，本质与本守卫要防的
//   「行为随侧车切换」不同，属误报。本守卫暂以基线（DUAL_IMPL_BASELINE）方式容忍这类方法，
//   后续可加 `&& !rustFirst.has(m)` 过滤条件收敛，把纯 RUST_FIRST 方法移出双实现名单。
//   （本次仅补注释说明，**不改判定逻辑**，避免影响现有 exit code。）
// ─────────────────────────────────────────────────────────────────────────────

/** 既有 47 个双实现（2026-10-09 基线，待逐项语义审计）。**只许减少，不许新增。** */
const DUAL_IMPL_BASELINE = new Set([
  'artifacts.edit_inline', 'artifacts.get', 'artifacts.list',
  'artifacts.open_in_system', 'artifacts.register',
  'channels.list', 'channels.pairings', 'channels.status',
  'memory.import.apply', 'memory.import.cancel', 'memory.import.discard',
  'memory.import.info', 'memory.import.retry', 'memory.import.start',
  'memory.import.status', 'memory.import.undo',
  'models.gate.check', 'models.list',
  'models.local.catalog', 'models.local.deploy', 'models.local.download',
  'models.local.finetune', 'models.local.finetune.cancel',
  'models.local.finetune.list', 'models.local.finetune.status',
  'models.local.list', 'models.local.recommend', 'models.local.remove',
  'models.local.start', 'models.local.status', 'models.local.stop',
  'models.local.system',
  'models.profiles.add', 'models.profiles.list', 'models.profiles.remove',
  'models.profiles.set_active', 'models.profiles.set_copilot',
  'models.routing.get', 'models.routing.set',
  'onboarding.catalog', 'onboarding.ensemble.configure',
  'onboarding.provider.configure', 'onboarding.router.configure',
  'onboarding.status',
  'providers.status', 'router.decision', 'router.selflearning.status',
])

function dualImplementations() {
  const src = stripComments(
    readFileSync(resolve(CORE_GATEWAY, 'methods.rs'), 'utf8'),
  )
  const block = src.match(/PY_ENGINE_PREFIXES:\s*&\[&str\]\s*=\s*&\[([\s\S]*?)\];/)
  if (!block) return null
  const prefixes = [...block[1].matchAll(/"([^"]+)"/g)].map((m) => m[1])
  const arms = new Set(
    [...src.matchAll(/^\s*"([a-z0-9_.]+)"\s*=>/gm)].map((m) => m[1]),
  )
  // RUST_OWNED_METHODS 里的方法已被显式禁止代理，不算双实现
  const owned = new Set(
    [...(src.match(/RUST_OWNED_METHODS:\s*&\[&str\]\s*=\s*&\[([\s\S]*?)\];/)?.[1] ?? '')
      .matchAll(/"([^"]+)"/g)].map((m) => m[1]),
  )
  return [...arms]
    .filter((m) => !owned.has(m) && prefixes.some((p) => m.startsWith(p)))
    .sort()
}

const dual = dualImplementations()
if (dual === null) {
  console.error(
    '[check-rpc-contract] 无法解析 PY_ENGINE_PREFIXES，双实现守卫拒绝静默通过',
  )
  process.exit(1)
}

const newcomers = dual.filter((m) => !DUAL_IMPL_BASELINE.has(m))
if (newcomers.length) {
  console.error(
    `[check-rpc-contract] 新增 ${newcomers.length} 个「双实现」方法（命中 Python 代理前缀且已有 Rust 臂）。\n` +
      '这会让该方法的返回值取决于侧车是否在线（两套数据源 / 两套字段形状）：',
  )
  for (const m of newcomers) console.error(`  - ${m}`)
  console.error(
    '\n修法（二选一）：\n' +
      '  ① 同源：在 core/src/gateway/methods.rs 的 RUST_OWNED_METHODS 里加上它（禁止代理，Rust 独占）；\n' +
      '  ② 委托：把方法名加入本脚本 DUAL_IMPL_BASELINE 并说明两侧为何可共存。',
  )
  process.exit(1)
}

const resolved = [...DUAL_IMPL_BASELINE].filter((m) => !dual.includes(m))
console.log(
  `[check-rpc-contract] OK — 双实现守卫：${dual.length} 个在册（基线 ${DUAL_IMPL_BASELINE.size}）` +
    (resolved.length ? `，其中 ${resolved.length} 个已消除：${resolved.join(', ')}` : '') ,
)
