# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: settings-modal.spec.ts >> Settings modal >> opens from the sidebar with the curated section rail and readiness banner
- Location: e2e\settings-modal.spec.ts:29:3

# Error details

```
Error: expect(locator).toBeVisible() failed

Locator: getByRole('dialog', { name: 'Settings' }).locator('.settings-banner')
Expected: visible
Timeout: 5000ms
Error: element(s) not found

Call log:
  - Expect "toBeVisible" with timeout 5000ms
  - waiting for getByRole('dialog', { name: 'Settings' }).locator('.settings-banner')

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
  - button "Manage gateway connection": Connected
  - button "Language":
    - img
    - text: English
  - button "Theme":
    - img
- main
- status
- dialog "Settings":
  - tablist "Settings sections":
    - 'tab "Connection: Connected"': Connection
    - 'tab "Model Service: Ready" [selected]': Model Service
    - 'tab "Model Routing: Ready"': Model Routing
    - 'tab "Capabilities: Optional"': Capabilities
    - 'tab "Behavior: Applies on save"': Behavior
    - 'tab "Privacy: Applies on save"': Privacy
    - tab "Sandbox"
    - tab "Memory & Profile"
    - tab "Appearance"
    - tab "Keyboard"
    - tab "Advanced"
  - banner:
    - heading "Settings" [level=2]
    - button "Close":
      - img
  - 'tabpanel "Model Service: Ready"':
    - group:
      - heading "Model Service" [level=3]
      - paragraph: Connect providers and manage their credentials and models.
      - button "Add provider"
      - group:
        - heading "Configured providers" [level=4]
        - paragraph: 1 of 1 ready. Testing sends one small model request.
        - list:
          - listitem:
            - button "Edit DeepSeek — Active — Credentials ready — Not verified": DeepSeek
            - button "Verify saved configuration — DeepSeek": Verify saved configuration
            - button "Edit DeepSeek": Edit
  - contentinfo:
    - text: More options live in
    - code: C:\Users\Administrator\AppData\Local\Marguerite\pyengine-home\default\config.toml
    - button "Copy config path":
      - img
    - text: Most changes apply live; some need a gateway restart
```

# Test source

```ts
  1   | import { test, expect } from '@playwright/test'
  2   | 
  3   | const CONTROL_URL = '/control/'
  4   | // Backend-config sections carry a readiness/status dot; Connection is the first
  5   | // entry (live socket state). Memory & Profile is an RPC action surface that
  6   | // deliberately stays outside config save state; the remaining entries below
  7   | // are ordinary client-only preferences.
  8   | // (Runtime exists too, but it is desktop-only so the web rail hides it.)
  9   | const SECTIONS = ['Connection', 'Model Service', 'Model Routing', 'Capabilities', 'Behavior', 'Privacy']
  10  | const CLIENT_SECTIONS = ['Sandbox', 'Memory & Profile', 'Appearance', 'Keyboard', 'Advanced']
  11  | 
  12  | const settingsRow = (page: import('@playwright/test').Page) =>
  13  |   page.locator('.sidebar-foot button')
  14  | 
  15  | const dialog = (page: import('@playwright/test').Page) =>
  16  |   page.getByRole('dialog', { name: 'Settings' })
  17  | 
  18  | const railTab = (page: import('@playwright/test').Page, name: string) =>
  19  |   dialog(page).getByRole('tab', { name: new RegExp(`^${name}:`) })
  20  | 
  21  | async function openFromSidebar(page: import('@playwright/test').Page) {
  22  |   await page.goto(CONTROL_URL)
  23  |   await page.waitForSelector('.conn-pill', { timeout: 10000 })
  24  |   await settingsRow(page).click()
  25  |   await expect(dialog(page)).toBeVisible()
  26  | }
  27  | 
  28  | test.describe('Settings modal', () => {
  29  |   test('opens from the sidebar with the curated section rail and readiness banner', async ({ page }) => {
  30  |     await openFromSidebar(page)
  31  | 
  32  |     // Dialog a11y: focus moves into the modal on open, before any interaction.
  33  |     await expect(page.getByRole('button', { name: 'Close' })).toBeFocused()
  34  | 
  35  |     // Rail has the backend-config sections (each with a readiness/status dot)
  36  |     // plus the client-only sections.
  37  |     const tabs = dialog(page).getByRole('tab')
  38  |     await expect(tabs).toHaveCount(SECTIONS.length + CLIENT_SECTIONS.length)
  39  |     for (const name of SECTIONS) {
  40  |       await expect(railTab(page, name)).toBeVisible()
  41  |     }
  42  |     for (const name of CLIENT_SECTIONS) {
  43  |       await expect(dialog(page).getByRole('tab', { name, exact: true })).toBeVisible()
  44  |     }
  45  | 
  46  |     // Readiness banner: quiet ready line or actionable count.
  47  |     const banner = dialog(page).locator('.settings-banner')
> 48  |     await expect(banner).toBeVisible()
      |                          ^ Error: expect(locator).toBeVisible() failed
  49  |     await expect(banner.locator('.settings-banner__row')).toContainText(/Ready to run|Action needed \(\d+\)/)
  50  | 
  51  |     // CLI handoff disclosure expands with command groups and the config summary.
  52  |     await banner.getByRole('button', { name: 'CLI handoff' }).click()
  53  |     await expect(banner.locator('.setup-cli__group', { hasText: 'CLI handoff' })).toBeVisible()
  54  |     await expect(banner.locator('.setup-cli__group', { hasText: 'CLI recipes' })).toBeVisible()
  55  |     await expect(banner.locator('.setup-summary')).toContainText('Provider')
  56  |   })
  57  | 
  58  |   test('no YAML editor, raw key search, or guided-setup wording anywhere', async ({ page }) => {
  59  |     await openFromSidebar(page)
  60  | 
  61  |     await expect(dialog(page).getByRole('button', { name: 'YAML', exact: true })).toHaveCount(0)
  62  |     await expect(dialog(page).getByRole('button', { name: 'Form', exact: true })).toHaveCount(0)
  63  |     await expect(dialog(page).locator('#cfg-search')).toHaveCount(0)
  64  |     await expect(dialog(page).locator('textarea#cfg-yaml-area')).toHaveCount(0)
  65  |     await expect(dialog(page).getByText('Guided setup')).toHaveCount(0)
  66  | 
  67  |     // Footer keeps the config.toml escape hatch with a copy affordance.
  68  |     const foot = dialog(page).locator('.settings-foot')
  69  |     await expect(foot).toContainText('More options live in')
  70  |     // Honest restart copy: most edits apply live, only some need a restart.
  71  |     await expect(foot).toContainText('Most changes apply live; some need a gateway restart')
  72  |     // The old blanket "always restart" leak is gone.
  73  |     await expect(foot).not.toContainText('Restart the gateway after manual edits')
  74  |     await expect(foot.locator('.settings-foot__path')).toContainText(/config.*\.toml/)
  75  |     await foot.getByRole('button', { name: 'Copy config path' }).click()
  76  |     await expect(page.locator('.toast', { hasText: /Copied/ }).first()).toBeVisible()
  77  |   })
  78  | 
  79  |   test('rail switches sections, marks the active tab, and syncs the URL with replace', async ({ page }) => {
  80  |     await openFromSidebar(page)
  81  |     await expect(page).toHaveURL(/\/settings$/)
  82  | 
  83  |     await railTab(page, 'Capabilities').click()
  84  |     await expect(railTab(page, 'Capabilities')).toHaveAttribute('aria-selected', 'true')
  85  |     // Switching a section reflects in the URL so it stays deep-linkable.
  86  |     await expect(page).toHaveURL(/\/settings\/capabilities$/)
  87  |     // The Capabilities panel renders its tools as stacked sections (Web search,
  88  |     // Memory, Image, Audio) rather than a redundant "Capabilities" card heading.
  89  |     await expect(dialog(page).getByRole('heading', { name: 'Web search' })).toBeVisible()
  90  | 
  91  |     await railTab(page, 'Model Routing').click()
  92  |     await expect(railTab(page, 'Model Routing')).toHaveAttribute('aria-selected', 'true')
  93  |     await expect(page).toHaveURL(/\/settings\/modelStrategy$/)
  94  |     await expect(dialog(page).getByRole('radiogroup', { name: 'Model routing', exact: true })).toBeVisible()
  95  | 
  96  |     // Section navigation uses replace, so a single Back exits Settings rather
  97  |     // than walking section history.
  98  |     await page.goBack()
  99  |     await expect(dialog(page)).toBeHidden()
  100 |     await expect(page).not.toHaveURL(/\/settings/)
  101 |   })
  102 | 
  103 |   test('opens Memory & Profile as a first-level action route without global save state', async ({ page }) => {
  104 |     await openFromSidebar(page)
  105 | 
  106 |     const memoryTab = dialog(page).getByRole('tab', { name: 'Memory & Profile', exact: true })
  107 |     await memoryTab.click()
  108 | 
  109 |     await expect(memoryTab).toHaveAttribute('aria-selected', 'true')
  110 |     await expect(page).toHaveURL(/\/settings\/memory$/)
  111 |     await expect(dialog(page).getByRole('heading', { name: 'Memory & Profile' })).toBeVisible()
  112 |     await expect(dialog(page).locator('[data-testid="settings-memory-panel"]')).toBeVisible()
  113 |     await expect(dialog(page).locator('.settings-dirtybar')).toBeHidden()
  114 |   })
  115 | 
  116 |   test('keeps the SettingsDialog mounted when routing from default settings to a section', async ({ page }) => {
  117 |     await openFromSidebar(page)
  118 |     await expect(page).toHaveURL(/\/settings$/)
  119 |     await expect(dialog(page).locator('.settings-banner')).toBeVisible()
  120 |     await expect(dialog(page).locator('.settings-loading')).toHaveCount(0)
  121 | 
  122 |     await page.evaluate(() => {
  123 |       const modal = document.querySelector('.settings-modal')
  124 |       if (!modal) throw new Error('settings modal missing')
  125 |       const state = {
  126 |         modal,
  127 |         removed: false,
  128 |         loadingSeen: false,
  129 |         observer: null as MutationObserver | null,
  130 |       }
  131 |       state.observer = new MutationObserver(() => {
  132 |         state.removed = state.removed || !document.body.contains(state.modal)
  133 |         state.loadingSeen = state.loadingSeen || !!document.querySelector('.settings-loading')
  134 |       })
  135 |       state.observer.observe(document.body, { childList: true, subtree: true })
  136 |       ;(window as typeof window & { __settingsModalProbe?: typeof state }).__settingsModalProbe = state
  137 |     })
  138 | 
  139 |     await railTab(page, 'Capabilities').click()
  140 |     await expect(page).toHaveURL(/\/settings\/capabilities$/)
  141 |     await expect(railTab(page, 'Capabilities')).toHaveAttribute('aria-selected', 'true')
  142 |     await expect(dialog(page).getByRole('heading', { name: 'Web search' })).toBeVisible()
  143 |     // Wait past the route fade and Settings pop enter windows so delayed
  144 |     // remounts, re-entry animations, or loading fallbacks are caught.
  145 |     await page.waitForTimeout(350)
  146 | 
  147 |     const probe = await page.evaluate(() => {
  148 |       const state = (window as typeof window & {
```