# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: console-clarity.spec.ts >> Console clarity >> Status impact count jumps to the inline readiness report
- Location: e2e\console-clarity.spec.ts:137:3

# Error details

```
Test timeout of 30000ms exceeded.
```

```
Error: locator.click: Test timeout of 30000ms exceeded.
Call log:
  - waiting for locator('.ov-count').first()

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
    - generic "Recent tasks" [ref=e32]:
      - generic [ref=e33]:
        - generic [ref=e34]: Recents
        - button "Search tasks (Ctrl+K)" [ref=e35] [cursor=pointer]:
          - img [ref=e37]
      - generic [ref=e40]:
        - paragraph [ref=e41]: Unable to load tasks
        - button "Reload tasks" [ref=e42] [cursor=pointer]
    - button "Settings" [ref=e44] [cursor=pointer]:
      - img [ref=e46]
      - generic [ref=e49]: Settings
  - separator "Resize sidebar" [ref=e50]:
    - status [ref=e51]
  - generic [ref=e52]:
    - banner:
      - generic [ref=e53]:
        - button "Manage gateway connection" [ref=e54] [cursor=pointer]: Disconnected
        - button "Language" [ref=e56] [cursor=pointer]:
          - img [ref=e58]
          - generic [ref=e61]: English
        - button "Theme" [ref=e63] [cursor=pointer]:
          - img [ref=e65]
    - main [ref=e68]:
      - generic [ref=e69]:
        - generic [ref=e70]:
          - navigation "View usage" [ref=e71]:
            - link "Usage" [ref=e72] [cursor=pointer]:
              - /url: /control/usage
              - img [ref=e74]
              - generic [ref=e75]: Usage
            - link "Overview" [ref=e76] [cursor=pointer]:
              - /url: /control/overview
              - img [ref=e78]
              - generic [ref=e81]: Overview
          - button "Support & diagnostics" [ref=e84] [cursor=pointer]:
            - img [ref=e86]
            - generic [ref=e89]: Support & diagnostics
            - img [ref=e91]
        - generic [ref=e94]:
          - region "Health summary" [ref=e95]:
            - img [ref=e98]
            - strong [ref=e101]: Checking
            - button "Refresh" [ref=e103] [cursor=pointer]:
              - img [ref=e105]
              - generic [ref=e108]: Refresh
          - region "Status" [ref=e109]:
            - button "Total tokens — —" [ref=e110] [cursor=pointer]:
              - generic [ref=e111]: Total tokens
              - generic [ref=e112]: —
              - generic [ref=e113]: —
            - button "Total sessions — view all →" [ref=e114] [cursor=pointer]:
              - generic [ref=e115]: Total sessions
              - generic [ref=e116]: —
              - generic [ref=e117]: view all →
            - generic [ref=e118]:
              - generic [ref=e119]: Uptime
              - generic [ref=e120]: —
              - generic [ref=e121]: —
          - region "Environment" [ref=e122]:
            - generic [ref=e123]: v— · —
          - region "Health findings" [ref=e124]:
            - article [ref=e125]: Loading health report
  - status [ref=e126]
```

# Test source

```ts
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
  54  |     await expect(hub.getByRole('link')).toHaveText(['Status', 'Usage'])
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
> 140 |     await page.locator('.ov-count').first().click()
      |                                             ^ Error: locator.click: Test timeout of 30000ms exceeded.
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