# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: fold-live-turn.spec.ts >> Fold-authoritative live turn >> absent localStorage key resolves to the fold (ON) by default
- Location: e2e\fold-live-turn.spec.ts:52:3

# Error details

```
TimeoutError: page.waitForSelector: Timeout 10000ms exceeded.
Call log:
  - waiting for locator('.conn-pill') to be visible

```

# Test source

```ts
  1   | import { test, expect, type Page } from '@playwright/test'
  2   | 
  3   | // fold-authoritative live render proof. The fold is now the default live
  4   | // render source (the activity timeline/artifacts/live-thinking project from the
  5   | // folded event log); only `marguerite.chat.foldLiveTurn=0` forces the legacy
  6   | // refs back. This spec pins the flag to '1' (any non-'0' value is ON) so it stays
  7   | // a deterministic ON-path proof regardless of the default, drives the same real
  8   | // live-stream path the legacy live specs drive, and proves the ON path renders
  9   | // correctly with NO `[live-turn parity]` divergence.
  10  | //
  11  | // Two gates apply, exactly like the other live specs:
  12  | //   - It only runs against a real gateway (OPENSQUILLA_E2E_LIVE=1).
  13  | //   - The flag only flips the consumer in a non-DEV build (DEV is pinned to
  14  | //     SHADOW). The orchestrator builds the bundle, then runs this with the flag.
  15  | const CONTROL_URL = '/control/'
  16  | const LIVE = process.env.OPENSQUILLA_E2E_LIVE === '1'
  17  | const FOLD_FLAG_KEY = 'marguerite.chat.foldLiveTurn'
  18  | 
  19  | // Pin the ON flag before any page script runs so useChatTurnLog resolves
  20  | // useReducer === true at composable init (the fold becomes authoritative).
  21  | async function forceFoldAuthoritative(page: Page) {
  22  |   await page.addInitScript(([key]) => {
  23  |     try {
  24  |       window.localStorage.setItem(key, '1')
  25  |     } catch {
  26  |       // localStorage can be unavailable in some sandboxes; the flag simply
  27  |       // stays OFF there and the run falls back to the legacy render.
  28  |     }
  29  |   }, [FOLD_FLAG_KEY])
  30  | }
  31  | 
  32  | test.describe('Fold-authoritative live turn', () => {
  33  |   // Reuse the console-clarity hard-fail: any `[live-turn parity]` error means
  34  |   // the fold diverged from legacy. With the fold authoritative this is the
  35  |   // deterministic guard that the ON path is byte-faithful to legacy.
  36  |   let parityErrors: string[]
  37  | 
  38  |   test.beforeEach(async ({ page }) => {
  39  |     parityErrors = []
  40  |     page.on('console', msg => {
  41  |       if (msg.type() === 'error' && msg.text().includes('[live-turn parity]')) {
  42  |         parityErrors.push(msg.text())
  43  |       }
  44  |     })
  45  |     await forceFoldAuthoritative(page)
  46  |   })
  47  | 
  48  |   test.afterEach(() => {
  49  |     expect(parityErrors, 'fold-authoritative live render diverged from legacy').toEqual([])
  50  |   })
  51  | 
  52  |   test('absent localStorage key resolves to the fold (ON) by default', async ({ browser }) => {
  53  |     // A context WITHOUT the init script leaves the key unset. Post default-flip an
  54  |     // absent key resolves to ON (the fold is authoritative); only an explicit '0'
  55  |     // forces legacy. The key stays null — the ON default needs no key written.
  56  |     const context = await browser.newContext()
  57  |     const page = await context.newPage()
  58  |     await page.goto(CONTROL_URL + 'chat/new')
> 59  |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
      |                ^ TimeoutError: page.waitForSelector: Timeout 10000ms exceeded.
  60  |     const flag = await page.evaluate(key => window.localStorage.getItem(key), FOLD_FLAG_KEY)
  61  |     expect(flag).toBeNull()
  62  |     await context.close()
  63  |   })
  64  | 
  65  |   test('fold drives the flat activity timeline and tool rows for a live search run', async ({ page }) => {
  66  |     test.skip(!LIVE, 'Live gateway test; set OPENSQUILLA_E2E_LIVE=1 to run.')
  67  |     test.setTimeout(240000)
  68  | 
  69  |     await page.goto(CONTROL_URL + 'chat/new')
  70  |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  71  | 
  72  |     // Confirm the flag took effect for this build (a DEV build pins SHADOW and
  73  |     // never flips ON; the orchestrator runs this against a built bundle).
  74  |     const flag = await page.evaluate(key => window.localStorage.getItem(key), FOLD_FLAG_KEY)
  75  |     expect(flag).toBe('1')
  76  | 
  77  |     const textarea = page.locator('.chat-textarea')
  78  |     await textarea.fill('Use your web search tool to find one recent headline about space exploration, then answer in one sentence.')
  79  |     await page.locator('.chat-send-btn[aria-label="Send"]').click()
  80  | 
  81  |     const liveActivity = page.locator('.assistant-activity--live')
  82  |     await expect(liveActivity).toBeVisible({ timeout: 30000 })
  83  |     await expect(page.locator('.work-card')).toHaveCount(0)
  84  | 
  85  |     const sawChecklistRow = await page.evaluate(async () => {
  86  |       const t0 = Date.now()
  87  |       while (Date.now() - t0 < 180000) {
  88  |         const rows = document.querySelectorAll('.assistant-activity--live .tool-timeline--checklist .tool-row').length
  89  |         if (rows > 0) return true
  90  |         if (document.querySelector('.assistant-activity--live') === null) return false
  91  |         await new Promise(resolve => setTimeout(resolve, 150))
  92  |       }
  93  |       return false
  94  |     })
  95  |     expect(sawChecklistRow).toBe(true)
  96  | 
  97  |     // Run completes: live activity collapses, the transcript keeps the rows.
  98  |     await expect(liveActivity).toHaveCount(0, { timeout: 180000 })
  99  |     const activity = page.locator('.msg-ai .assistant-activity').first()
  100 |     await expect(activity).toBeVisible()
  101 |     const summary = activity.locator('.assistant-activity__summary')
  102 |     await expect(summary).toHaveAttribute('aria-expanded', 'false')
  103 |     await summary.press('Enter')
  104 |     await expect(summary).toHaveAttribute('aria-expanded', 'true')
  105 |     const searchRow = page.locator('.msg-ai .tool-row[data-op="web.search"]').first()
  106 |     await expect(searchRow).toBeVisible()
  107 |     await expect(searchRow).toHaveAttribute('aria-expanded', 'false')
  108 |   })
  109 | })
  110 | 
```