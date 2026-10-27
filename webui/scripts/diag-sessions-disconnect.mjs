// 诊断：抓 `[useSessions] sessions.list error: Disconnected` 的**真实调用栈**。
// 静态读代码只能推到"只可能来自 disconnect()"，再往下就是猜。用 CDP 的
// Runtime.consoleAPICalled.stackTrace 拿到 async 链路，避免猜错方向。

import { chromium } from 'playwright'

const base = process.argv[2] || 'http://127.0.0.1:18791'
const token = process.argv[3] || ''
const rounds = Number(process.argv[4] || 4)

const url = `${base}/control/channels${token ? `?token=${encodeURIComponent(token)}` : ''}`

const browser = await chromium.launch()
let hits = 0

for (let i = 0; i < rounds; i += 1) {
  const context = await browser.newContext()
  const page = await context.newPage()
  const cdp = await context.newCDPSession(page)
  await cdp.send('Runtime.enable')

  const frames = []
  cdp.on('Runtime.consoleAPICalled', (ev) => {
    if (ev.type !== 'error') return
    const text = (ev.args || [])
      .map((a) => String(a.value ?? a.description ?? ''))
      .join(' ')
    if (!text.includes('sessions.list')) return
    hits += 1
    const st = ev.stackTrace
    if (st && Array.isArray(st.callFrames)) {
      for (const f of st.callFrames.slice(0, 14)) {
        frames.push(`    ${f.functionName || '(anon)'} @ ${String(f.url).split('/').pop()}:${f.lineNumber + 1}`)
      }
    }
  })

  await page.goto(url, { waitUntil: 'networkidle', timeout: 45000 })
  await page.waitForTimeout(2500)

  if (frames.length) {
    console.log(`\n=== round ${i + 1}: HIT ===`)
    console.log(frames.join('\n'))
  } else {
    console.log(`round ${i + 1}: clean`)
  }
  await context.close()
}

console.log(`\nhits=${hits}/${rounds}`)
await browser.close()
