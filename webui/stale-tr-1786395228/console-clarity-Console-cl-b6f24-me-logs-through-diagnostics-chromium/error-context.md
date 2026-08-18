# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: console-clarity.spec.ts >> Console clarity >> Overview stays focused on status and routes runtime logs through diagnostics
- Location: e2e\console-clarity.spec.ts:50:3

# Error details

```
Error: expect(locator).toHaveText(expected) failed

Locator: getByRole('navigation', { name: 'Overview' }).getByRole('link')
Timeout: 5000ms
- Expected  - 4
+ Received  + 1

- Array [
-   "Status",
-   "Usage",
- ]
+ Array []

Call log:
  - Expect "toHaveText" with timeout 5000ms
  - waiting for getByRole('navigation', { name: 'Overview' }).getByRole('link')
    14 × locator resolved to 0 elements

```

# Page snapshot

```yaml
- generic [ref=e2]:
  - navigation "Primary" [ref=e3]:
    - generic [ref=e4]:
      - generic [ref=e5]:
        - img [ref=e6]
        - generic [ref=e7]: Marguerite
      - button "Collapse sidebar" [expanded] [ref=e8] [cursor=pointer]:
        - img [ref=e10]
    - navigation "Control navigation" [ref=e12]:
      - button "New task" [ref=e13] [cursor=pointer]:
        - img [ref=e15]
        - generic [ref=e16]: New task
      - link "Skills & Channels" [ref=e17] [cursor=pointer]:
        - /url: /control/skills
        - img [ref=e19]
        - generic [ref=e21]: Skills & Channels
      - link "Cron" [ref=e22] [cursor=pointer]:
        - /url: /control/cron
        - img [ref=e24]
        - generic [ref=e27]: Cron
      - link "View usage" [ref=e28] [cursor=pointer]:
        - /url: /control/usage
        - img [ref=e30]
        - generic [ref=e31]: View usage
    - button "Settings" [ref=e33] [cursor=pointer]:
      - img [ref=e35]
      - generic [ref=e38]: Settings
  - separator "Resize sidebar" [ref=e39]:
    - status [ref=e40]
  - generic [ref=e41]:
    - banner:
      - generic [ref=e42]:
        - button "Manage gateway connection" [ref=e43] [cursor=pointer]: Disconnected
        - button "Language" [ref=e45] [cursor=pointer]:
          - img [ref=e47]
          - generic [ref=e50]: English
        - button "Theme" [ref=e52] [cursor=pointer]:
          - img [ref=e54]
    - main [ref=e57]:
      - generic [ref=e58]:
        - generic [ref=e59]:
          - navigation "View usage" [ref=e60]:
            - link "Usage" [ref=e61] [cursor=pointer]:
              - /url: /control/usage
              - img [ref=e63]
              - generic [ref=e64]: Usage
            - link "Overview" [ref=e65] [cursor=pointer]:
              - /url: /control/overview
              - img [ref=e67]
              - generic [ref=e70]: Overview
          - button "Support & diagnostics" [ref=e73] [cursor=pointer]:
            - img [ref=e75]
            - generic [ref=e78]: Support & diagnostics
            - img [ref=e80]
        - generic [ref=e83]:
          - region "Health summary" [ref=e84]:
            - img [ref=e87]
            - strong [ref=e90]: Checking
            - button "Refresh" [ref=e92] [cursor=pointer]:
              - img [ref=e94]
              - generic [ref=e97]: Refresh
          - region "Status" [ref=e98]:
            - button "Total tokens — —" [ref=e99] [cursor=pointer]:
              - generic [ref=e100]: Total tokens
              - generic [ref=e101]: —
              - generic [ref=e102]: —
            - button "Total sessions — view all →" [ref=e103] [cursor=pointer]:
              - generic [ref=e104]: Total sessions
              - generic [ref=e105]: —
              - generic [ref=e106]: view all →
            - generic [ref=e107]:
              - generic [ref=e108]: Uptime
              - generic [ref=e109]: —
              - generic [ref=e110]: —
          - region "Environment" [ref=e111]:
            - generic [ref=e112]: v— · —
          - region "Health findings" [ref=e113]:
            - article [ref=e114]: Loading health report
  - status [ref=e115]
```

# Test source

```ts
  1   | import { test, expect, type Page } from '@playwright/test'
  2   | 
  3   | const CONTROL_URL = '/control/'
  4   | 
  5   | async function openControl(page: Page, path = '') {
  6   |   await page.goto(CONTROL_URL + path)
  7   |   await page.waitForSelector('.conn-pill', { timeout: 10000 })
  8   | }
  9   | 
  10  | const settingsDialog = (page: Page) => page.getByRole('dialog', { name: 'Settings' })
  11  | 
  12  | test.describe('Console clarity', () => {
  13  |   // The DEV-only parts/fold parity check logs `[live-turn parity]` on any
  14  |   // fold/key divergence between message.parts and the rendered timeline. Treat
  15  |   // it as a hard failure so a regression is caught in CI, not eyeballed.
  16  |   let parityErrors: string[]
  17  | 
  18  |   test.beforeEach(({ page }) => {
  19  |     parityErrors = []
  20  |     page.on('console', msg => {
  21  |       if (msg.type() === 'error' && msg.text().includes('[live-turn parity]')) {
  22  |         parityErrors.push(msg.text())
  23  |       }
  24  |     })
  25  |   })
  26  | 
  27  |   test.afterEach(() => {
  28  |     expect(parityErrors, 'live-turn parts/fold parity check reported a divergence').toEqual([])
  29  |   })
  30  | 
  31  |   test('flat navigation removes the Console fold and keeps Settings distinct', async ({ page }) => {
  32  |     await openControl(page)
  33  | 
  34  |     const settingsRow = page.locator('.sidebar-foot .sidebar-fn-item')
  35  |     await expect(settingsRow).toHaveAttribute('data-icon', 'settings')
  36  |     await expect(page.locator('.sidebar-nav-group-toggle')).toHaveCount(0)
  37  |     await expect(page.locator('.sidebar-core .sidebar-fn-label')).toHaveText([
  38  |       'Overview', 'Skills & Channels', 'Cron',
  39  |     ])
  40  |   })
  41  | 
  42  |   test('/health deep link redirects to /overview with the readiness report inline', async ({ page }) => {
  43  |     await openControl(page, 'health')
  44  | 
  45  |     await expect(page).toHaveURL(/\/overview$/)
  46  |     await expect(page.locator('#overview-health')).toBeVisible()
  47  |     await expect(page.locator('section[aria-label="Health findings"]')).toBeVisible()
  48  |   })
  49  | 
  50  |   test('Overview stays focused on status and routes runtime logs through diagnostics', async ({ page }) => {
  51  |     await openControl(page, 'overview')
  52  | 
  53  |     const hub = page.getByRole('navigation', { name: 'Overview' })
> 54  |     await expect(hub.getByRole('link')).toHaveText(['Status', 'Usage'])
      |                                         ^ Error: expect(locator).toHaveText(expected) failed
  55  |     await expect(page.locator('.ov-grid')).toHaveCount(0)
  56  |     await expect(page.locator('.ov-recent')).toHaveCount(0)
  57  |     await expect(page.locator('.ov-event-log')).toHaveCount(0)
  58  | 
  59  |     await page.getByRole('button', { name: 'Support & diagnostics' }).click()
  60  |     await page.getByRole('menuitem', { name: /View runtime logs/ }).click()
  61  |     await expect(page).toHaveURL(/\/logs$/)
  62  |     await expect(page.getByRole('heading', { name: 'Logs', level: 1 })).toBeVisible()
  63  |     await expect(page.locator('.sidebar-core').getByRole('link', { name: 'Overview' }))
  64  |       .toHaveClass(/is-active/)
  65  | 
  66  |     const breadcrumb = page.getByRole('navigation', { name: 'Breadcrumb' })
  67  |     await expect(breadcrumb.getByRole('link', { name: 'Overview', exact: true }))
  68  |       .toHaveAttribute('href', '/control/overview')
  69  |     await breadcrumb.getByRole('link', { name: 'Overview', exact: true }).click()
  70  |     await expect(page).toHaveURL(/\/overview$/)
  71  |     await page.goBack()
  72  |     await expect(page).toHaveURL(/\/logs$/)
  73  | 
  74  |     await page.getByRole('button', { name: 'Support & diagnostics' }).click()
  75  |     await expect(page.getByRole('menuitem', { name: /View runtime logs/ })).toHaveCount(0)
  76  |     await page.goBack()
  77  |     await expect(page).toHaveURL(/\/overview$/)
  78  |   })
  79  | 
  80  |   test('Overview and Logs stay within the target responsive viewports', async ({ page }) => {
  81  |     const scenarios = [
  82  |       { width: 320, height: 800, locale: 'zh-Hans', path: 'overview' },
  83  |       { width: 390, height: 844, locale: 'en', path: 'logs' },
  84  |       { width: 768, height: 900, locale: 'zh-Hans', path: 'logs' },
  85  |       { width: 1440, height: 1000, locale: 'en', path: 'overview' },
  86  |     ] as const
  87  | 
  88  |     for (const scenario of scenarios) {
  89  |       await page.setViewportSize({ width: scenario.width, height: scenario.height })
  90  |       await page.goto(CONTROL_URL)
  91  |       await page.evaluate((locale) => {
  92  |         localStorage.setItem('marguerite-locale', locale)
  93  |       }, scenario.locale)
  94  |       await openControl(page, scenario.path)
  95  | 
  96  |       const overflow = await page.evaluate(() =>
  97  |         document.documentElement.scrollWidth - document.documentElement.clientWidth)
  98  |       expect(overflow).toBeLessThanOrEqual(0)
  99  | 
  100 |       if (scenario.path === 'overview' && scenario.width <= 320) {
  101 |         const tabs = await page.locator('.route-hub__tabs').boundingBox()
  102 |         const actions = await page.locator('.route-hub__actions').boundingBox()
  103 |         expect(tabs).not.toBeNull()
  104 |         expect(actions).not.toBeNull()
  105 |         expect(tabs!.x + tabs!.width).toBeLessThanOrEqual(actions!.x)
  106 |       }
  107 |     }
  108 |   })
  109 | 
  110 |   test('Channels aligns its refresh action with the hub tabs only on desktop', async ({ page }) => {
  111 |     for (const scenario of [
  112 |       { width: 900, height: 800, inline: true },
  113 |       { width: 390, height: 844, inline: false },
  114 |     ]) {
  115 |       await page.setViewportSize({ width: scenario.width, height: scenario.height })
  116 |       await openControl(page, 'channels')
  117 | 
  118 |       const tabs = await page.locator('.route-hub__tabs').boundingBox()
  119 |       const actions = await page.locator('.ch-stage__actions').boundingBox()
  120 |       expect(tabs).not.toBeNull()
  121 |       expect(actions).not.toBeNull()
  122 | 
  123 |       if (scenario.inline) {
  124 |         const tabsCenter = tabs!.y + tabs!.height / 2
  125 |         const actionsCenter = actions!.y + actions!.height / 2
  126 |         expect(Math.abs(tabsCenter - actionsCenter)).toBeLessThanOrEqual(2)
  127 |       } else {
  128 |         expect(actions!.y).toBeGreaterThanOrEqual(tabs!.y + tabs!.height)
  129 |       }
  130 | 
  131 |       const overflow = await page.evaluate(() =>
  132 |         document.documentElement.scrollWidth - document.documentElement.clientWidth)
  133 |       expect(overflow).toBeLessThanOrEqual(0)
  134 |     }
  135 |   })
  136 | 
  137 |   test('Status impact count jumps to the inline readiness report', async ({ page }) => {
  138 |     await openControl(page, 'overview')
  139 | 
  140 |     await page.locator('.ov-count').first().click()
  141 |     // Still on Overview — the card scrolls instead of navigating away.
  142 |     await expect(page).toHaveURL(/\/overview$/)
  143 |     await expect(page.locator('#overview-health')).toBeInViewport()
  144 |   })
  145 | 
  146 |   test('Agents header link opens the Settings modal', async ({ page }) => {
  147 |     await openControl(page, 'agents')
  148 | 
  149 |     await page.locator('.ag-stage__actions').getByRole('button', { name: 'open settings' }).click()
  150 |     await expect(settingsDialog(page)).toBeVisible()
  151 |   })
  152 | })
  153 | 
```