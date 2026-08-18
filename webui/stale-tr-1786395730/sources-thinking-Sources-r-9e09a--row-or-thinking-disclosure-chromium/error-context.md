# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: sources-thinking.spec.ts >> Sources row and thinking disclosure >> idle chat renders no sources row or thinking disclosure
- Location: e2e\sources-thinking.spec.ts:146:3

# Error details

```
TimeoutError: page.waitForSelector: Timeout 10000ms exceeded.
Call log:
  - waiting for locator('.conn-pill') to be visible

```

# Test source

```ts
  48  |                       ok: true,
  49  |                       results: [
  50  |                         {
  51  |                           title: 'OpenSquilla Search Notes',
  52  |                           url: 'https://example.com/opensquilla-search',
  53  |                           domain: 'example.com',
  54  |                           provider: 'tavily',
  55  |                           excerpt: 'Compact citation-ready excerpt.',
  56  |                           fetched: true,
  57  |                         },
  58  |                       ],
  59  |                     }),
  60  |                     status: 'success',
  61  |                   },
  62  |                 ],
  63  |               },
  64  |             ],
  65  |             has_more: false,
  66  |           },
  67  |         }))
  68  |         return
  69  |       }
  70  |       ws.send(JSON.stringify({ type: 'res', id: frame.id, ok: true, payload: {} }))
  71  |     })
  72  |     ws.send(JSON.stringify({ type: 'event', event: 'connect.challenge', payload: {} }))
  73  |   })
  74  | }
  75  | 
  76  | async function seedPersistedSearchSourcesHistory(page: Page) {
  77  |   await page.routeWebSocket(/\/ws$/, ws => {
  78  |     ws.onMessage(message => {
  79  |       let frame: Record<string, unknown>
  80  |       try {
  81  |         frame = JSON.parse(String(message)) as Record<string, unknown>
  82  |       } catch {
  83  |         return
  84  |       }
  85  |       if (frame?.type !== 'req' || frame.id === undefined) return
  86  |       if (frame.method === 'connect') {
  87  |         ws.send(JSON.stringify({ protocol: 3, policy: {} }))
  88  |         return
  89  |       }
  90  |       if (frame?.type === 'req' && frame.method === 'chat.history') {
  91  |         ws.send(JSON.stringify({
  92  |           type: 'res',
  93  |           id: frame.id,
  94  |           ok: true,
  95  |           payload: {
  96  |             messages: [
  97  |               {
  98  |                 role: 'assistant',
  99  |                 text: 'The answer cites the persisted source.',
  100 |                 id: 'msg-e2e-persisted-source',
  101 |                 timestamp: Math.floor(Date.now() / 1000) - 60,
  102 |                 tool_calls: [
  103 |                   {
  104 |                     tool_use_id: 'tool-e2e-persisted-source',
  105 |                     name: 'web_search',
  106 |                     input: { query: 'OpenSquilla persisted source' },
  107 |                     sources: [
  108 |                       {
  109 |                         title: 'OpenSquilla Persisted Source',
  110 |                         url: 'https://example.com/opensquilla-persisted-source',
  111 |                         domain: 'example.com',
  112 |                         provider: 'duckduckgo',
  113 |                         fetched: true,
  114 |                       },
  115 |                     ],
  116 |                     result: JSON.stringify({
  117 |                       ok: true,
  118 |                       results: [
  119 |                         {
  120 |                           title: 'Truncated fallback source',
  121 |                           url: 'https://example.com/opensquilla-persisted-sour…',
  122 |                           domain: 'example.com',
  123 |                           provider: 'duckduckgo',
  124 |                           excerpt: 'Compacted persisted result with a dead-link URL.',
  125 |                           fetched: true,
  126 |                         },
  127 |                       ],
  128 |                     }),
  129 |                     status: 'success',
  130 |                   },
  131 |                 ],
  132 |               },
  133 |             ],
  134 |             has_more: false,
  135 |           },
  136 |         }))
  137 |         return
  138 |       }
  139 |       ws.send(JSON.stringify({ type: 'res', id: frame.id, ok: true, payload: {} }))
  140 |     })
  141 |     ws.send(JSON.stringify({ type: 'event', event: 'connect.challenge', payload: {} }))
  142 |   })
  143 | }
  144 | 
  145 | test.describe('Sources row and thinking disclosure', () => {
  146 |   test('idle chat renders no sources row or thinking disclosure', async ({ page }) => {
  147 |     await page.goto(CONTROL_URL + 'chat/new')
> 148 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
      |                ^ TimeoutError: page.waitForSelector: Timeout 10000ms exceeded.
  149 | 
  150 |     await expect(page.locator('.sources-row')).toHaveCount(0)
  151 |     await expect(page.locator('.thinking-fold')).toHaveCount(0)
  152 |   })
  153 | 
  154 |   test('replayed web_search results render a sources row with links', async ({ page }) => {
  155 |     await seedSearchHistory(page, 'web_search')
  156 |     await page.goto(CONTROL_URL + 'chat?session=' + encodeURIComponent(RESEARCH_SESSION_KEY))
  157 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  158 | 
  159 |     await openFirstAssistantActivity(page)
  160 |     await expect(page.locator('.msg-ai .tool-row[data-op="web.search"]').first()).toBeVisible({ timeout: 10000 })
  161 | 
  162 |     const sourcesRow = page.locator('.msg-ai .sources-row').first()
  163 |     await expect(sourcesRow).toBeVisible({ timeout: 10000 })
  164 |     await expect(sourcesRow.locator('.sources-row__count')).toHaveText('1')
  165 | 
  166 |     await sourcesRow.locator('.sources-row__toggle').click()
  167 |     const link = sourcesRow.locator('.sources-row__link')
  168 |     await expect(link).toHaveAttribute('href', 'https://example.com/opensquilla-search')
  169 |     await expect(sourcesRow.locator('.sources-row__title')).toHaveText('OpenSquilla Search Notes')
  170 |   })
  171 | 
  172 |   test('replayed web_search prefers persisted sources over truncated result URLs', async ({ page }) => {
  173 |     await seedPersistedSearchSourcesHistory(page)
  174 |     await page.goto(CONTROL_URL + 'chat?session=' + encodeURIComponent(RESEARCH_SESSION_KEY))
  175 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  176 | 
  177 |     await openFirstAssistantActivity(page)
  178 |     await expect(page.locator('.msg-ai .tool-row[data-op="web.search"]').first()).toBeVisible({ timeout: 10000 })
  179 | 
  180 |     const sourcesRow = page.locator('.msg-ai .sources-row').first()
  181 |     await expect(sourcesRow).toBeVisible({ timeout: 10000 })
  182 |     await expect(sourcesRow.locator('.sources-row__count')).toHaveText('1')
  183 | 
  184 |     await sourcesRow.locator('.sources-row__toggle').click()
  185 |     const link = sourcesRow.locator('.sources-row__link')
  186 |     await expect(link).toHaveAttribute('href', 'https://example.com/opensquilla-persisted-source')
  187 |     await expect(sourcesRow.locator('.sources-row__title')).toHaveText('OpenSquilla Persisted Source')
  188 |   })
  189 | 
  190 |   test('replayed web_discover results render as discovery without sources row', async ({ page }) => {
  191 |     await seedSearchHistory(page, 'web_discover')
  192 |     await page.goto(CONTROL_URL + 'chat?session=' + encodeURIComponent(RESEARCH_SESSION_KEY))
  193 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  194 | 
  195 |     await openFirstAssistantActivity(page)
  196 |     await expect(page.locator('.msg-ai .tool-row[data-op="web.discover"]').first()).toBeVisible({ timeout: 10000 })
  197 |     await expect(page.locator('.msg-ai .sources-row')).toHaveCount(0)
  198 |   })
  199 | 
  200 |   test('live search turn renders a sources row with real links', async ({ page }) => {
  201 |     test.skip(!LIVE, 'Live gateway test; set OPENSQUILLA_E2E_LIVE=1 to run.')
  202 |     test.setTimeout(240000)
  203 | 
  204 |     await page.goto(CONTROL_URL + 'chat/new')
  205 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  206 | 
  207 |     const textarea = page.locator('.chat-textarea')
  208 |     await textarea.fill('Use your web search tool to find one recent headline about renewable energy, then answer in one sentence.')
  209 |     await page.locator('.chat-send-btn[aria-label="Send"]').click()
  210 | 
  211 |     // The turn runs and completes.
  212 |     const ribbon = page.locator('.stream-activity')
  213 |     await expect(ribbon).toBeVisible({ timeout: 30000 })
  214 |     await expect(ribbon).toHaveCount(0, { timeout: 180000 })
  215 | 
  216 |     // Sources row appears on the finished assistant turn, collapsed.
  217 |     const sourcesRow = page.locator('.msg-ai .sources-row').first()
  218 |     await expect(sourcesRow).toBeVisible({ timeout: 30000 })
  219 |     const toggle = sourcesRow.locator('.sources-row__toggle')
  220 |     await expect(toggle).toHaveAttribute('aria-expanded', 'false')
  221 |     await expect(toggle.locator('.sources-row__count')).toHaveText(/^[1-9]\d*$/)
  222 | 
  223 |     // Expanding reveals real external links.
  224 |     await toggle.click()
  225 |     await expect(toggle).toHaveAttribute('aria-expanded', 'true')
  226 |     const links = sourcesRow.locator('.sources-row__link')
  227 |     expect(await links.count()).toBeGreaterThan(0)
  228 |     for (const href of await links.evaluateAll(nodes => nodes.map(node => node.getAttribute('href') || ''))) {
  229 |       expect(href).toMatch(/^https?:\/\//)
  230 |       // Compacted tool results truncate long strings with a '…' suffix; such
  231 |       // URLs are dead links and must never be rendered as hrefs.
  232 |       expect(href).not.toMatch(/…|%E2%80%A6/i)
  233 |     }
  234 |     const resolvedHrefs = await links.evaluateAll(nodes =>
  235 |       nodes.map(node => (node as HTMLAnchorElement).href),
  236 |     )
  237 |     for (const href of resolvedHrefs) {
  238 |       expect(href).not.toMatch(/…|%E2%80%A6/i)
  239 |     }
  240 |     const rels = await links.evaluateAll(nodes => nodes.map(node => node.getAttribute('rel') || ''))
  241 |     for (const rel of rels) {
  242 |       expect(rel).toContain('noopener')
  243 |       expect(rel).toContain('noreferrer')
  244 |     }
  245 | 
  246 |     // Completed execution activity is compact by default. Reasoning, when
  247 |     // present, lives inside this one disclosure instead of a nested accordion.
  248 |     const activities = page.locator('.msg-ai .assistant-activity')
```