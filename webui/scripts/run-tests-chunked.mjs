#!/usr/bin/env node
// Chunked Vitest runner.
//
// Why this exists: a single `vitest run` over the full 290-file suite exceeds
// the resource ceiling on constrained machines (4 GB-class dev boxes, CI
// containers with low memory limits). The worker pool is torn down mid-run and
// the parent process dies *without emitting a summary* — indistinguishable, at
// the shell level, from a genuine test failure. Both produce a non-zero exit.
//
// This runner removes that ambiguity:
//   1. Test files are enumerated and executed in chunks.
//   2. A chunk that dies without producing a summary line is treated as an
//      infrastructure crash, not a failure. It is split in half and retried,
//      recursively, down to a single file.
//   3. Only a run that emits a summary and reports failing tests counts as a
//      real failure.
//
// A single file that still crashes on its own IS reported as a hard error —
// that is a real defect (OOM inside the test, infinite loop, native crash).
//
// Usage:
//   node scripts/run-tests-chunked.mjs [pathFilter ...] [--chunk=N] [--verbose]
//
// Examples:
//   node scripts/run-tests-chunked.mjs                    # whole suite
//   node scripts/run-tests-chunked.mjs src/views          # one subtree
//   node scripts/run-tests-chunked.mjs src/views src/utils/chat --chunk=6

import { spawnSync } from 'node:child_process'
import { readdirSync, statSync } from 'node:fs'
import { join, relative, sep } from 'node:path'

const ROOT = new URL('..', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1')

const args = process.argv.slice(2)
const filters = args.filter(a => !a.startsWith('--'))
const chunkArg = args.find(a => a.startsWith('--chunk='))
const CHUNK_SIZE = chunkArg ? Math.max(1, Number(chunkArg.split('=')[1]) || 8) : 8
const VERBOSE = args.includes('--verbose')

const TEST_RE = /\.(test|spec)\.ts$/
const SKIP_DIRS = new Set(['node_modules', 'dist', '.git'])

/** Recursively collect test files under `dir`. */
function collect(dir, out = []) {
  let entries
  try {
    entries = readdirSync(dir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const entry of entries) {
    if (entry.isDirectory()) {
      if (SKIP_DIRS.has(entry.name)) continue
      collect(join(dir, entry.name), out)
    } else if (TEST_RE.test(entry.name)) {
      out.push(join(dir, entry.name))
    }
  }
  return out
}

function discover() {
  const roots = filters.length ? filters : ['src']
  const files = new Set()
  for (const rootArg of roots) {
    const abs = join(ROOT, rootArg)
    let st
    try {
      st = statSync(abs)
    } catch {
      console.error(`[chunked] path not found: ${rootArg}`)
      process.exit(2)
    }
    if (st.isDirectory()) {
      for (const f of collect(abs)) files.add(f)
    } else if (TEST_RE.test(abs)) {
      files.add(abs)
    }
  }
  // Stable order so chunk composition is reproducible across runs.
  return [...files].map(f => relative(ROOT, f).split(sep).join('/')).sort()
}

/**
 * Run one vitest invocation over `files`.
 * @returns {{ crashed: boolean, failed: boolean, passedFiles: number, failedTests: number, output: string }}
 */
function runChunk(files) {
  const res = spawnSync(
    process.platform === 'win32' ? 'npx.cmd' : 'npx',
    ['vitest', 'run', ...files],
    { cwd: ROOT, encoding: 'utf8', shell: process.platform === 'win32', maxBuffer: 64 * 1024 * 1024 },
  )
  const output = `${res.stdout || ''}${res.stderr || ''}`
  // The summary block is the only reliable proof that the runner completed and
  // reported. Its absence means the process died before reporting.
  const summary = output.match(/Test Files\s+(.+)/)
  if (!summary) {
    return { crashed: true, failed: false, passedFiles: 0, failedTests: 0, output }
  }
  const failedTests = Number((output.match(/Tests\s+(\d+) failed/) || [])[1] || 0)
  const failedFiles = Number((output.match(/Test Files\s+(\d+) failed/) || [])[1] || 0)
  const passedFiles = Number((output.match(/Test Files.*?(\d+) passed/) || [])[1] || 0)
  return {
    crashed: false,
    failed: failedFiles > 0 || failedTests > 0 || res.status !== 0,
    passedFiles,
    failedTests,
    output,
  }
}

const realFailures = []
const hardCrashes = []
let filesPassed = 0

/** Run `files`, bisecting on infrastructure crashes. */
function runWithBisect(files, depth = 0) {
  const pad = '  '.repeat(depth)
  const result = runChunk(files)

  if (result.crashed) {
    if (files.length === 1) {
      // A lone file that cannot complete is a genuine defect, not scheduling noise.
      console.error(`${pad}✗ CRASH (single file, real defect): ${files[0]}`)
      hardCrashes.push(files[0])
      return
    }
    if (VERBOSE) {
      console.log(`${pad}~ chunk of ${files.length} died without a summary — bisecting`)
    }
    const mid = Math.ceil(files.length / 2)
    runWithBisect(files.slice(0, mid), depth + 1)
    runWithBisect(files.slice(mid), depth + 1)
    return
  }

  if (result.failed) {
    if (files.length === 1) {
      console.error(`${pad}✗ FAIL: ${files[0]}`)
      const detail = result.output
        .split('\n')
        .filter(l => /FAIL|AssertionError|✗|×|Error:/.test(l))
        .slice(0, 12)
        .join('\n')
      realFailures.push({ file: files[0], detail })
      return
    }
    // Narrow down to the exact failing file(s) so the report names them.
    if (VERBOSE) console.log(`${pad}~ chunk of ${files.length} has failures — narrowing`)
    const mid = Math.ceil(files.length / 2)
    runWithBisect(files.slice(0, mid), depth + 1)
    runWithBisect(files.slice(mid), depth + 1)
    return
  }

  filesPassed += files.length
  console.log(`${pad}✓ ${files.length} file(s) passed`)
}

const all = discover()
console.log(`[chunked] ${all.length} test file(s), chunk size ${CHUNK_SIZE}`)

for (let i = 0; i < all.length; i += CHUNK_SIZE) {
  const chunk = all.slice(i, i + CHUNK_SIZE)
  const idx = Math.floor(i / CHUNK_SIZE) + 1
  const total = Math.ceil(all.length / CHUNK_SIZE)
  console.log(`\n[chunk ${idx}/${total}] ${chunk.length} file(s)`)
  runWithBisect(chunk)
}

console.log('\n──────── summary ────────')
console.log(`files passed : ${filesPassed}/${all.length}`)
console.log(`files failed : ${realFailures.length}`)
console.log(`hard crashes : ${hardCrashes.length}`)

if (realFailures.length) {
  console.log('\nFailing files:')
  for (const f of realFailures) {
    console.log(`\n  ${f.file}`)
    for (const line of f.detail.split('\n')) console.log(`    ${line}`)
  }
}
if (hardCrashes.length) {
  console.log('\nFiles that crash in isolation:')
  for (const f of hardCrashes) console.log(`  ${f}`)
}

process.exit(realFailures.length || hardCrashes.length ? 1 : 0)
