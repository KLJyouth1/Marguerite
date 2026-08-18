# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: compaction-card.spec.ts >> Compaction maintenance card >> idle chat renders no maintenance card
- Location: e2e\compaction-card.spec.ts:7:3

# Error details

```
TimeoutError: page.waitForSelector: Timeout 10000ms exceeded.
Call log:
  - waiting for locator('.conn-pill') to be visible

```

# Test source

```ts
  1   | import { test, expect } from '@playwright/test'
  2   | 
  3   | const CONTROL_URL = '/control/'
  4   | const LIVE = process.env.OPENSQUILLA_E2E_LIVE === '1'
  5   | 
  6   | test.describe('Compaction maintenance card', () => {
  7   |   test('idle chat renders no maintenance card', async ({ page }) => {
  8   |     await page.goto(CONTROL_URL + 'chat/new')
> 9   |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
      |                ^ TimeoutError: page.waitForSelector: Timeout 10000ms exceeded.
  10  | 
  11  |     await expect(page.locator('.chat-compact-status')).toHaveCount(0)
  12  |   })
  13  | 
  14  |   test('live /compact shows the card with elapsed ticking, then settles and dismisses', async ({ page }) => {
  15  |     test.skip(!LIVE, 'Live gateway test; set OPENSQUILLA_E2E_LIVE=1 to run.')
  16  |     test.setTimeout(300000)
  17  | 
  18  |     await page.goto(CONTROL_URL + 'chat/new')
  19  |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  20  | 
  21  |     // Seed the session with one short exchange so compaction has messages.
  22  |     const textarea = page.locator('.chat-textarea')
  23  |     await textarea.fill('Reply with the single word: ok')
  24  |     await page.locator('.chat-send-btn[aria-label="Send"]').click()
  25  |     await expect(page.locator('.msg-ai').first()).toBeVisible({ timeout: 120000 })
  26  |     await expect(page.locator('.work-card')).toHaveCount(0, { timeout: 120000 })
  27  | 
  28  |     // Trigger manual compaction through the slash command path.
  29  |     await textarea.fill('/compact')
  30  |     await page.locator('.chat-send-btn[aria-label="Send"]').click()
  31  | 
  32  |     const card = page.locator('.chat-compact-status')
  33  |     await expect(card).toBeVisible({ timeout: 10000 })
  34  | 
  35  |     // Observe the card until it settles. Fast compactions finish in under a
  36  |     // second — the elapsed chip never leaves 0s and the busy state may never
  37  |     // be sampled — so tick/busy assertions are conditional on duration while
  38  |     // lifecycle assertions stay unconditional.
  39  |     const observed = await page.evaluate(async () => {
  40  |       const t0 = Date.now()
  41  |       const samples: Array<{
  42  |         title: string | null
  43  |         elapsed: string | null
  44  |         busyIndicator: boolean
  45  |         gaugeIndeterminate: boolean
  46  |         gaugeDone: boolean
  47  |         gaugeAriaValueNow: string | null
  48  |         gaugeFillRatio: number | null
  49  |       }> = []
  50  |       while (Date.now() - t0 < 120000) {
  51  |         const cardEl = document.querySelector('.chat-compact-status')
  52  |         const titleEl = document.querySelector('.chat-compact-status__title')
  53  |         const elapsedEl = document.querySelector('.chat-compact-status__elapsed')
  54  |         const gaugeEl = document.querySelector<HTMLElement>('.chat-compact-status__gauge')
  55  |         const gaugeFillEl = document.querySelector<HTMLElement>('.chat-compact-status__gauge-fill')
  56  |         const gaugeWidth = gaugeEl?.getBoundingClientRect().width ?? 0
  57  |         samples.push({
  58  |           title: titleEl ? titleEl.textContent : null,
  59  |           elapsed: elapsedEl ? elapsedEl.textContent : null,
  60  |           busyIndicator: !!document.querySelector(
  61  |             '.chat-compact-status__indicator--busy',
  62  |           ),
  63  |           gaugeIndeterminate: !!document.querySelector(
  64  |             '.chat-compact-status__gauge-fill--indeterminate',
  65  |           ),
  66  |           gaugeDone: !!document.querySelector('.chat-compact-status__gauge-fill--done'),
  67  |           gaugeAriaValueNow: gaugeEl?.getAttribute('aria-valuenow') ?? null,
  68  |           gaugeFillRatio: gaugeFillEl && gaugeWidth > 0
  69  |             ? gaugeFillEl.getBoundingClientRect().width / gaugeWidth
  70  |             : null,
  71  |         })
  72  |         const settled = cardEl !== null
  73  |           && !document.querySelector('.chat-compact-status__indicator--busy')
  74  |         if ((settled || cardEl === null) && samples.length > 2) break
  75  |         await new Promise((resolve) => setTimeout(resolve, 150))
  76  |       }
  77  |       return samples
  78  |     })
  79  | 
  80  |     // Lifecycle: the card rendered a title and reached a terminal state.
  81  |     const titles = observed.map((s) => s.title).filter((t): t is string => t !== null)
  82  |     expect(titles.length).toBeGreaterThan(0)
  83  |     expect(
  84  |       titles.some(
  85  |         (t) => /^(Compacting context|Context compacted|Already within context budget|Context within budget\.?)/.test(t),
  86  |       ),
  87  |     ).toBe(true)
  88  |     const last = observed[observed.length - 1]
  89  |     expect(last.busyIndicator).toBe(false)
  90  |     expect(
  91  |       last.title === null
  92  |       || /^(Context compacted|Already within context budget|Context within budget\.?)/.test(last.title),
  93  |     ).toBe(true)
  94  | 
  95  |     const busySamples = observed.filter((s) => s.busyIndicator)
  96  |     if (busySamples.length > 0) {
  97  |       // While busy the operation exposes no fake percentage: a short
  98  |       // indeterminate segment moves across the track and omits aria-valuenow.
  99  |       expect(busySamples.some((s) => s.gaugeIndeterminate)).toBe(true)
  100 |       expect(busySamples.every((s) => s.gaugeAriaValueNow === null)).toBe(true)
  101 |       expect(
  102 |         busySamples.some(
  103 |           (s) => s.gaugeFillRatio !== null
  104 |             && s.gaugeFillRatio >= 0.38
  105 |             && s.gaugeFillRatio <= 0.46,
  106 |         ),
  107 |       ).toBe(true)
  108 |     }
  109 |     // Tick proof: ~2.4s of busy time (16 samples at 150ms) must show at
```