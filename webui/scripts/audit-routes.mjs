// 逐路由巡检：真实浏览器打开每个路由，采集「会不会崩」的客观证据。
//
// 为什么需要它：静态门禁（vue-tsc / 守卫 / 单测）只能证明"代码自洽"，证明不了
// "这个页面在真浏览器里打得开"。缺失的正是这一层：某页在 RPC 失败/空数据时会白屏，
// 而所有门禁全绿。本脚本把这件事变成可重复的机器判定。
//
// 判定口径（不靠肉眼）：
//   · pageerror        —— 未捕获异常，一票否决
//   · console.error    —— 运行时错误（过滤已知噪声，见 NOISE）
//   · 空白页           —— #app 文本 < 8 字符且无 svg/canvas/img 结构
//   · 无内容容器       —— 找不到任何带 role/class 的主内容节点
//
// 用法：node scripts/audit-routes.mjs [--base http://127.0.0.1:18791] [--out dir]

import { chromium } from 'playwright'
import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const argv = process.argv.slice(2)
const argOf = (name, fallback) => {
  const i = argv.indexOf(name)
  return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback
}
const base = argOf('--base', 'http://127.0.0.1:18791').replace(/\/$/, '')
const outDir = argOf('--out', 'route-audit')
const token = argOf('--token', '')

// 与 router/sharedRoutes.ts 同源的路由清单。刻意手写而非解析源码：
// 解析器一旦跟不上源码形态就会静默漏页，而这正是本脚本要防的事。
const ROUTES = [
  '/splash', '/', '/chat', '/chat/new', '/task', '/sessions', '/overview', '/usage',
  '/logs', '/approvals', '/agents', '/skills', '/channels', '/cron', '/changelog',
  '/health', '/computer-use', '/fleet', '/evolve', '/plugins', '/graph', '/poker',
  '/games', '/brain', '/engine', '/ops', '/expert-center', '/editor',
  '/browser-cockpit', '/terminal', '/ssh-collab', '/capabilities-hub',
  '/control-center', '/local-models', '/knowledge', '/island', '/island-game',
  '/pet', '/wallpaper', '/device-twin',
]

// 已知噪声：这些错误与"页面能不能用"无关，计入会淹没真问题。
// 每条都写明为什么可以忽略，避免"为了报告好看而屏蔽"。
const NOISE = [
  /Failed to load resource.*favicon/i,
  /ResizeObserver loop/i,
  /\[vite\]/i,
  /Download the Vue Devtools/i,
]

const isNoise = (text) => NOISE.some((re) => re.test(text))

const slug = (route) => (route === '/' ? 'root' : route.replace(/^\//, '').replace(/\//g, '_'))

async function main() {
  mkdirSync(outDir, { recursive: true })
  const browser = await chromium.launch()
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 } })
  const results = []

  for (const route of ROUTES) {
    const page = await context.newPage()
    const consoleErrors = []
    const pageErrors = []
    const failedRequests = []
    page.on('console', (msg) => {
      if (msg.type() !== 'error') return
      const text = msg.text()
      if (!isNoise(text)) consoleErrors.push(text.slice(0, 300))
    })
    page.on('pageerror', (err) => pageErrors.push(String(err && err.message ? err.message : err).slice(0, 300)))
    page.on('requestfailed', (req) => {
      const url = req.url()
      if (isNoise(url)) return
      failedRequests.push(`${req.method()} ${url.slice(0, 160)} :: ${req.failure()?.errorText ?? ''}`)
    })

    const url = `${base}/control${route}${token ? `?token=${encodeURIComponent(token)}` : ''}`
    let status = 'ok'
    let detail = ''
    let textLen = 0
    let nodeCount = 0
    try {
      const resp = await page.goto(url, { waitUntil: 'networkidle', timeout: 45_000 })
      if (resp && resp.status() >= 400) {
        status = 'http-error'
        detail = `HTTP ${resp.status()}`
      }
      // 给懒加载路由与首屏动画留出稳定时间：立刻取 innerText 会把"还在加载"误判成白屏
      await page.waitForTimeout(1200)
      const probe = await page.evaluate(() => {
        const app = document.querySelector('#app') || document.body
        const text = (app.innerText || '').replace(/\s+/g, '')
        const structural = app.querySelectorAll('svg, canvas, img, input, textarea, button, [role], table').length
        return { textLen: text.length, structural, nodes: app.querySelectorAll('*').length }
      })
      textLen = probe.textLen
      nodeCount = probe.nodes
      if (textLen < 8 && probe.structural === 0) {
        status = 'blank'
        detail = `app 文本 ${probe.textLen} 字、结构节点 0`
      } else if (probe.nodes < 12) {
        status = 'thin'
        detail = `仅 ${probe.nodes} 个节点，疑似未挂载`
      }
    } catch (err) {
      status = 'nav-failed'
      detail = String(err && err.message ? err.message : err).slice(0, 200)
    }

    await page.screenshot({ path: join(outDir, `${slug(route)}.png`), fullPage: false }).catch(() => {})
    results.push({
      route,
      status,
      detail,
      textLen,
      nodeCount,
      pageErrors,
      consoleErrors: consoleErrors.slice(0, 6),
      failedRequests: failedRequests.slice(0, 6),
    })
    await page.close()
  }

  await browser.close()

  const bad = results.filter((r) => r.status !== 'ok' || r.pageErrors.length || r.consoleErrors.length)
  const summary = {
    base,
    total: results.length,
    clean: results.length - bad.length,
    blank: results.filter((r) => r.status === 'blank').length,
    navFailed: results.filter((r) => r.status === 'nav-failed').length,
    httpError: results.filter((r) => r.status === 'http-error').length,
    thin: results.filter((r) => r.status === 'thin').length,
    withPageError: results.filter((r) => r.pageErrors.length).length,
    withConsoleError: results.filter((r) => r.consoleErrors.length).length,
    withFailedRequest: results.filter((r) => r.failedRequests.length).length,
  }
  writeFileSync(join(outDir, 'report.json'), JSON.stringify({ summary, results }, null, 2), 'utf8')

  console.log(JSON.stringify(summary, null, 2))
  console.log('\n--- 有问题的路由 ---')
  for (const r of bad) {
    console.log(`\n[${r.status}] ${r.route} ${r.detail}`)
    for (const e of r.pageErrors) console.log(`  pageerror: ${e}`)
    for (const e of r.consoleErrors) console.log(`  console:   ${e}`)
    for (const e of r.failedRequests) console.log(`  request:   ${e}`)
  }
}

main().catch((err) => {
  console.error('AUDIT_FATAL', err)
  process.exit(1)
})
