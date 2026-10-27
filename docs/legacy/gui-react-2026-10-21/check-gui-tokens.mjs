// GUI 设计 token 纪律守卫。
//
// 为什么需要它（2026-10-21 审计）：`theme.css` 定义了 `--status-error/warning/success`，
// 且深色主题（data-theme="ink"）把它们重新调过（--status-error: #EF4444 → #B23A2E）。
// 但面板里曾有 27 处直接写死 `#ef4444` / `#f59e0b` / `#22c55e`：
//   · 深色主题下用的是浅色主题的值，对比度与观感都不对；
//   · 同一语义散落 27 份，改一次必然漏。
// 手工替换过一次不等于以后不会再犯——所以把它变成门禁。
//
// 允许的情况（白名单必须给理由，不能靠注释宽恕）：
//   · `var(--token, #fallback)` 里的兜底色：变量缺失时才生效，属正常防御；
//   · 图形/数据可视化里的**分类色**（GraphPanel 的边类型色）：它们不是状态语义，
//     不属于主题 token 的职责范围，改用状态色反而会误导。

import { readFileSync, readdirSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const here = dirname(fileURLToPath(import.meta.url))
const srcDir = resolve(here, "..", "src")
const panelsDir = join(srcDir, "panels")

// 状态色字面量 → 应该用的 token
const STATUS_LITERALS = {
  "#ef4444": "--status-error",
  "#EF4444": "--status-error",
  "#f59e0b": "--status-warning",
  "#F59E0B": "--status-warning",
  "#22c55e": "--status-success",
  "#22C55E": "--status-success",
  "#10b981": "--status-success",
  "#10B981": "--status-success",
}

// 同一批颜色的 rgb()/rgba() 写法同样违规：它们**完全不跟随主题**。
// 漏掉这一条就会出现"hex 修干净了、rgba 还留着"的半修。
const STATUS_RGB = {
  "239,68,68": "--status-error",
  "239, 68, 68": "--status-error",
  "34,197,94": "--status-success",
  "34, 197, 94": "--status-success",
  "16,185,129": "--status-success",
  "16, 185, 129": "--status-success",
  "245,158,11": "--status-warning",
  "245, 158, 11": "--status-warning",
}

// 允许保留字面量的文件（每个都要能说出为什么）
const ALLOW = new Map([
  // GraphPanel 的边类型色是**数据分类**色彩（co_occurs / is_a / uses ...），
  // 不是状态语义；硬套状态色会让"uses 边"和"错误"同色。
  ["GraphPanel.tsx", "知识图谱边类型分类色，非状态语义"],
])

let failed = false
const hits = []

function scanFile(dir, file) {
  const full = join(dir, file)
  const lines = readFileSync(full, "utf8").split("\n")
  lines.forEach((line, i) => {
    // 跳过注释行：说明文字里提到色值不是违规
    const trimmed = line.trim()
    if (trimmed.startsWith("//") || trimmed.startsWith("*") || trimmed.startsWith("/*")) return
    for (const [lit, token] of Object.entries(STATUS_LITERALS)) {
      if (!line.includes(lit)) continue
      // `var(--x, #ef4444)` 形态是合法兜底
      const asFallback = new RegExp(`var\\(\\s*--[a-z-]+\\s*,\\s*${lit.replace("#", "#")}\\s*\\)`).test(line)
      if (asFallback) continue
      hits.push({ file, line: i + 1, literal: lit, shouldUse: token })
    }
    for (const [rgb, token] of Object.entries(STATUS_RGB)) {
      if (!line.includes(rgb)) continue
      // `color-mix(... var(--status-success) ...)` 里不会出现裸 rgb 三元组，
      // 故此分支无需兜底判断：命中即违规。
      hits.push({ file, line: i + 1, literal: `rgb(${rgb})`, shouldUse: token })
    }
  })
}

for (const file of readdirSync(panelsDir)) {
  if (!file.endsWith(".tsx")) continue
  if (ALLOW.has(file)) continue
  scanFile(panelsDir, file)
}
// 组件目录同样适用（Notice 之外不该再出现状态色字面量）
const componentsDir = join(srcDir, "components")
try {
  for (const file of readdirSync(componentsDir)) {
    if (!file.endsWith(".tsx")) continue
    // Notice.tsx 里 `var(--status-error, #ef4444)` 是兜底，会被 asFallback 放行
    scanFile(componentsDir, file)
  }
} catch {
  // components 目录不存在不算失败（结构变化时不该误判）
}

if (hits.length) {
  failed = true
  console.error("[check-gui-tokens] 状态色被写死，应改用主题 token：")
  for (const h of hits) {
    console.error(`  ${h.file}:${h.line}  ${h.literal} → var(${h.shouldUse})`)
  }
}

if (failed) {
  console.error("[check-gui-tokens] FAILED")
  process.exit(1)
}
console.log(
  `[check-gui-tokens] OK — 状态色全部走主题 token（${ALLOW.size} 个文件按分类色白名单放行）`,
)
