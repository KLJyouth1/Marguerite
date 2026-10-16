// Version single-source-of-truth assertion.
// Checks that tauri.conf.json, gui Cargo.toml (marguerite-gui) and core Cargo.toml
// (marguerite-core) all carry the same package version, and that any git tag is
// consistent with that version. Pure Node, no third-party dependencies.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(SCRIPT_DIR, '..');

const TAURI_CONF = path.join(REPO_ROOT, 'gui', 'src-tauri', 'tauri.conf.json');
const GUI_CARGO = path.join(REPO_ROOT, 'gui', 'src-tauri', 'Cargo.toml');
const CORE_CARGO = path.join(REPO_ROOT, 'core', 'Cargo.toml');

const failures = [];

function fail(msg) {
  failures.push(msg);
}

// Parse the [package] table's version from a Cargo.toml file, scoped to a
// specific package name when provided. Handles [package] and
// [lib]/[[bin]] sections that follow it.
function cargoPackageVersion(filePath, expectedName) {
  const text = fs.readFileSync(filePath, 'utf8');
  const lines = text.split(/\r?\n/);
  let inPackage = false;
  let nameMatched = false;
  let version = null;
  let name = null;
  for (const raw of lines) {
    const line = raw.trim();
    if (line.startsWith('[')) {
      // Commit any [package] result once the section ends.
      if (inPackage && version !== null) {
        if (expectedName === undefined || nameMatched) return version;
      }
      if (line === '[package]') {
        inPackage = true;
        nameMatched = expectedName === undefined;
        version = null;
        name = null;
      } else {
        inPackage = false;
      }
      continue;
    }
    if (!inPackage || line.startsWith('#') || line.length === 0) continue;
    const m = line.match(/^(name|version)\s*=\s*"([^"]+)"/);
    if (m) {
      if (m[1] === 'name') {
        name = m[2];
        if (expectedName !== undefined) nameMatched = name === expectedName;
      } else if (version === null) {
        version = m[2];
      }
    }
  }
  if (inPackage && version !== null && (expectedName === undefined || nameMatched)) {
    return version;
  }
  return null;
}

function tauriConfVersion(filePath) {
  const text = fs.readFileSync(filePath, 'utf8');
  const conf = JSON.parse(text);
  return conf.version ?? null;
}

// --- Collect versions ---
let versions = {};
try {
  versions.tauri = tauriConfVersion(TAURI_CONF);
  versions.gui = cargoPackageVersion(GUI_CARGO, 'marguerite-gui');
  versions.core = cargoPackageVersion(CORE_CARGO, 'marguerite-core');
} catch (err) {
  console.error('FATAL: failed to read version sources:', err.message);
  process.exit(1);
}

console.log('Version single source of truth check');
console.log('  tauri.conf.json .version              =', versions.tauri ?? '(missing)');
console.log('  gui/src-tauri/Cargo.toml (marguerite-gui) =', versions.gui ?? '(missing)');
console.log('  core/Cargo.toml (marguerite-core)         =', versions.core ?? '(missing)');

for (const [k, v] of Object.entries(versions)) {
  if (v === null || v === undefined) {
    fail(`could not determine version for ${k}`);
  }
}

const distinct = new Set(Object.values(versions).filter(Boolean));
if (distinct.size > 1) {
  fail(`version mismatch across sources: ${JSON.stringify(versions)}`);
} else if (distinct.size === 1) {
  console.log('  all three sources agree on version', [...distinct][0]);
}

// --- Git tag check ---
let tag = null;
try {
  const out = execFileSync('git', ['-C', REPO_ROOT, 'tag', '--list'], {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'ignore'],
  });
  const tags = out.split(/\r?\n/).map((t) => t.trim()).filter(Boolean);
  if (tags.length === 0) {
    console.log('WARN: no git tags found in repository (skip tag consistency check)');
  } else if (distinct.size === 1) {
    const ver = [...distinct][0];
    const matching = tags.filter((t) => t === `v${ver}` || t === ver);
    if (matching.length > 0) {
      console.log(`  git tag consistency OK: found matching tag(s) ${matching.join(', ')}`);
    } else {
      fail(`git tags exist (${tags.join(', ')}) but none matches version ${ver}`);
    }
    tag = tags.join(', ');
  }
} catch (err) {
  console.log(`WARN: git tag check skipped (${err.message})`);
}

// --- Verdict ---
if (failures.length > 0) {
  console.error('');
  console.error(`FAIL: ${failures.length} problem(s) detected:`);
  for (const f of failures) console.error('  - ' + f);
  process.exit(1);
}
console.log('');
console.log('PASS: all version sources are in sync.');
process.exit(0);
