const fs = require('fs')

function replaceOnce(file, from, to) {
  const s = fs.readFileSync(file, 'utf8')
  const i = s.indexOf(from)
  if (i === -1) { console.log('MISS', file, JSON.stringify(from)); return }
  fs.writeFileSync(file, s.slice(0, i) + to + s.slice(i + from.length))
  console.log('OK', file)
}
function removeLineRe(file, re) {
  const s = fs.readFileSync(file, 'utf8')
  const newS = s.replace(re, '')
  if (newS === s) { console.log('MISS', file, re.toString()); return }
  fs.writeFileSync(file, newS)
  console.log('RM', file)
}

// skill_tools.rs: grouped imports — keep the still-used sibling token
replaceOnce('core/src/tools/skill_tools.rs', 'use std::path::{Path, PathBuf};', 'use std::path::PathBuf;')
replaceOnce('core/src/tools/skill_tools.rs', 'use anyhow::{anyhow, Result};', 'use anyhow::Result;')
removeLineRe('core/src/tools/skill_tools.rs', /^[ \t]*use async_trait::async_trait;\r?\n/m)

// 7 unused `std::os::windows::process::CommandExt` imports (indentation-independent)
const cmdExtFiles = [
  'core/src/tools/builtin.rs',
  'core/src/tools/terminal.rs',
  'core/src/tools/code.rs',
  'core/src/security/exec.rs',
  'core/src/sandbox/mod.rs',
  'core/src/pyengine/sidecar.rs',
]
for (const f of cmdExtFiles) removeLineRe(f, /^[ \t]*use std::os::windows::process::CommandExt;\r?\n/m)
// builtin.rs has a second occurrence with different indentation
removeLineRe('core/src/tools/builtin.rs', /^[ \t]*use std::os::windows::process::CommandExt;\r?\n/m)

// llm.rs: unused binding destructuring in Tool match arm
replaceOnce('core/src/model/llm.rs', 'Message::Tool { name, input, output, .. }', 'Message::Tool { .. }')

// hooks.rs: ToolCallRecord fields are written to a buffer for later round-summary use
replaceOnce('core/src/memory/hooks.rs', 'struct ToolCallRecord {', '#[allow(dead_code)]\nstruct ToolCallRecord {')
