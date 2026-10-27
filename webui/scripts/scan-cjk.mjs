import fs from 'node:fs'
import path from 'node:path'

const root = path.resolve(import.meta.dirname, '../src')
const skip = /(locales[\\/]|\.test\.|\.spec\.|i18n[\\/])/

function walk(dir, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) walk(full, out)
    else if (/\.(vue|ts|js)$/.test(entry.name)) out.push(full)
  }
  return out
}

function strip(src) {
  return src
    .replace(/<style[\s\S]*?<\/style>/g, '')
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/^\s*\/\/.*$/gm, '')
    .replace(/(^|\s)\/\/.*$/gm, '$1')
}

const rows = []
for (const file of walk(root)) {
  const rel = path.relative(root, file).replace(/\\/g, '/')
  if (skip.test(rel)) continue
  const lines = strip(fs.readFileSync(file, 'utf8')).split('\n')
  const hits = lines.filter(line => /[\u4e00-\u9fff]/.test(line))
  if (hits.length) rows.push({ rel, n: hits.length })
}

rows.sort((a, b) => b.n - a.n)
console.log('files:', rows.length, 'lines:', rows.reduce((s, r) => s + r.n, 0))
for (const r of rows.slice(0, 45)) console.log(String(r.n).padStart(5), r.rel)
