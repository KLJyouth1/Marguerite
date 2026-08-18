# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: chat.spec.ts >> Chat Page >> Overview and Skills & Channels own disjoint route families
- Location: e2e\chat.spec.ts:69:3

# Error details

```
Error: expect(locator).toHaveClass(expected) failed

Locator: locator('.sidebar-core').getByRole('link', { name: 'Overview' })
Expected pattern: /is-active/
Timeout: 5000ms
Error: element(s) not found

Call log:
  - Expect "toHaveClass" with timeout 5000ms
  - waiting for locator('.sidebar-core').getByRole('link', { name: 'Overview' })

```

```yaml
- navigation "Primary":
  - text: Marguerite
  - button "Collapse sidebar" [expanded]:
    - img
  - navigation "Control navigation":
    - button "New task":
      - img
      - text: New task
    - link "Skills & Channels":
      - /url: /control/skills
      - img
      - text: Skills & Channels
    - link "Cron":
      - /url: /control/cron
      - img
      - text: Cron
    - link "View usage":
      - /url: /control/usage
      - img
      - text: View usage
  - button "Settings":
    - img
    - text: Settings
- separator "Resize sidebar":
  - status
- banner:
  - button "Manage gateway connection": Disconnected
  - button "Language":
    - img
    - text: English
  - button "Theme":
    - img
- main:
  - navigation "View usage":
    - link "Usage":
      - /url: /control/usage
    - link "Overview":
      - /url: /control/overview
  - button "Support & diagnostics":
    - img
    - text: Support & diagnostics
    - img
  - region "Health summary":
    - strong: Checking
    - button "Refresh":
      - img
      - text: Refresh
  - region "Status":
    - button "Total tokens — —"
    - button "Total sessions — view all →"
    - text: Uptime — —
  - region "Environment": v— · —
  - region "Health findings":
    - article: Loading health report
- status
```

# Test source

```ts
  1   | import { test, expect } from '@playwright/test'
  2   | 
  3   | const CONTROL_URL = '/control/'
  4   | const LIVE = process.env.OPENSQUILLA_E2E_LIVE === '1'
  5   | 
  6   | test.describe('Chat Page', () => {
  7   |   test.beforeEach(async ({ page }) => {
  8   |     await page.goto(CONTROL_URL)
  9   |     // Wait for WebSocket connection
  10  |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  11  |   })
  12  | 
  13  |   test('page loads with correct title', async ({ page }) => {
  14  |     await expect(page).toHaveTitle(/OpenSquilla/)
  15  |   })
  16  | 
  17  |   test('sidebar core shows the flat primary navigation', async ({ page }) => {
  18  |     const core = page.locator('.sidebar-core')
  19  | 
  20  |     // Chat stays the dedicated New-chat action. Long-lived Agent management and
  21  |     // the old Build disclosure are intentionally absent from the primary rail.
  22  |     await expect(core.getByText('Chat', { exact: true })).toHaveCount(0)
  23  |     // Sessions is routed but off the nav; New task leads the index as an action
  24  |     // row (its own class) rather than a destination.
  25  |     await expect(core.locator('> .sidebar-new-session')).toHaveText(/New task/)
  26  |     await expect(core.locator('> .sidebar-fn-item .sidebar-fn-label')).toHaveText(
  27  |       ['Overview', 'Skills & Channels', 'Cron'],
  28  |     )
  29  |     await expect(core.getByText('Sessions', { exact: true })).toHaveCount(0)
  30  |     await expect(core.getByText('Agents', { exact: true })).toHaveCount(0)
  31  |     await expect(core.locator('.sidebar-nav-group-toggle')).toHaveCount(0)
  32  |   })
  33  | 
  34  |   test('command palette opens on recent tasks, not on a list of destinations', async ({ page }) => {
  35  |     await page.locator('.sidebar-cmd-btn').click()
  36  |     const palette = page.getByRole('dialog', { name: 'Search and go to' })
  37  |     await expect(palette).toBeVisible()
  38  | 
  39  |     // Untyped state answers "which task?" — the button promises task search, so
  40  |     // destinations must not be the resting content.
  41  |     await expect(palette.locator('.cmdp-group-label')).toHaveText(['Recent tasks'])
  42  |     for (const name of ['Overview', 'Skills & Channels', 'Cron']) {
  43  |       await expect(palette.getByRole('option', { name, exact: true })).toHaveCount(0)
  44  |     }
  45  |   })
  46  | 
  47  |   test('command palette keeps the Skills & Channels hub together in Work', async ({ page }) => {
  48  |     await page.locator('.sidebar-cmd-btn').click()
  49  |     const palette = page.getByRole('dialog', { name: 'Search and go to' })
  50  |     await expect(palette).toBeVisible()
  51  | 
  52  |     // Destinations surface by name rather than by default, so the grouping
  53  |     // contract is asserted against a query that matches the whole hub.
  54  |     await palette.getByRole('combobox').fill('channels')
  55  |     for (const name of ['Skills & Channels', 'Channels']) {
  56  |       await expect(palette.getByRole('option', { name, exact: true })).toBeVisible()
  57  |     }
  58  |     const labels = await palette.locator('.cmdp-option__label').allTextContents()
  59  |     expect(labels.indexOf('Channels')).toBe(labels.indexOf('Skills & Channels') + 1)
  60  |     await expect(palette.getByRole('option', { name: 'Agents', exact: true })).toHaveCount(0)
  61  |     await expect(palette.locator('.cmdp-group-label', { hasText: /^Build$/ })).toHaveCount(0)
  62  | 
  63  |     // Usage and Logs stay reachable from their own band despite being off the rail.
  64  |     await palette.getByRole('combobox').fill('usage')
  65  |     await expect(palette.locator('.cmdp-group-label', { hasText: /^Overview$/ })).toBeVisible()
  66  |     await expect(palette.getByRole('option', { name: 'Usage', exact: true })).toBeVisible()
  67  |   })
  68  | 
  69  |   test('Overview and Skills & Channels own disjoint route families', async ({ page }) => {
  70  |     const overview = page.locator('.sidebar-core').getByRole('link', { name: 'Overview' })
  71  |     const skillsChannels = page.locator('.sidebar-core').getByRole('link', { name: 'Skills & Channels' })
  72  |     for (const path of ['overview', 'usage', 'logs']) {
  73  |       await page.goto(CONTROL_URL + path)
> 74  |       await expect(overview).toHaveClass(/is-active/)
      |                              ^ Error: expect(locator).toHaveClass(expected) failed
  75  |       await expect(overview).toHaveAttribute('aria-current', 'page')
  76  |       await expect(skillsChannels).not.toHaveClass(/is-active/)
  77  |     }
  78  |     for (const path of ['skills', 'channels']) {
  79  |       await page.goto(CONTROL_URL + path)
  80  |       await expect(skillsChannels).toHaveClass(/is-active/)
  81  |       await expect(skillsChannels).toHaveAttribute('aria-current', 'page')
  82  |       await expect(overview).not.toHaveClass(/is-active/)
  83  |     }
  84  |   })
  85  | 
  86  |   test('Skills and Channels use canonical route links through history and reload', async ({ page }) => {
  87  |     await page.goto(CONTROL_URL + 'skills')
  88  |     let hub = page.getByRole('navigation', { name: 'Skills & Channels' })
  89  |     const skills = hub.getByRole('link', { name: 'Skills', exact: true })
  90  |     const channels = hub.getByRole('link', { name: 'Channels', exact: true })
  91  | 
  92  |     await expect(skills).toHaveAttribute('aria-current', 'page')
  93  |     await expect(channels).not.toHaveAttribute('aria-current', 'page')
  94  |     await channels.click()
  95  |     await expect(page).toHaveURL(/\/channels$/)
  96  |     await expect(channels).toHaveAttribute('aria-current', 'page')
  97  | 
  98  |     await page.goBack()
  99  |     await expect(page).toHaveURL(/\/skills$/)
  100 |     await expect(skills).toHaveAttribute('aria-current', 'page')
  101 | 
  102 |     await page.reload()
  103 |     hub = page.getByRole('navigation', { name: 'Skills & Channels' })
  104 |     await expect(hub.getByRole('link', { name: 'Skills', exact: true }))
  105 |       .toHaveAttribute('aria-current', 'page')
  106 |   })
  107 | 
  108 |   test('Overview exposes only Status and Usage while keeping Logs directly reachable', async ({ page }) => {
  109 |     await page.goto(CONTROL_URL + 'overview')
  110 |     const hub = page.getByRole('navigation', { name: 'Overview' })
  111 |     const status = hub.getByRole('link', { name: 'Status', exact: true })
  112 |     const usage = hub.getByRole('link', { name: 'Usage', exact: true })
  113 | 
  114 |     await expect(hub.getByRole('link')).toHaveCount(2)
  115 |     await expect(status).toHaveAttribute('aria-current', 'page')
  116 |     await expect(hub.getByRole('link', { name: 'Logs', exact: true })).toHaveCount(0)
  117 |     await usage.click()
  118 |     await expect(page).toHaveURL(/\/usage$/)
  119 |     await expect(usage).toHaveAttribute('aria-current', 'page')
  120 | 
  121 |     await page.goBack()
  122 |     await expect(page).toHaveURL(/\/overview$/)
  123 |     await expect(status).toHaveAttribute('aria-current', 'page')
  124 | 
  125 |     await page.goto(CONTROL_URL + 'logs')
  126 |     await expect(page).toHaveURL(/\/logs$/)
  127 |     await expect(page.getByRole('heading', { name: 'Logs', level: 1 })).toBeVisible()
  128 |   })
  129 | 
  130 |   test('can navigate between views', async ({ page }) => {
  131 |     const core = page.locator('.sidebar-core')
  132 | 
  133 |     await core.getByText('Overview', { exact: true }).click()
  134 |     await expect(page).toHaveURL(/\/overview/)
  135 | 
  136 |     await core.getByText('Skills & Channels', { exact: true }).click()
  137 |     await expect(page).toHaveURL(/\/skills/)
  138 | 
  139 |     await core.getByText('Cron', { exact: true }).click()
  140 |     await expect(page).toHaveURL(/\/cron/)
  141 | 
  142 |     // New task is instant (no modal): the primary row drops straight to a
  143 |     // draft. `exact` matches the New-task button precisely.
  144 |     await page.getByRole('button', { name: 'New task', exact: true }).click()
  145 |     await expect(page.getByRole('dialog', { name: 'New chat' })).toHaveCount(0)
  146 |     await expect(page).toHaveURL(/\/chat\/new\?agent=[a-z0-9_-]+$/i)
  147 |   })
  148 | 
  149 |   test('sidebar conversation list is free of raw identifiers', async ({ page }) => {
  150 |     // The family filter chips were retired with the Recents-only sidebar.
  151 |     await expect(page.locator('.sidebar-filter-chip')).toHaveCount(0)
  152 | 
  153 |     // Let the session list settle before inspecting sidebar text.
  154 |     await page.waitForSelector('.conn-pill.connected', { timeout: 10000 }).catch(() => {})
  155 |     await page.waitForTimeout(800)
  156 |     await expect(page.locator('.sidebar-history-list, .sidebar-onboarding, .sidebar-history-empty').first()).toBeVisible()
  157 | 
  158 |     const sidebarText = await page.locator('.sidebar').innerText()
  159 |     expect(sidebarText).not.toMatch(/agent:[a-z0-9_-]+:[a-z0-9_-]+:/i)
  160 |     expect(sidebarText).not.toMatch(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i)
  161 | 
  162 |     // Contract-gap chips stay hidden unless the debug feature flag is on.
  163 |     await expect(page.locator('.sidebar-history-gap')).toHaveCount(0)
  164 |   })
  165 | 
  166 |   test('theme menu picks a mode directly', async ({ page }) => {
  167 |     const html = page.locator('html')
  168 | 
  169 |     await page.evaluate(() => localStorage.setItem('marguerite-theme', 'light'))
  170 |     await page.reload()
  171 |     await page.waitForSelector('.topbar .conn-pill', { timeout: 10000 })
  172 |     await expect(html).toHaveAttribute('data-theme', 'light')
  173 | 
  174 |     const themeButton = page.getByRole('button', { name: 'Theme', exact: true })
```