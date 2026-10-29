import { execFileSync } from 'node:child_process'
import { mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterAll, describe, expect, it } from 'vitest'

// The built console ships inside the desktop installer. Measured on the
// published 0.2.17 MSI: 206 of 206 emitted `.js.map` files carried a
// non-empty `sourcesContent`, so 771 hand-written frontend source files
// (32.45 MB) travelled to end users inside a product that is not open
// source. The guard under test is what makes that loud at build time
// instead of silent at install time.
const webuiRoot = resolve(fileURLToPath(new URL('..', import.meta.url)))
const verifier = join(webuiRoot, 'scripts', 'verify-dist.mjs')

function runVerifier(args: string[]): { code: number; stderr: string } {
  try {
    execFileSync(process.execPath, [verifier, ...args], {
      cwd: webuiRoot,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    return { code: 0, stderr: '' }
  } catch (error) {
    const failure = error as { status?: number; stderr?: string }
    return { code: failure.status ?? 1, stderr: failure.stderr ?? '' }
  }
}

const created: string[] = []

function builtDist(files: Record<string, string>, tempDir: string): string {
  const root = resolve(webuiRoot, tempDir)
  mkdirSync(root, { recursive: true })
  created.push(root)
  const entries: Record<string, string> = {
    'index.html':
      '<script type="module" src="assets/app.js"></script>' +
      '<link rel="stylesheet" href="assets/app.css">\n',
    'assets/app.js': 'console.log("app")\n',
    'assets/app.css': 'body{margin:0}\n',
    ...files,
  }
  for (const [path, body] of Object.entries(entries)) {
    const full = join(root, ...path.split('/'))
    mkdirSync(dirname(full), { recursive: true })
    writeFileSync(full, body, 'utf8')
  }
  // The manifest is written by the verifier itself so the source fingerprint
  // always matches; this test is about artifact contents, not staleness.
  runVerifier(['--write', root])
  return root
}

afterAll(() => {
  for (const dir of created) rmSync(dir, { recursive: true, force: true })
})

describe('shipped console artifact must not carry frontend source', () => {
  it('rejects a source map file', () => {
    const dist = builtDist(
      {
        'assets/app.js.map': JSON.stringify({
          version: 3,
          sources: ['../../src/App.vue'],
          sourcesContent: ['<template>\n  <div>{{ secret }}</div>\n</template>\n'],
          mappings: 'AAAA',
        }),
      },
      join('.tmp', `sourcemap-guard-${process.pid}-1`),
    )

    const result = runVerifier([dist])

    expect(result.code, `expected failure, stderr was: ${result.stderr}`).not.toBe(0)
    expect(result.stderr).toContain('assets/app.js.map')
  })

  it('rejects inlined source even without a map file', () => {
    const dist = builtDist(
      {
        'assets/app.js':
          'console.log("app")\n//# inline={"sourcesContent":["<template>secret</template>"]}\n',
      },
      join('.tmp', `sourcemap-guard-${process.pid}-2`),
    )

    const result = runVerifier([dist])

    expect(result.code, `expected failure, stderr was: ${result.stderr}`).not.toBe(0)
    expect(result.stderr).toContain('assets/app.js')
  })
})
