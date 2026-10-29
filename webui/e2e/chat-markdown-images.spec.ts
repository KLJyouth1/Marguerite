import { expect, test, type Page } from '@playwright/test'

const CONTROL_URL = '/control/'
const SESSION_KEY = 'agent:main:webchat:e2e-markdown-images'

// Assistant markdown used to lose every image: DOMPurify was configured without
// `<img>`, so `![diagram](https://…)` silently rendered as nothing. happy-dom
// cannot exercise DOMPurify's tree walk (verified: only the root node reaches
// the hooks), so this contract is proven in real Chromium — the same engine the
// desktop/web console runs — instead of in a unit test that would be vacuous.
async function seedAssistantMarkdown(page: Page) {
  await page.route('**/api/approvals', route => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ pending: [] }),
  }))

  await page.routeWebSocket(/\/ws$/, ws => {
    ws.send(JSON.stringify({ type: 'event', event: 'connect.challenge', payload: {} }))
    ws.onMessage(message => {
      try {
        const frame = JSON.parse(String(message))
        if (frame?.type !== 'req') return
        if (frame.method === 'connect') {
          ws.send(JSON.stringify({
            protocol: 3,
            policy: { tick_interval_ms: 30000 },
          }))
          return
        }

        const payloads: Record<string, unknown> = {
          'agents.list': { agents: [] },
          'chat.history': {
            messages: [
              {
                role: 'assistant',
                text: [
                  'Remote image: ![diagram](https://example.com/a.png)',
                  'Script source: ![bad](javascript:alert(1))',
                  'Raw tag: <img src="https://example.com/raw.png" onerror="alert(1)">',
                ].join('\n\n'),
                id: 'msg-markdown-images',
                timestamp: Math.floor(Date.now() / 1000) - 30,
              },
            ],
            has_more: false,
          },
          'commands.list_for_surface': { commands: [] },
          'config.get': {
            squilla_router: { enabled: false, rollout_phase: 'observe', tiers: {} },
            permissions: {},
            skills: {},
          },
          'sessions.list': { sessions: [], has_more: false },
          'sessions.messages.subscribe': {
            subscribed: true,
            replay_complete: true,
            current_stream_seq: 0,
            run_status: 'idle',
          },
          'usage.status': { sessions: [] },
        }

        ws.send(JSON.stringify({
          type: 'res',
          id: frame.id,
          ok: true,
          payload: payloads[String(frame.method)] ?? {},
        }))
      } catch {}
    })
  })
}

test('assistant markdown keeps safe images and drops script-bearing ones', async ({ page }) => {
  await seedAssistantMarkdown(page)
  await page.goto(CONTROL_URL + 'chat?session=' + encodeURIComponent(SESSION_KEY))
  await page.waitForSelector('.conn-pill', { timeout: 10000 })

  const markdown = page.locator('.msg-ai-text').first()
  await expect(markdown).toContainText('Remote image:')

  const remote = markdown.locator('img[src="https://example.com/a.png"]')
  await expect(remote).toHaveCount(1)
  await expect(remote).toHaveAttribute('alt', 'diagram')
  await expect(remote).toHaveAttribute('loading', 'lazy')
  await expect(remote).toHaveAttribute('decoding', 'async')
  await expect(remote).toHaveAttribute('referrerpolicy', 'no-referrer')

  const raw = markdown.locator('img[src="https://example.com/raw.png"]')
  await expect(raw).toHaveCount(1)
  expect(await raw.evaluate(node => node.hasAttribute('onerror'))).toBe(false)

  // No element may carry a javascript: source, whether it came from markdown
  // image syntax or from a raw tag.
  await expect(markdown.locator('img[src^="javascript:"]')).toHaveCount(0)
  await expect(markdown.locator('[src^="javascript:"]')).toHaveCount(0)
})
